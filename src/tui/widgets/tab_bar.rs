//! Dashboard tab bar — numbered tabs with active highlight.

use std::sync::LazyLock;

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::Frame;

use crate::theme::catalog::{ColorRole, StyleRole};
use crate::theme::model::ResolvedTheme;

/// The version string shown at the far right, resolved once at runtime.
///
/// * `SSHUB_VERSION_LABEL` **unset** → `v{CARGO_PKG_VERSION}`, plus an
///   install-channel suffix when one is detected (see [`install_channel`]).
/// * set but **empty** → `None`, the version is hidden entirely (used by the
///   demo recordings so their GIFs never advertise a stale version).
/// * set and **non-empty** → that exact string, verbatim (custom label wins
///   over the channel suffix so recordings stay byte-exact).
fn version_label() -> Option<&'static str> {
    static LABEL: LazyLock<Option<String>> = LazyLock::new(|| {
        compose_version_label(std::env::var("SSHUB_VERSION_LABEL").ok(), install_channel())
    });
    LABEL.as_deref()
}

/// Pure composition of the final label (extracted so the suffix rules can be
/// unit tested without touching process-global env): the compiled default
/// gains the install-channel suffix; a custom label is used verbatim.
fn compose_version_label(var: Option<String>, channel: Option<String>) -> Option<String> {
    let custom = var.as_deref().is_some_and(|s| !s.trim().is_empty());
    let label = resolve_version_label(var)?;
    if custom {
        return Some(label);
    }
    Some(match channel {
        Some(channel) => format!("{label} · {channel}"),
        None => label,
    })
}

/// Pure resolution of the version label from the raw env value (extracted so it
/// can be unit tested without touching process-global env).
fn resolve_version_label(var: Option<String>) -> Option<String> {
    match var {
        // Explicitly hidden (demo recordings set SSHUB_VERSION_LABEL="").
        Some(s) if s.trim().is_empty() => None,
        Some(s) => Some(s),
        None => Some(concat!("v", env!("CARGO_PKG_VERSION")).to_string()),
    }
}

/// How the running binary was installed, shown next to the version so a
/// support conversation can tell how the binary got onto the machine.
///
/// Detection, most specific first:
///
/// * `SSHUB_INSTALL_CHANNEL` set → that value (the npm shim sets `npm`; the
///   prebuilt npm binary is byte-identical to the release tarball one, so no
///   path heuristic can tell them apart).
/// * the binary lives in cargo's install bin dir (`$CARGO_HOME/bin`) → `cargo`.
/// * the binary sits inside a build-target dir (`target/`, `…-target/`) →
///   `source` — a `cargo run`/`cargo build` straight out of the checkout.
/// * the binary lives in `~/.local/bin` → `source` (`just install`).
/// * otherwise → `None`: a distro package or a manual copy shows no suffix.
fn install_channel() -> Option<String> {
    static CHANNEL: LazyLock<Option<String>> = LazyLock::new(|| {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let cargo_bin = match std::env::var_os("CARGO_HOME") {
            Some(dir) => Some(std::path::PathBuf::from(dir).join("bin")),
            None => home.as_ref().map(|h| h.join(".cargo").join("bin")),
        };
        let local_bin = home.as_ref().map(|h| h.join(".local").join("bin"));
        resolve_install_channel(
            std::env::var("SSHUB_INSTALL_CHANNEL").ok(),
            std::env::current_exe().ok().as_deref(),
            cargo_bin.as_deref(),
            local_bin.as_deref(),
        )
    });
    CHANNEL.clone()
}

/// Pure resolution of the install channel from its inputs (extracted so it can
/// be unit tested without touching process-global env or the exe location).
fn resolve_install_channel(
    env_val: Option<String>,
    exe: Option<&std::path::Path>,
    cargo_bin: Option<&std::path::Path>,
    local_bin: Option<&std::path::Path>,
) -> Option<String> {
    if let Some(channel) = env_val.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return Some(channel.to_string());
    }
    let exe = exe?;
    if cargo_bin.is_some_and(|dir| exe.starts_with(dir)) {
        return Some("cargo".into());
    }
    // A build-target dir (`target/`, or a custom `…-target/` via CARGO_TARGET_DIR).
    if exe.components().any(|c| {
        let name = c.as_os_str();
        name == "target" || name.to_string_lossy().ends_with("-target")
    }) {
        return Some("source".into());
    }
    if local_bin.is_some_and(|dir| exe.starts_with(dir)) {
        return Some("source".into());
    }
    None
}

/// Tab definitions: (number label, display name).
const TABS: [(&str, &str); 5] = [
    ("1", "hosts"),
    ("2", "sftp"),
    ("3", "tunnels"),
    ("4", "identities"),
    ("5", "audit"),
];

/// Render the tab bar into a 1-row `area`.
///
/// * `active_tab` — 1-based index (1 = hosts, 2 = tunnels, …)
/// * `scope_path` — shown at far right, e.g. `"~/.config/sshub"`
pub fn render_tab_bar(
    frame: &mut Frame,
    area: Rect,
    active_tab: usize,
    scope_path: &str,
    theme: &ResolvedTheme,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let buf = frame.buffer_mut();
    let y = area.y;
    let mut x = area.x + 1; // 1-char left margin
    let active_style = theme.style(StyleRole::TabsActive);
    // `tabs.inactive` is the muted role the *number* of a closed tab carries;
    // its label has always sat one step further back, on `text.dim`.
    let inactive_style = theme.style(StyleRole::TabsInactive);
    let dim = theme.style(StyleRole::TextDim);
    let muted = theme.style(StyleRole::TextMuted);

    for (i, (num, label)) in TABS.iter().enumerate() {
        let tab_num = i + 1;
        let is_active = tab_num == active_tab;

        if is_active {
            // Active: number inverted (bright bg, dark fg)
            buf.set_string(x, y, "[", dim);
            x += 1;
            buf.set_string(x, y, num, active_style);
            x += num.len() as u16;
            buf.set_string(x, y, "]", dim);
            x += 1;
            buf.set_string(x, y, " ", muted);
            x += 1;
            buf.set_string(x, y, label, muted);
            x += label.len() as u16;
        } else {
            buf.set_string(x, y, " ", dim);
            x += 1;
            buf.set_string(x, y, num, inactive_style);
            x += num.len() as u16;
            buf.set_string(x, y, " ", dim);
            x += 1;
            buf.set_string(x, y, " ", dim);
            x += 1;
            buf.set_string(x, y, label, dim);
            x += label.len() as u16;
        }

        // Space between tabs
        buf.set_string(x, y, "   ", dim);
        x += 3;
    }

    // Version + scope path — far right. The version (when shown) sits at the
    // very edge; the scope path is placed to its left, or hugs the edge itself
    // when the version is hidden (SSHUB_VERSION_LABEL="").
    // No `Style` role resolves to the highlight slot alone, and the scope path
    // has always been the brightest text on this row.
    let scope_style = Style::default().fg(theme.semantic().text_highlight);
    let draw_scope = |buf: &mut ratatui::buffer::Buffer, right_x: u16| {
        let scope_len = (7 + scope_path.len()) as u16; // "scope: " + path
        if right_x > area.x + scope_len + 2 {
            let scope_x = right_x - scope_len;
            buf.set_string(scope_x, y, "scope: ", dim);
            buf.set_string(scope_x + 7, y, scope_path, scope_style);
        }
    };

    match version_label() {
        Some(version) => {
            // Columns, not bytes: the channel suffix's " · " separator is one
            // column but two UTF-8 bytes.
            let ver_len = version.chars().count() as u16;
            if area.width > ver_len + 2 {
                let ver_x = area.x + area.width - ver_len - 1;
                let ok = Style::default().fg(theme.color(ColorRole::StatusSuccess));
                buf.set_string(ver_x, y, version, ok);
                // Two-space gap between the scope path and the version.
                draw_scope(buf, ver_x.saturating_sub(2));
            }
        }
        None => draw_scope(buf, area.x + area.width - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::{compose_version_label, resolve_install_channel, resolve_version_label};
    use std::path::Path;

    #[test]
    fn install_channel_resolution() {
        let ch = |env_val: Option<&str>,
                  exe: Option<&Path>,
                  cargo_bin: Option<&Path>,
                  local_bin: Option<&Path>| {
            resolve_install_channel(env_val.map(str::to_string), exe, cargo_bin, local_bin)
        };

        // The env var wins outright — the npm shim sets it because the prebuilt
        // npm binary is byte-identical to the release-tarball one.
        assert_eq!(ch(Some("npm"), None, None, None).as_deref(), Some("npm"));
        // A blank env value falls through to the path heuristics.
        assert_eq!(
            ch(Some("   "), Some(Path::new("/usr/bin/sshub")), None, None),
            None
        );

        // cargo install → $CARGO_HOME/bin (or the default ~/.cargo/bin).
        assert_eq!(
            ch(
                None,
                Some(Path::new("/home/u/.cargo/bin/sshub")),
                Some(Path::new("/home/u/.cargo/bin")),
                None
            )
            .as_deref(),
            Some("cargo")
        );

        // A dev run straight out of a build dir — plain `target/` and a custom
        // `…-target/` (CARGO_TARGET_DIR) both count.
        assert_eq!(
            ch(
                None,
                Some(Path::new("/repo/target/release/sshub")),
                None,
                None
            )
            .as_deref(),
            Some("source")
        );
        assert_eq!(
            ch(
                None,
                Some(Path::new("/repo/.cargo-target/debug/sshub")),
                None,
                None
            )
            .as_deref(),
            Some("source")
        );

        // `just install` destination.
        assert_eq!(
            ch(
                None,
                Some(Path::new("/home/u/.local/bin/sshub")),
                None,
                Some(Path::new("/home/u/.local/bin"))
            )
            .as_deref(),
            Some("source")
        );

        // A distro package or manual copy — no channel, no suffix.
        assert_eq!(
            ch(None, Some(Path::new("/usr/bin/sshub")), None, None),
            None
        );
        // No exe path resolvable at all.
        assert_eq!(ch(None, None, None, None), None);
    }

    #[test]
    fn version_label_gains_channel_suffix() {
        let v = concat!("v", env!("CARGO_PKG_VERSION"));
        // Default label + detected channel → suffixed.
        assert_eq!(
            compose_version_label(None, Some("npm".into())).as_deref(),
            Some(concat!("v", env!("CARGO_PKG_VERSION"), " · npm"))
        );
        assert_eq!(
            compose_version_label(None, Some("cargo".into())).as_deref(),
            Some(concat!("v", env!("CARGO_PKG_VERSION"), " · cargo"))
        );
        // No channel detected → the plain compiled label.
        assert_eq!(compose_version_label(None, None).as_deref(), Some(v));
        // Hidden stays hidden regardless of the channel.
        assert_eq!(
            compose_version_label(Some(String::new()), Some("npm".into())),
            None
        );
        // A custom label wins verbatim — no suffix appended.
        assert_eq!(
            compose_version_label(Some("demo".into()), Some("npm".into())).as_deref(),
            Some("demo")
        );
    }

    #[test]
    fn version_label_resolution() {
        // Unset → the compiled version.
        assert_eq!(
            resolve_version_label(None).as_deref(),
            Some(concat!("v", env!("CARGO_PKG_VERSION")))
        );
        // Empty (or whitespace) → hidden.
        assert_eq!(resolve_version_label(Some(String::new())), None);
        assert_eq!(resolve_version_label(Some("   ".into())), None);
        // Non-empty → that exact custom label.
        assert_eq!(
            resolve_version_label(Some("demo".into())).as_deref(),
            Some("demo")
        );
    }
}
