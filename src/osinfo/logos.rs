//! Vendored, colorized OS logos.
//!
//! The logo art lives in `assets/os_logos.json` (21 canonical ids) and is
//! embedded at compile time via `include_str!`. Each logo is a block of
//! Braille art (2x4 dots per cell) rendered from the official distro logo, plus
//! one brand `color` (`[r, g, b]`) applied to the whole glyph. The art is built
//! offline on the dev machine from the `font-logos` SVG set via
//! `magick -alpha extract` + `chafa --symbols braille` (see the design spec).

use std::collections::HashMap;
use std::sync::OnceLock;

use ratatui::style::{Color, Style};
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

/// Raw logo as stored in the JSON: a brand color plus the Braille art lines.
#[derive(Deserialize)]
struct RawLogo {
    /// Brand color `[r, g, b]` applied to every glyph cell of the logo.
    color: [u8; 3],
    /// Braille art, one string per row.
    lines: Vec<String>,
}

/// True if a line carries no visible glyphs — empty, or only blank Braille
/// (`U+2800`) / whitespace. Used to trim trailing spacer rows.
fn line_is_blank(line: &str) -> bool {
    line.chars().all(|c| c == '\u{2800}' || c.is_whitespace())
}

/// Convert a raw logo (interning its id) into a resolved [`OsLogo`]: each art
/// row becomes one span styled in the brand color. Trailing blank rows trimmed.
fn build_logo(id: &'static str, raw: RawLogo) -> OsLogo {
    let [r, g, b] = raw.color;
    let style = Style::default().fg(Color::Rgb(r, g, b));

    let mut lines = raw.lines;
    while lines.last().map(|l| line_is_blank(l)).unwrap_or(false) {
        lines.pop();
    }
    let lines = lines
        .into_iter()
        .map(|text| OsLogoLine(vec![OsLogoSpan { text, style }]))
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
    "almalinux",
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
        assert!(logo_for("windows").is_none()); // not a Linux distro, dropped
        assert!(logo_for("generic").is_none());
        assert!(logo_for("").is_none());
        assert!(logo_for("not-a-distro").is_none());
    }

    #[test]
    fn trailing_empty_lines_are_trimmed() {
        // The vendored art may carry trailing blank rows for spacing; they must
        // not survive into the resolved logo.
        let logo = logo_for("arch").unwrap();
        let last = logo.lines.last().unwrap();
        let text: String = last.0.iter().map(|s| s.text.as_str()).collect();
        assert!(!line_is_blank(&text), "trailing blank line was not trimmed");
    }

    #[test]
    fn max_width_is_positive() {
        let logo = logo_for("ubuntu").unwrap();
        assert!(os_logo_max_width(logo) > 0);
    }

    #[test]
    fn parse_output_ids_resolve_to_logos() {
        // Cross-module guard: every canonical id `parse_os` can emit must have a
        // vendored logo.
        use crate::osinfo::parse_os;
        let cases = [
            ("ID=arch\n", "arch"),
            ("ID=ubuntu\n", "ubuntu"),
            ("ID=debian\n", "debian"),
            ("ID=opensuse-leap\n", "opensuse"),
            ("ID=pop\n", "popos"),
            ("ID=almalinux\n", "almalinux"),
            ("Darwin\n", "macos"),
            ("FreeBSD\n", "freebsd"),
            ("Linux\n", "linux"),
        ];
        for (raw, expected) in cases {
            let id = parse_os(raw).unwrap_or_else(|| panic!("{raw:?} did not parse"));
            assert_eq!(id, expected);
            assert!(logo_for(id).is_some(), "{id} parsed but has no logo");
        }
    }
}
