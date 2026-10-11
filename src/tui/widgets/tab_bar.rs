//! Dashboard tab bar — numbered tabs with active highlight.

use std::sync::LazyLock;

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::Frame;

use crate::theme::catalog::{ColorRole, StyleRole};
use crate::theme::model::ResolvedTheme;

/// The version string shown at the far right, resolved once at runtime.
///
/// * `SSHUB_VERSION_LABEL` **unset** → `v{CARGO_PKG_VERSION}`, plus an
///   install-channel suffix when one is detected (see
///   [`crate::update::install_channel`]).
/// * set but **empty** → `None`, the version is hidden entirely (used by the
///   demo recordings so their GIFs never advertise a stale version).
/// * set and **non-empty** → that exact string, verbatim (custom label wins
///   over the channel suffix so recordings stay byte-exact).
fn version_label() -> Option<&'static str> {
    static LABEL: LazyLock<Option<String>> = LazyLock::new(|| {
        compose_version_label(
            std::env::var("SSHUB_VERSION_LABEL").ok(),
            crate::update::install_channel(),
        )
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
/// * `update` — release-check state appended to the version, e.g. `↑ v0.18.0`
pub fn render_tab_bar(
    frame: &mut Frame,
    area: Rect,
    active_tab: usize,
    scope_path: &str,
    profile_name: Option<&str>,
    update: Option<&str>,
    theme: &ResolvedTheme,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    // A pending update rides on the version label, in the warning colour.
    let version: Option<std::borrow::Cow<str>> = version_label().map(|v| match update {
        Some(badge) => format!("{v}  {badge}").into(),
        None => v.into(),
    });
    let version_style = Style::default().fg(theme.color(if update.is_some() {
        ColorRole::StatusWarning
    } else {
        ColorRole::StatusSuccess
    }));

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

        // Compact tab spacing leaves room for the profile and badges at 120 columns.
        let gap = if profile_name.is_some() && area.width < 132 {
            " "
        } else {
            "   "
        };
        buf.set_string(x, y, gap, dim);
        x += gap.len() as u16;
    }

    if let Some(name) = profile_name {
        // Reserve installation metadata before sizing the profile name so a
        // long name cannot evict the badges. Tabs always keep their own space.
        let right = area.right().saturating_sub(1);
        let available = right.saturating_sub(x) as usize;
        const PREFIX: &str = "profile: ";
        if available > PREFIX.len() {
            let name_width = Span::raw(name).width();
            let version = version.as_deref();
            let scope_len = 7 + Span::raw(scope_path).width();
            let version_width = version.map(|label| Span::raw(label).width()).unwrap_or(0);
            let version_part = version.map_or(0, |_| 2 + version_width);
            // An update badge outranks the scope path, which never changes:
            // when both do not fit, the scope goes rather than everything.
            let show_scope =
                update.is_none() || PREFIX.len() + 8 + 2 + scope_len + version_part <= available;
            let badges_width = if show_scope {
                scope_len + version_part
            } else {
                version_width
            };
            let reserve_badges = PREFIX.len() + 8 + 2 + badges_width <= available;
            let name_budget =
                available - PREFIX.len() - if reserve_badges { badges_width + 2 } else { 0 };
            let name = if name_width <= name_budget {
                std::borrow::Cow::Borrowed(name)
            } else {
                let span = Span::raw(name);
                let mut shortened = String::new();
                let mut width = 0;
                for grapheme in span.styled_graphemes(Style::default()) {
                    let next_width = Span::raw(grapheme.symbol).width();
                    if width + next_width > name_budget.saturating_sub(1) {
                        break;
                    }
                    shortened.push_str(grapheme.symbol);
                    width += next_width;
                }
                shortened.push('…');
                std::borrow::Cow::Owned(shortened)
            };
            buf.set_string(x, y, PREFIX, dim);
            let profile_style = Style::default().fg(theme.semantic().text_highlight);
            buf.set_stringn(
                x + PREFIX.len() as u16,
                y,
                &name,
                name_budget,
                profile_style,
            );
            if reserve_badges {
                let scope_right = if let Some(version) = version {
                    let version_x = right - version_width as u16;
                    buf.set_stringn(version_x, y, version, version_width, version_style);
                    version_x - 2
                } else {
                    right
                };
                if show_scope {
                    let scope_x = scope_right - scope_len as u16;
                    buf.set_string(scope_x, y, "scope: ", dim);
                    buf.set_stringn(scope_x + 7, y, scope_path, scope_len - 7, profile_style);
                }
            }
        }
        return;
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

    match version.as_deref() {
        Some(version) => {
            // Columns, not bytes: the channel suffix's " · " separator is one
            // column but two UTF-8 bytes.
            let ver_len = version.chars().count() as u16;
            if area.width > ver_len + 2 {
                let ver_x = area.x + area.width - ver_len - 1;
                buf.set_string(ver_x, y, version, version_style);
                // Two-space gap between the scope path and the version.
                draw_scope(buf, ver_x.saturating_sub(2));
            }
        }
        None => draw_scope(buf, area.x + area.width - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::{compose_version_label, resolve_version_label};

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
