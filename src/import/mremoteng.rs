//! Importer for mRemoteNG connection files (`confCons.xml`).
//!
//! mRemoteNG stores its connection tree as a small XML document:
//!
//! ```xml
//! <?xml version="1.0" encoding="utf-8"?>
//! <mrng:Connections xmlns:mrng="http://mremoteng.org" Name="Connections" ConfVersion="2.6">
//!   <Node Name="Prod" Type="Container" ...>
//!     <Node Name="web1" Type="Connection" Hostname="10.0.0.1" Username="admin"
//!           Protocol="SSH2" Port="22" Password="AESblob" />
//!   </Node>
//!   <Node Name="db" Type="Connection" Hostname="db.example.com" Protocol="SSH2" Port="2222" />
//! </mrng:Connections>
//! ```
//!
//! The project ships no XML crate on purpose — parsing is a hand-rolled,
//! byte-scanning reader (same house style as `termius_csv.rs`). We only need
//! enough of XML to walk `<Node>` elements: the prolog, comments, CDATA, a
//! doctype, self-closing tags, nested open/close tags, quoted attributes, and
//! the handful of predefined + numeric entities.
//!
//! `Type="Container"` nodes form a folder breadcrumb which becomes the host's
//! tags; `Type="Connection"` nodes with an `SSH*` protocol become hosts. Every
//! other protocol (RDP/VNC/telnet/…) is counted as skipped.

use std::path::Path;

use anyhow::{Context, Result};

use crate::import::HostImportReport;
use crate::store::{HostSource, LauncherStore, NewHost};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single SSH connection extracted from `confCons.xml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MremoteHost {
    pub name: String,
    pub hostname: String,
    pub port: u16,
    pub username: String,
    /// Container breadcrumb from the tree root down to this node's parent.
    pub folders: Vec<String>,
}

/// Result of parsing `confCons.xml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MremoteParse {
    pub hosts: Vec<MremoteHost>,
    /// `Type="Connection"` nodes whose protocol was not SSH.
    pub non_ssh_skipped: usize,
}

// ---------------------------------------------------------------------------
// XML parsing
// ---------------------------------------------------------------------------

/// Parse an mRemoteNG `confCons.xml` document into SSH hosts + a non-SSH count.
///
/// Robust to (and ignores) the XML prolog, comments, CDATA and doctype. It
/// tracks container nesting with an explicit element stack so that closing
/// tags — including the root element and self-closing nodes — pop the folder
/// breadcrumb correctly.
pub fn parse_conf_cons(xml: &str) -> MremoteParse {
    let bytes = xml.as_bytes();
    let n = bytes.len();
    let mut i = 0;

    // Folder breadcrumb (container names) and a parallel element stack recording
    // whether each still-open element pushed a breadcrumb entry, so its close
    // pops exactly what it pushed.
    let mut breadcrumb: Vec<String> = Vec::new();
    let mut pushed_folder: Vec<bool> = Vec::new();

    let mut hosts: Vec<MremoteHost> = Vec::new();
    let mut non_ssh_skipped = 0usize;

    while i < n {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }

        // Prolog / processing instruction: <? ... ?>
        if starts_with(bytes, i, b"<?") {
            i = find_seq(bytes, i + 2, b"?>").map_or(n, |p| p + 2);
            continue;
        }
        // Comment: <!-- ... -->
        if starts_with(bytes, i, b"<!--") {
            i = find_seq(bytes, i + 4, b"-->").map_or(n, |p| p + 3);
            continue;
        }
        // CDATA: <![CDATA[ ... ]]>
        if starts_with(bytes, i, b"<![CDATA[") {
            i = find_seq(bytes, i + 9, b"]]>").map_or(n, |p| p + 3);
            continue;
        }
        // Doctype / other declaration: <! ... >
        if starts_with(bytes, i, b"<!") {
            i = find_tag_end(bytes, i).map_or(n, |e| e + 1);
            continue;
        }

        // Regular element tag. Scan to the terminating '>', ignoring any '>'
        // that sits inside a quoted attribute value.
        let Some(end) = find_tag_end(bytes, i) else {
            break;
        };
        let inner = &xml[i + 1..end];
        i = end + 1;

        let trimmed = inner.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Closing tag: </Name>. The local name is irrelevant — the element
        // stack tells us exactly what this close pops.
        if trimmed.starts_with('/') {
            if let Some(folded) = pushed_folder.pop() {
                if folded {
                    breadcrumb.pop();
                }
            }
            continue;
        }

        // Open or self-closing tag.
        let self_closing = trimmed.ends_with('/');
        let body = if self_closing {
            trimmed[..trimmed.len() - 1].trim()
        } else {
            trimmed
        };

        // Split the element name from its attributes.
        let (name_tok, attr_str) = match body.find(|c: char| c.is_whitespace()) {
            Some(idx) => (&body[..idx], &body[idx..]),
            None => (body, ""),
        };
        let is_node = local_name(name_tok).eq_ignore_ascii_case("Node");

        if !is_node {
            // Non-Node element (e.g. the root <mrng:Connections>). Still track
            // it on the stack so its close pops correctly, but it never pushes
            // a breadcrumb entry.
            if !self_closing {
                pushed_folder.push(false);
            }
            continue;
        }

        let attrs = parse_attrs(attr_str);
        let typ = attr(&attrs, "Type").unwrap_or("");

        if typ.eq_ignore_ascii_case("Container") {
            if !self_closing {
                let name = attr_owned(&attrs, "Name");
                breadcrumb.push(name);
                pushed_folder.push(true);
            }
            // A self-closing container has no children; it pushes nothing.
            continue;
        }

        if typ.eq_ignore_ascii_case("Connection") {
            let protocol = attr(&attrs, "Protocol").unwrap_or("");
            // `get(..3)` (not a byte slice) so a non-ASCII Protocol value whose
            // 3rd byte is mid-codepoint yields None instead of panicking.
            if protocol
                .get(..3)
                .is_some_and(|p| p.eq_ignore_ascii_case("SSH"))
            {
                let name = attr_owned(&attrs, "Name");
                let hostname = attr_owned(&attrs, "Hostname");
                // Empty name or hostname: skip silently, do not count.
                if !name.is_empty() && !hostname.is_empty() {
                    let port = attr(&attrs, "Port")
                        .and_then(|p| p.trim().parse::<u16>().ok())
                        .unwrap_or(22);
                    hosts.push(MremoteHost {
                        name,
                        hostname,
                        port,
                        username: attr_owned(&attrs, "Username"),
                        folders: breadcrumb.clone(),
                    });
                }
            } else {
                non_ssh_skipped += 1;
            }
            // Connections are normally self-closing; if one has children, keep
            // the stack balanced.
            if !self_closing {
                pushed_folder.push(false);
            }
            continue;
        }

        // A <Node> with an unrecognised (or missing) Type — keep the stack
        // balanced but do not treat it as a folder or a host.
        if !self_closing {
            pushed_folder.push(false);
        }
    }

    MremoteParse {
        hosts,
        non_ssh_skipped,
    }
}

/// Strip a namespace prefix from an element/attribute name (`mrng:Node` -> `Node`).
fn local_name(tok: &str) -> &str {
    match tok.rfind(':') {
        Some(idx) => &tok[idx + 1..],
        None => tok,
    }
}

/// True when `bytes[at..]` begins with `pat`.
fn starts_with(bytes: &[u8], at: usize, pat: &[u8]) -> bool {
    bytes.len() >= at + pat.len() && &bytes[at..at + pat.len()] == pat
}

/// Find the byte index where `pat` next occurs at or after `from`.
fn find_seq(bytes: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    if pat.is_empty() || from > bytes.len() {
        return None;
    }
    let mut i = from;
    while i + pat.len() <= bytes.len() {
        if &bytes[i..i + pat.len()] == pat {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Given the index of a tag's opening `<`, return the index of the `>` that
/// terminates it, ignoring `>` characters inside single- or double-quoted
/// attribute values.
fn find_tag_end(bytes: &[u8], open: usize) -> Option<usize> {
    let mut i = open + 1;
    let mut quote: u8 = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Parse an attribute list (`key="value" key='value' bare`) into lowercased
/// keys paired with entity-decoded values.
fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let bytes = s.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    let mut out: Vec<(String, String)> = Vec::new();

    while i < n {
        // Skip leading whitespace.
        while i < n && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        // Read the key up to '=' or whitespace.
        let ks = i;
        while i < n && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key = &s[ks..i];
        // Skip whitespace before a possible '='.
        while i < n && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < n && bytes[i] == b'=' {
            i += 1;
            while i < n && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < n && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let q = bytes[i];
                i += 1;
                let vs = i;
                while i < n && bytes[i] != q {
                    i += 1;
                }
                let raw = &s[vs..i];
                if i < n {
                    i += 1; // consume the closing quote
                }
                if !key.is_empty() {
                    out.push((key.to_ascii_lowercase(), decode_entities(raw)));
                }
            } else {
                // Unquoted value (not valid XML, but be lenient).
                let vs = i;
                while i < n && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if !key.is_empty() {
                    out.push((key.to_ascii_lowercase(), decode_entities(&s[vs..i])));
                }
            }
        } else if !key.is_empty() {
            // Valueless attribute.
            out.push((key.to_ascii_lowercase(), String::new()));
        }
    }

    out
}

/// Look up an attribute value by (case-insensitive) name.
fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
}

/// Look up an attribute value, returning an owned (possibly empty) string.
fn attr_owned(attrs: &[(String, String)], key: &str) -> String {
    attr(attrs, key).unwrap_or("").to_string()
}

/// Decode the predefined XML entities (`&amp; &lt; &gt; &quot; &apos;`) and
/// numeric character references (`&#NN;` / `&#xNN;`). Unknown entities are left
/// verbatim.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let mut ent = String::new();
        let mut terminated = false;
        while let Some(&nc) = chars.peek() {
            if nc == ';' {
                chars.next();
                terminated = true;
                break;
            }
            // Entity names are short and never contain these; bail out and emit
            // the '&' literally rather than swallowing surrounding text.
            if nc == '&' || nc == '<' || nc.is_whitespace() || ent.len() >= 12 {
                break;
            }
            ent.push(nc);
            chars.next();
        }

        if terminated {
            if let Some(ch) = resolve_entity(&ent) {
                out.push(ch);
                continue;
            }
            // Unknown but well-formed entity: keep it verbatim.
            out.push('&');
            out.push_str(&ent);
            out.push(';');
        } else {
            out.push('&');
            out.push_str(&ent);
        }
    }

    out
}

/// Resolve a single entity name (without the surrounding `&`/`;`) to a char.
fn resolve_entity(ent: &str) -> Option<char> {
    match ent {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let num = ent.strip_prefix('#')?;
            let cp = if let Some(hex) = num.strip_prefix(['x', 'X']) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                num.parse::<u32>().ok()?
            };
            char::from_u32(cp)
        }
    }
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Import an mRemoteNG `confCons.xml` file into the launcher store.
///
/// Reads and parses the file, then inserts one host per SSH connection. The
/// container breadcrumb of each connection becomes the host's tags. Hosts whose
/// name already exists are skipped (never overwritten). Non-SSH connections are
/// counted in [`HostImportReport::skipped_non_ssh`].
pub fn import_mremoteng(path: &Path, store: &LauncherStore) -> Result<HostImportReport> {
    let xml =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let parsed = parse_conf_cons(&xml);

    let mut report = HostImportReport {
        skipped_non_ssh: parsed.non_ssh_skipped,
        ..Default::default()
    };

    for host in &parsed.hosts {
        if store.get_host_by_name(&host.name)?.is_some() {
            report.skipped_existing += 1;
            continue;
        }
        let username = (!host.username.is_empty()).then(|| host.username.clone());
        let new_host = NewHost {
            name: host.name.clone(),
            address: host.hostname.clone(),
            port: host.port,
            username,
            tags: host.folders.clone(),
            notes: Some("Imported from mRemoteNG".into()),
            source: HostSource::Launcher,
            ..Default::default()
        };
        store.create_host(&new_host)?;
        report.imported += 1;
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_conf_cons_reads_container_and_top_level() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<mrng:Connections xmlns:mrng="http://mremoteng.org" Name="Connections" ConfVersion="2.6">
  <Node Name="Prod" Type="Container" Descr="" Icon="mRemoteNG" Id="g1">
    <Node Name="web1" Type="Connection" Hostname="10.0.0.1" Username="admin" Protocol="SSH2" Password="AESblob" />
    <Node Name="win" Type="Connection" Hostname="10.0.0.9" Protocol="RDP" Port="3389" />
  </Node>
  <Node Name="a &amp; b" Type="Connection" Hostname="db.example.com" Username="root" Protocol="SSH2" Port="2222" />
</mrng:Connections>"#;

        let parsed = parse_conf_cons(xml);
        assert_eq!(parsed.hosts.len(), 2);
        assert_eq!(parsed.non_ssh_skipped, 1);

        // Nested SSH host: folder breadcrumb + default port (no Port attr).
        let web1 = &parsed.hosts[0];
        assert_eq!(web1.name, "web1");
        assert_eq!(web1.hostname, "10.0.0.1");
        assert_eq!(web1.username, "admin");
        assert_eq!(web1.port, 22);
        assert_eq!(web1.folders, vec!["Prod".to_string()]);

        // Top-level SSH host: empty folders, explicit port, decoded entity.
        let top = &parsed.hosts[1];
        assert_eq!(top.name, "a & b");
        assert_eq!(top.hostname, "db.example.com");
        assert_eq!(top.username, "root");
        assert_eq!(top.port, 2222);
        assert!(top.folders.is_empty());
    }

    #[test]
    fn parse_conf_cons_handles_self_closing_and_nested_containers() {
        let xml = r#"<mrng:Connections Name="Connections">
  <!-- an empty folder should not disturb the breadcrumb -->
  <Node Name="Empty" Type="Container" Id="e" />
  <Node Name="L1" Type="Container">
    <Node Name="L2" Type="Container">
      <Node Name="deep" Type="Connection" Hostname="1.2.3.4" Protocol="SSH1" Port="22" />
    </Node>
  </Node>
</mrng:Connections>"#;

        let parsed = parse_conf_cons(xml);
        assert_eq!(parsed.non_ssh_skipped, 0);
        assert_eq!(parsed.hosts.len(), 1);

        let deep = &parsed.hosts[0];
        assert_eq!(deep.name, "deep");
        assert_eq!(deep.folders, vec!["L1".to_string(), "L2".to_string()]);
        assert_eq!(deep.port, 22);
    }

    #[test]
    fn parse_conf_cons_skips_empty_name_or_hostname() {
        let xml = r#"<mrng:Connections>
  <Node Name="" Type="Connection" Hostname="1.1.1.1" Protocol="SSH2" />
  <Node Name="noaddr" Type="Connection" Hostname="" Protocol="SSH2" />
  <Node Name="ok" Type="Connection" Hostname="2.2.2.2" Protocol="SSH2" />
</mrng:Connections>"#;

        let parsed = parse_conf_cons(xml);
        // The two malformed SSH nodes are dropped silently, not counted.
        assert_eq!(parsed.non_ssh_skipped, 0);
        assert_eq!(parsed.hosts.len(), 1);
        assert_eq!(parsed.hosts[0].name, "ok");
    }

    #[test]
    fn parse_conf_cons_ignores_gt_inside_attribute() {
        // A '>' inside a quoted value must not terminate the tag early.
        let xml = r#"<mrng:Connections>
  <Node Name="cmp" Type="Connection" Hostname="h" Protocol="SSH2" Descr="a > b &lt; c" Port="42" />
</mrng:Connections>"#;

        let parsed = parse_conf_cons(xml);
        assert_eq!(parsed.hosts.len(), 1);
        assert_eq!(parsed.hosts[0].hostname, "h");
        assert_eq!(parsed.hosts[0].port, 42);
    }

    #[test]
    fn parse_conf_cons_multibyte_protocol_does_not_panic() {
        // A Protocol value with a multi-byte char straddling byte offset 3 must
        // be treated as non-SSH, never panic on a fixed byte-slice.
        let xml = r#"<mrng:Connections><Node Name="x" Type="Connection" Hostname="h" Protocol="a日SH2"/></mrng:Connections>"#;
        let parsed = parse_conf_cons(xml);
        assert!(parsed.hosts.is_empty());
        assert_eq!(parsed.non_ssh_skipped, 1);
    }

    #[test]
    fn decode_entities_handles_named_and_numeric() {
        assert_eq!(decode_entities("a &amp; b"), "a & b");
        assert_eq!(decode_entities("&lt;tag&gt;"), "<tag>");
        assert_eq!(decode_entities("&quot;q&apos;"), "\"q'");
        assert_eq!(decode_entities("&#65;&#x42;"), "AB");
        // Unknown entity is preserved verbatim.
        assert_eq!(decode_entities("100% &unknown; ok"), "100% &unknown; ok");
        // Bare ampersand without a terminator.
        assert_eq!(decode_entities("a & b"), "a & b");
    }

    #[test]
    fn import_mremoteng_imports_and_dedups() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("confCons.xml");
        std::fs::write(
            &path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<mrng:Connections xmlns:mrng="http://mremoteng.org" Name="Connections">
  <Node Name="Prod" Type="Container">
    <Node Name="web1" Type="Connection" Hostname="10.0.0.1" Username="admin" Protocol="SSH2" />
    <Node Name="win" Type="Connection" Hostname="10.0.0.9" Protocol="RDP" Port="3389" />
  </Node>
  <Node Name="db" Type="Connection" Hostname="db.example.com" Username="root" Protocol="SSH2" Port="2222" />
</mrng:Connections>"#,
        )
        .unwrap();

        let store = LauncherStore::open_in_memory().unwrap();
        let report = import_mremoteng(&path, &store).unwrap();
        assert_eq!(report.imported, 2);
        assert_eq!(report.skipped_existing, 0);
        assert_eq!(report.skipped_non_ssh, 1);

        // Folder breadcrumb landed in the host's tags.
        let web1 = store.get_host_by_name("web1").unwrap().unwrap();
        assert_eq!(web1.address, "10.0.0.1");
        assert_eq!(web1.username.as_deref(), Some("admin"));
        assert_eq!(web1.tags, vec!["Prod".to_string()]);
        assert_eq!(web1.source, HostSource::Launcher);

        let db = store.get_host_by_name("db").unwrap().unwrap();
        assert_eq!(db.port, 2222);
        assert!(db.tags.is_empty());

        // Re-running the same import touches nothing new.
        let again = import_mremoteng(&path, &store).unwrap();
        assert_eq!(again.imported, 0);
        assert_eq!(again.skipped_existing, 2);
        assert_eq!(again.skipped_non_ssh, 1);
    }
}
