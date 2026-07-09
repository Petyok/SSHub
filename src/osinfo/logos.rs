//! Vendored, colorized OS logos.
//!
//! The logo art lives in `assets/os_logos.json` (21 canonical ids) and is
//! embedded at compile time via `include_str!`. Each logo is a list of lines,
//! each line a list of styled spans. Colors are ANSI 0-7 (+ a `bright` flag)
//! resolved to ratatui [`Color`]s; `fg: null` means "use the terminal default"
//! ([`Color::Reset`]).

use std::collections::HashMap;
use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

/// A single styled run of text within a logo line.
pub struct OsLogoSpan {
    pub text: String,
    pub style: Style,
}

/// One line of a logo, a sequence of styled spans.
pub struct OsLogoLine(pub Vec<OsLogoSpan>);

/// A fully-resolved logo: its canonical id plus its rendered lines.
pub struct OsLogo {
    pub id: &'static str,
    pub lines: Vec<OsLogoLine>,
}

/// Raw span as stored in the JSON: an ANSI color index (0-7) or `null`, plus
/// `bright`/`bold` flags and the literal text.
#[derive(Deserialize)]
struct RawSpan {
    #[serde(default)]
    fg: Option<u8>,
    #[serde(default)]
    bright: bool,
    #[serde(default)]
    bold: bool,
    text: String,
}

/// Raw logo as stored in the JSON.
#[derive(Deserialize)]
struct RawLogo {
    /// The fastfetch source logo name (e.g. `arch_small`); metadata only, kept
    /// so the vendored JSON is self-documenting about its provenance.
    #[allow(dead_code)]
    logo: String,
    lines: Vec<Vec<RawSpan>>,
}

/// Resolve a raw span's `fg`/`bright`/`bold` into a ratatui [`Style`].
///
/// ANSI index mapping: 0 Black, 1 Red, 2 Green, 3 Yellow, 4 Blue, 5 Magenta,
/// 6 Cyan, 7 White. With `bright` set, 0-6 map to the ratatui `Light*` / dark
/// variants; 7 stays White. A `null` fg maps to [`Color::Reset`] (terminal
/// default foreground). `bold` adds [`Modifier::BOLD`].
fn resolve_style(span: &RawSpan) -> Style {
    let color = match span.fg {
        None => Color::Reset,
        Some(i) => {
            let i = i & 0x07;
            if span.bright {
                match i {
                    0 => Color::DarkGray,
                    1 => Color::LightRed,
                    2 => Color::LightGreen,
                    3 => Color::LightYellow,
                    4 => Color::LightBlue,
                    5 => Color::LightMagenta,
                    6 => Color::LightCyan,
                    _ => Color::White,
                }
            } else {
                match i {
                    0 => Color::Black,
                    1 => Color::Red,
                    2 => Color::Green,
                    3 => Color::Yellow,
                    4 => Color::Blue,
                    5 => Color::Magenta,
                    6 => Color::Cyan,
                    _ => Color::White,
                }
            }
        }
    };
    let mut style = Style::default().fg(color);
    if span.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    style
}

/// True if a line has no visible text (all spans empty). Used to trim trailing
/// blank lines the art may carry for spacing.
fn line_is_empty(spans: &[RawSpan]) -> bool {
    spans.iter().all(|s| s.text.is_empty())
}

/// Convert a raw logo (interning its id) into a resolved [`OsLogo`], trimming
/// trailing all-empty lines.
fn build_logo(id: &'static str, raw: RawLogo) -> OsLogo {
    let mut lines = raw.lines;
    // Trim trailing lines whose spans are all empty text.
    while lines.last().map(|l| line_is_empty(l)).unwrap_or(false) {
        lines.pop();
    }
    let lines = lines
        .into_iter()
        .map(|spans| {
            OsLogoLine(
                spans
                    .into_iter()
                    .map(|s| {
                        let style = resolve_style(&s);
                        OsLogoSpan {
                            text: s.text,
                            style,
                        }
                    })
                    .collect(),
            )
        })
        .collect();
    OsLogo { id, lines }
}

/// The 21 canonical logo ids, interned as `'static` literals so [`OsLogo::id`]
/// and the map keys share the same lifetime. Any id present in the JSON but not
/// listed here is ignored; any id here but absent from the JSON is skipped.
const CANONICAL_IDS: [&str; 21] = [
    "arch",
    "ubuntu",
    "debian",
    "alpine",
    "fedora",
    "rocky",
    "rhel",
    "centos",
    "opensuse",
    "linuxmint",
    "manjaro",
    "popos",
    "kali",
    "gentoo",
    "void",
    "nixos",
    "endeavouros",
    "freebsd",
    "macos",
    "windows",
    "linux",
];

static LOGOS: OnceLock<HashMap<&'static str, OsLogo>> = OnceLock::new();

/// Parse and cache the embedded logo set on first use.
fn logos() -> &'static HashMap<&'static str, OsLogo> {
    LOGOS.get_or_init(|| {
        let mut by_id: HashMap<String, RawLogo> =
            serde_json::from_str(include_str!("../../assets/os_logos.json"))
                .expect("assets/os_logos.json is malformed");
        let mut map = HashMap::with_capacity(CANONICAL_IDS.len());
        for &id in CANONICAL_IDS.iter() {
            if let Some(raw) = by_id.remove(id) {
                map.insert(id, build_logo(id, raw));
            }
        }
        map
    })
}

/// Look up a resolved logo by canonical id. Returns `None` for unknown ids or
/// canonical ids that have no vendored art (e.g. `"generic"`, `"almalinux"`).
pub fn logo_for(id: &str) -> Option<&'static OsLogo> {
    logos().get(id)
}

/// Maximum display width across a logo's lines. Helper for sizing a detail-panel
/// sub-column so the logo never clips.
pub fn os_logo_max_width(logo: &OsLogo) -> u16 {
    logo.lines
        .iter()
        .map(|line| {
            // ASCII/block-element art only, so char count == display width.
            let w: usize = line.0.iter().map(|s| s.text.chars().count()).sum();
            w
        })
        .max()
        .unwrap_or(0)
        .min(u16::MAX as usize) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_canonical_id_resolves_non_empty() {
        for &id in CANONICAL_IDS.iter() {
            let logo = logo_for(id).unwrap_or_else(|| panic!("missing logo for {id}"));
            assert_eq!(logo.id, id);
            assert!(!logo.lines.is_empty(), "{id} has no lines");
            // At least one line must carry visible text.
            let has_text = logo
                .lines
                .iter()
                .any(|l| l.0.iter().any(|s| !s.text.is_empty()));
            assert!(has_text, "{id} has no visible text");
        }
    }

    #[test]
    fn unknown_and_logoless_ids_return_none() {
        assert!(logo_for("almalinux").is_none());
        assert!(logo_for("generic").is_none());
        assert!(logo_for("").is_none());
        assert!(logo_for("not-a-distro").is_none());
    }

    #[test]
    fn trailing_empty_lines_are_trimmed() {
        // The vendored art carries a trailing empty line for spacing; it must
        // not survive into the resolved logo.
        let logo = logo_for("arch").unwrap();
        let last = logo.lines.last().unwrap();
        assert!(
            last.0.iter().any(|s| !s.text.is_empty()),
            "trailing empty line was not trimmed"
        );
    }

    #[test]
    fn max_width_is_positive() {
        let logo = logo_for("ubuntu").unwrap();
        assert!(os_logo_max_width(logo) > 0);
    }

    #[test]
    fn parse_output_ids_resolve_to_logos() {
        // Cross-module guard: every canonical id `parse_os` can emit must have a
        // vendored logo, with the sole documented exception of `almalinux`
        // (recognised distro, no art -> `logo_for` returns None on purpose).
        use crate::osinfo::parse_os;
        let cases = [
            ("ID=arch\n", "arch"),
            ("ID=ubuntu\n", "ubuntu"),
            ("ID=debian\n", "debian"),
            ("ID=opensuse-leap\n", "opensuse"),
            ("ID=pop\n", "popos"),
            ("Darwin\n", "macos"),
            ("FreeBSD\n", "freebsd"),
            ("Linux\n", "linux"),
        ];
        for (raw, expected) in cases {
            let id = parse_os(raw).unwrap_or_else(|| panic!("{raw:?} did not parse"));
            assert_eq!(id, expected);
            assert!(logo_for(id).is_some(), "{id} parsed but has no logo");
        }
        // almalinux parses but is intentionally logoless.
        assert_eq!(parse_os("ID=almalinux\n"), Some("almalinux"));
        assert!(logo_for("almalinux").is_none());
    }
}
