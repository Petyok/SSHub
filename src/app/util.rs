use super::*;

/// Look up the stored credential for a host entry and decide whether it's
/// a host password (sent at `password:` prompts) or an identity passphrase
/// (sent at `Enter passphrase for …`). Returns the pending secret and a
/// human-readable diagnostic line for the SSH log.
pub fn resolve_pending_secret(
    entry: &HostEntry,
    password_store: &dyn crate::credentials::PasswordStore,
) -> (Option<crate::session::PendingSecret>, String) {
    let Some(managed) = entry.managed() else {
        return (
            None,
            "auth: legacy ssh_config host — no stored credential".into(),
        );
    };

    if managed.has_password {
        let key = crate::credentials::host_key(managed.id);
        return match password_store.get(&key) {
            Ok(Some(pw)) => (
                Some(crate::session::PendingSecret::Password(pw)),
                format!("auth: using stored password ({key})"),
            ),
            Ok(None) => (
                None,
                format!(
                    "auth: has_password=true but keyring entry {key} is empty — ssh will prompt"
                ),
            ),
            Err(e) => (
                None,
                format!("auth: keyring lookup failed for {key}: {e:#} — ssh will prompt"),
            ),
        };
    }

    if let Some(identity) = managed.identity.as_ref() {
        if identity.has_password {
            let key = crate::credentials::identity_key(identity.id);
            // A secret on an identity WITH a key unlocks that key (passphrase);
            // on a keyless identity it's a shared login password, letting many
            // hosts reuse one user+password credential.
            let has_key = identity.private_key.is_some();
            return match password_store.get(&key) {
                Ok(Some(pw)) => (
                    Some(if has_key {
                        crate::session::PendingSecret::Passphrase(pw)
                    } else {
                        crate::session::PendingSecret::Password(pw)
                    }),
                    format!(
                        "auth: using stored {} ({key})",
                        if has_key { "passphrase" } else { "password" }
                    ),
                ),
                Ok(None) => (
                    None,
                    format!(
                        "auth: identity has_password=true but keyring entry {key} is empty — ssh will prompt"
                    ),
                ),
                Err(e) => (
                    None,
                    format!("auth: keyring lookup failed for {key}: {e:#} — ssh will prompt"),
                ),
            };
        }
    }

    (
        None,
        "auth: no stored credential — using agent / unlocked key / interactive prompt".into(),
    )
}

/// Capture host metadata used by the embedded session header + connect
/// animation.
pub(crate) fn session_meta_for_entry(entry: &HostEntry) -> crate::session::SessionMeta {
    match entry {
        HostEntry::Managed(m) => crate::session::SessionMeta {
            user: m
                .username
                .clone()
                .or_else(|| m.identity.as_ref().and_then(|i| i.username.clone())),
            address: Some(m.address.clone()),
            port: Some(m.port),
            identity: m
                .identity
                .as_ref()
                .and_then(|i| i.private_key.as_ref())
                .map(|p| p.to_string_lossy().into_owned()),
            proxy_jump: m.proxy_jump.clone(),
            host_id: Some(m.id),
        },
        HostEntry::Legacy { host, .. } => crate::session::SessionMeta {
            user: host.user.clone(),
            address: host.hostname.clone(),
            port: host.port,
            identity: host.identity_file.clone(),
            proxy_jump: host.proxy_jump.clone(),
            host_id: None,
        },
    }
}

/// Build the bare `ssh` argv for a host entry (no env / askpass prefix).
///
/// - Launcher-managed hosts: full options via `build_ssh_argv` so we don't
///   require an `~/.ssh/config` alias.
/// - SSH-config-sourced hosts: alias-only argv via `build_ssh_alias_argv` so
///   ssh inherits all options from the user's config.
/// - Legacy entries (ssh_config only, not in launcher DB): alias-only argv.
pub fn ssh_argv_for_entry(entry: &HostEntry) -> Vec<String> {
    match entry {
        HostEntry::Managed(m) => {
            let ssh_host = managed_to_ssh_host(m);
            if m.source == HostSource::SshConfig {
                crate::ssh::build_ssh_alias_argv(&ssh_host)
            } else {
                crate::ssh::build_ssh_argv(&ssh_host)
            }
        }
        HostEntry::Legacy { host, .. } => crate::ssh::build_ssh_alias_argv(host),
    }
}

pub(crate) fn managed_to_ssh_host(m: &ManagedHost) -> SshHost {
    let mut host = SshHost::new(&m.name);
    host.hostname = Some(m.address.clone());
    host.port = Some(m.port);
    host.user = m
        .username
        .clone()
        .or_else(|| m.identity.as_ref().and_then(|i| i.username.clone()));
    host.identity_file = m
        .identity
        .as_ref()
        .and_then(|i| i.private_key.as_ref())
        .map(|p| p.to_string_lossy().into_owned());
    host.certificate_file = m
        .identity
        .as_ref()
        .and_then(|i| i.certificate.as_ref())
        .map(|p| p.to_string_lossy().into_owned());
    host.proxy_jump = m.proxy_jump.clone();
    host.forward_agent = Some(m.forward_agent);
    host.remote_command = m.remote_command.clone();
    host
}

pub(crate) fn optional_path(raw: &str) -> Option<std::path::PathBuf> {
    optional_field(raw).map(std::path::PathBuf::from)
}

/// Write an OSC 52 set-clipboard sequence to stdout. Modern terminals
/// (kitty / iTerm2 / wezterm / Alacritty / foot) interpret this as
/// "put this base64-encoded payload on the system clipboard". The
/// sequence is invisible to the alternate-screen UI — the host terminal
/// consumes it before it ever lands on a buffer cell.
pub(crate) fn write_osc52(text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let encoded = base64_encode(text.as_bytes());
    let payload = format!("\x1b]52;c;{encoded}\x07");
    let mut out = std::io::stdout().lock();
    out.write_all(payload.as_bytes())?;
    out.flush()
}

/// Tiny base64 (standard alphabet, padded). Inlined so we don't pull in
/// another crate for a single ~20 line helper used in one place.
pub(crate) fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut chunks = input.chunks_exact(3);
    for chunk in chunks.by_ref() {
        let b = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
        out.push(ALPHABET[((b >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((b >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((b >> 6) & 0x3f) as usize] as char);
        out.push(ALPHABET[(b & 0x3f) as usize] as char);
    }
    let rem = chunks.remainder();
    match rem.len() {
        1 => {
            let b = (rem[0] as u32) << 16;
            out.push(ALPHABET[((b >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((b >> 12) & 0x3f) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let b = ((rem[0] as u32) << 16) | ((rem[1] as u32) << 8);
            out.push(ALPHABET[((b >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((b >> 12) & 0x3f) as usize] as char);
            out.push(ALPHABET[((b >> 6) & 0x3f) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

/// Expand a leading `~` (or `~/`) in a path to the user's home directory.
pub(crate) fn shellexpand_home(path: &str) -> std::path::PathBuf {
    if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(home);
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(home).join(rest);
        }
    }
    std::path::PathBuf::from(path)
}

pub(crate) fn os_icon_from_index(index: usize) -> Option<String> {
    match OS_ICON_OPTIONS.get(index) {
        Some(&"(none)") | None => None,
        Some(s) => Some((*s).to_string()),
    }
}

pub(crate) fn os_icon_index_from_option(icon: &Option<String>) -> usize {
    icon.as_deref()
        .and_then(|name| OS_ICON_OPTIONS.iter().position(|opt| *opt == name))
        .unwrap_or(0)
}

pub(crate) fn parse_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

pub(crate) fn sort_host_indices(hosts: &[HostEntry], indices: &mut [usize], mode: SortMode) {
    indices.sort_by(|&a, &b| compare_hosts(&hosts[a], &hosts[b], mode));
}

pub(crate) fn compare_hosts(a: &HostEntry, b: &HostEntry, mode: SortMode) -> std::cmp::Ordering {
    match mode {
        SortMode::Label => label_cmp(a, b),
        SortMode::LastConnected => match (b.last_connected(), a.last_connected()) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => label_cmp(a, b),
        },
        SortMode::FavoriteFirst => match (a.favorite(), b.favorite()) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => label_cmp(a, b),
        },
        SortMode::GroupThenLabel => group_sort_key(a)
            .cmp(&group_sort_key(b))
            .then_with(|| label_cmp(a, b)),
        SortMode::Manual => a
            .sort_order()
            .cmp(&b.sort_order())
            .then_with(|| a.name().cmp(b.name())),
    }
}

pub(crate) fn label_cmp(a: &HostEntry, b: &HostEntry) -> std::cmp::Ordering {
    a.display_name()
        .to_lowercase()
        .cmp(&b.display_name().to_lowercase())
}

pub(crate) fn group_sort_key(entry: &HostEntry) -> String {
    match entry.managed().and_then(|m| m.group.as_ref()) {
        Some(g) => format!("{:08}_{}", g.sort_order, g.name.to_lowercase()),
        None => format!("z_{UNGROUPED_LABEL}"),
    }
}

pub(crate) fn build_group_sections(
    hosts: &[HostEntry],
    groups: &[HostGroup],
    filtered: &[usize],
) -> Vec<HostGroupSection> {
    let mut sections = Vec::new();

    for group in groups {
        let host_indices: Vec<usize> = filtered
            .iter()
            .copied()
            .filter(|&idx| hosts[idx].group_id() == Some(group.id))
            .collect();
        sections.push(HostGroupSection {
            group: Some(group.clone()),
            label: group.name.clone(),
            host_indices,
            collapsed: false,
        });
    }

    let ungrouped: Vec<usize> = filtered
        .iter()
        .copied()
        .filter(|&idx| hosts[idx].group_id().is_none())
        .collect();
    if !ungrouped.is_empty() {
        sections.push(HostGroupSection {
            group: None,
            label: UNGROUPED_LABEL.to_string(),
            host_indices: ungrouped,
            collapsed: false,
        });
    }

    sections
}

/// Parse a keybinding spec like `"Ctrl+S"`, `"F2"`, `"Alt+Enter"` into a
/// (code, modifiers) pair. Returns `None` for unrecognised specs.
pub(crate) fn parse_keyspec(spec: &str) -> Option<(KeyCode, KeyModifiers)> {
    let parts: Vec<&str> = spec.split('+').map(|p| p.trim()).collect();
    let (key_part, mod_parts) = parts.split_last()?;
    let mut mods = KeyModifiers::empty();
    for m in mod_parts {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
            "alt" | "option" => mods |= KeyModifiers::ALT,
            "shift" => mods |= KeyModifiers::SHIFT,
            _ => return None,
        }
    }
    let key = key_part.trim();
    if key.is_empty() {
        return None;
    }
    let code = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        "esc" | "escape" => KeyCode::Esc,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "backtab" => KeyCode::BackTab,
        lower => {
            // Function key "F1".."F12"?
            if let Some(n) = lower
                .strip_prefix('f')
                .filter(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
                .and_then(|r| r.parse::<u8>().ok())
            {
                KeyCode::F(n)
            } else if lower.chars().count() == 1 {
                KeyCode::Char(lower.chars().next().unwrap())
            } else {
                return None;
            }
        }
    };
    Some((code, mods))
}

/// Serialize an incoming key event into a spec string (inverse of
/// [`parse_keyspec`]) for capturing a binding in the UI. Returns `None` for
/// keys that can't be a binding (bare modifiers, unsupported codes).
pub(crate) fn keyevent_to_spec(key: &KeyEvent) -> Option<String> {
    let base = match key.code {
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::BackTab => "BackTab".to_string(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Char(c) => c.to_ascii_uppercase().to_string(),
        _ => return None,
    };
    let mut out = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        out.push_str("Ctrl+");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        out.push_str("Alt+");
    }
    // Shift is only meaningful for keys that aren't already shifted into a
    // distinct char (e.g. Shift+H stays "Shift+H"; '?' has no Shift prefix).
    if key.modifiers.contains(KeyModifiers::SHIFT)
        && !matches!(key.code, KeyCode::Char(c) if !c.is_ascii_alphabetic())
    {
        out.push_str("Shift+");
    }
    out.push_str(&base);
    Some(out)
}

/// Match a parsed spec against an incoming event, comparing char keys
/// case-insensitively (so `Ctrl+S` matches whatever case crossterm reports).
pub(crate) fn keyspec_matches(code: KeyCode, mods: KeyModifiers, key: &KeyEvent) -> bool {
    let code_eq = match (code, key.code) {
        (KeyCode::Char(a), KeyCode::Char(b)) => a.eq_ignore_ascii_case(&b),
        (a, b) => a == b,
    };
    code_eq && key.modifiers == mods
}

pub(crate) fn optional_field(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(crate) fn tab_from_x(x: u16) -> Option<usize> {
    // Tab bar layout (from tab_bar.rs): 1-char left margin, then per tab:
    // 4 chars for number+brackets + label_len + 3 chars gap
    // Labels: "hosts"(5), "tunnels"(7), "identities"(10), "audit"(5)
    let labels = [5u16, 7, 10, 5];
    let mut cx = 1u16; // 1-char margin
    for (i, label_len) in labels.iter().enumerate() {
        let tab_w = 4 + label_len + 3;
        if x >= cx && x < cx + tab_w {
            return Some(i);
        }
        cx += tab_w;
    }
    None
}
