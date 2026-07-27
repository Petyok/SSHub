//! Neutral fixtures shared by the renderer tests.
//!
//! Everything here is in-memory: themes are parsed from strings beside the
//! embedded `default`, the app is backed by an in-memory SQLite store and a
//! no-op password store, and buffers come from ratatui's `TestBackend`. No test
//! using these helpers touches a real HOME, config file, database or keyring.
//!
//! The helpers live here rather than in one widget's test module because they
//! belong to no single widget: `panel_box`, `hosts_panel`, `middle_stack`,
//! `right_stack`, `sftp`, `broadcast`, `header`, `status_bar`, `blit` and
//! `session::render` all use them. Each widget keeps only the proof of *its own*
//! renderer next to that renderer.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::app::App;
use crate::theme::model::ResolvedTheme;

/// Resolve a theme source in memory: parse it beside the embedded `default` it
/// extends and run the real resolver.
///
/// No filesystem, no HOME, no registry — a marker theme is just a string in the
/// test that needs it.
pub(crate) fn resolved_source(id: &str, source: &str) -> ResolvedTheme {
    use crate::theme::model::{ThemeDefinition, ThemeId, ThemeOrigin};
    use crate::theme::parse::parse_theme;
    use crate::theme::resolve::resolve_theme;
    use std::collections::BTreeMap;

    let root = ThemeId::parse("default").unwrap();
    let root_def = parse_theme(
        root.clone(),
        ThemeOrigin::BuiltIn,
        crate::theme::builtins::source("default").unwrap(),
    )
    .definition
    .expect("`default` parses");
    let this = ThemeId::parse(id).unwrap();
    let def = parse_theme(this.clone(), ThemeOrigin::BuiltIn, source)
        .definition
        .expect("the test theme parses");

    let mut defs: BTreeMap<ThemeId, &ThemeDefinition> = BTreeMap::new();
    defs.insert(root, &root_def);
    defs.insert(this.clone(), &def);
    let outcome = resolve_theme(&this, &defs);
    outcome
        .theme
        .unwrap_or_else(|| panic!("the test theme resolves: {:#?}", outcome.diagnostics))
}

/// The built-in `default`, resolved in memory.
pub(crate) fn resolved_default() -> ResolvedTheme {
    resolved_source(
        "default",
        crate::theme::builtins::source("default").unwrap(),
    )
}

struct NoHosts;

impl crate::ssh::HostResolver for NoHosts {
    fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }
    fn resolve_host(&self, name: &str) -> anyhow::Result<crate::ssh::SshHost> {
        Ok(crate::ssh::SshHost::new(name))
    }
}

/// An app carrying one host (`web-prod`) and `theme`, built entirely in memory.
pub(crate) fn themed_app(theme: ResolvedTheme) -> App {
    use crate::app::{AppDeps, HostEntry};
    use crate::metadata::{HostMetadata, MetadataDb};
    use crate::ssh::SshHost;
    use std::sync::Arc;

    let mut app = App::new_with_deps(
        crate::config::AppConfig::default(),
        AppDeps {
            resolver: Box::new(NoHosts),
            metadata: Arc::new(MetadataDb::default()),
            store: Arc::new(crate::store::LauncherStore::open_in_memory().unwrap()),
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    let mut web = SshHost::new("web-prod");
    web.hostname = Some("10.0.0.1".into());
    web.user = Some("ubuntu".into());
    app.hosts = vec![HostEntry::Legacy {
        host: web,
        meta: HostMetadata {
            host_name: "web-prod".into(),
            tags: vec!["prod".into()],
            favorite: true,
            ..Default::default()
        },
    }];
    app.rebuild_filter();
    app.activate_resolved_theme(std::rc::Rc::new(theme));
    app
}

/// Render into a standalone buffer at the origin, so a test can name absolute
/// coordinates.
pub(crate) fn buffer_at(area: Rect, draw: impl FnOnce(&mut Buffer)) -> Buffer {
    let mut buf = Buffer::empty(area);
    draw(&mut buf);
    buf
}

/// Same, for the renderers that take a `Frame` rather than a `Buffer`.
pub(crate) fn frame_at(area: Rect, draw: impl FnOnce(&mut ratatui::Frame)) -> Buffer {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal.draw(draw).unwrap();
    terminal.backend().buffer().clone()
}

/// The cell where `needle` starts, searched row by row from `first_row`.
///
/// `first_row` exists because a panel *title* often repeats the very word the
/// body is being checked for; passing `area.top() + 1` skips the title row.
pub(crate) fn find_text_from(buf: &Buffer, needle: &str, first_row: u16) -> (u16, u16) {
    let area = buf.area;
    (first_row..area.bottom())
        .find_map(|y| {
            let line: String = (area.left()..area.right())
                .map(|x| buf.cell((x, y)).unwrap().symbol())
                .collect();
            line.find(needle)
                .map(|b| (area.left() + line[..b].chars().count() as u16, y))
        })
        .unwrap_or_else(|| panic!("`{needle}` is not in the rendered buffer below row {first_row}"))
}

/// The cell where `needle` starts, searched over the whole buffer.
pub(crate) fn find_text(buf: &Buffer, needle: &str) -> (u16, u16) {
    find_text_from(buf, needle, buf.area.top())
}

// ── Panel-frame proofs ───────────────────────────────────────
//
// Every `render_panel_box` caller is proved the same way: give its family a
// border, focused border, title, count and background nobody else uses, drive
// the *real* renderer, and read named cells. Comparing role constants proves
// nothing — a caller can pass the right bundle with the wrong arguments, which
// is exactly how the SFTP panes came to hard-code `focused = false`.

/// The ten `components.*` families behind the eleven panel call sites
/// (broadcast is used twice), in the order their marker colours are generated.
pub(crate) const PANEL_FAMILIES: &[&str] = &[
    "dashboard.host_list",
    "dashboard.details",
    "dashboard.ssh_log",
    "dashboard.agent",
    "dashboard.latency",
    "dashboard.recent",
    "dashboard.auth",
    "dashboard.ping",
    "sftp.panel",
    "broadcast.panel",
];

/// The families whose productive caller really supplies a badge. Only these
/// publish a `count` role at all.
const PANEL_FAMILIES_WITH_COUNT: &[&str] =
    &["dashboard.host_list", "sftp.panel", "broadcast.panel"];

fn family_index(family: &str) -> usize {
    PANEL_FAMILIES
        .iter()
        .position(|f| *f == family)
        .unwrap_or_else(|| panic!("`{family}` is not a known panel family"))
}

/// The marker colour of one slot of one family.
///
/// Families differ in the red channel and slots in the blue, so a bundle
/// copy-pasted from the neighbouring panel fails on an exact value rather than
/// on a vague "something changed".
pub(crate) fn panel_marker(family: &str, slot: u8) -> ratatui::style::Color {
    ratatui::style::Color::Rgb(0x11 + family_index(family) as u8, 0x00, slot)
}

/// A theme giving all ten panel families their own five (or four) markers.
pub(crate) fn panel_marker_theme() -> ResolvedTheme {
    let mut src = String::from("schema_version = 1\nname = \"Panels\"\nextends = \"default\"\n");
    for family in PANEL_FAMILIES {
        let hex = |slot: u8| {
            let c = panel_marker(family, slot);
            let ratatui::style::Color::Rgb(r, g, b) = c else {
                unreachable!()
            };
            format!("#{r:02x}{g:02x}{b:02x}")
        };
        src.push_str(&format!(
            "\n[components.{family}]\n\
             border = \"{}\"\n\
             border_focused = \"{}\"\n\
             title = {{ foreground = \"{}\" }}\n\
             background = \"{}\"\n",
            hex(0),
            hex(1),
            hex(2),
            hex(4),
        ));
        if PANEL_FAMILIES_WITH_COUNT.contains(family) {
            src.push_str(&format!("count = {{ foreground = \"{}\" }}\n", hex(3)));
        }
    }
    resolved_source("panels", &src)
}

/// What a panel frame must look like under [`panel_marker_theme`].
pub(crate) struct PanelProof<'a> {
    pub family: &'a str,
    pub focused: bool,
    pub title: &'a str,
    /// The badge text the *productive* caller passes, or `None` where it never
    /// passes one — in which case the family publishes no `count` role and
    /// there is nothing to prove.
    pub count: Option<&'a str>,
    /// A body cell no content is drawn over. The selection bar, list rows and
    /// footers paint their own backgrounds, so which cell is free differs per
    /// panel and the caller names it.
    pub body: (u16, u16),
}

/// Assert that a rendered panel wears its own family's roles.
pub(crate) fn assert_panel_wears(buf: &Buffer, area: Rect, proof: PanelProof<'_>) {
    let PanelProof {
        family,
        focused,
        title,
        count,
        body,
    } = proof;
    let state = if focused { "focused" } else { "unfocused" };

    assert_eq!(
        buf.cell((area.x, area.y)).unwrap().fg,
        panel_marker(family, if focused { 1 } else { 0 }),
        "{family} ({state}): top-left border corner"
    );

    let (tx, ty) = find_text(buf, title);
    assert_eq!(
        buf.cell((tx, ty)).unwrap().fg,
        panel_marker(family, 2),
        "{family} ({state}): title `{title}`"
    );

    if let Some(c) = count {
        assert!(
            PANEL_FAMILIES_WITH_COUNT.contains(&family),
            "{family} has no published count role, so a badge cannot be proved"
        );
        let (cx, cy) = find_text(buf, c);
        assert_eq!(
            buf.cell((cx, cy)).unwrap().fg,
            panel_marker(family, 3),
            "{family} ({state}): count `{c}`"
        );
    }

    assert_eq!(
        buf.cell(body).unwrap().bg,
        panel_marker(family, 4),
        "{family} ({state}): panel background at {body:?}"
    );
}
