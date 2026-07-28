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

/// A `#rrggbb` literal as a [`Color`], so a test can name the marker it wrote
/// into a theme and the value it expects back out of a cell with one constant.
pub(crate) fn marker(rgb: u32) -> ratatui::style::Color {
    ratatui::style::Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// One role of a marker theme: its published path, its foreground marker, and
/// — for the roles whose background is load-bearing — a second marker for that.
pub(crate) struct RoleMarker {
    pub path: &'static str,
    pub fg: u32,
    pub bg: Option<u32>,
}

/// Shorthand for a role marked on its foreground only.
pub(crate) const fn fg(path: &'static str, fg: u32) -> RoleMarker {
    RoleMarker { path, fg, bg: None }
}

/// Shorthand for a role marked on both channels.
pub(crate) const fn fg_bg(path: &'static str, fg: u32, bg: u32) -> RoleMarker {
    RoleMarker {
        path,
        fg,
        bg: Some(bg),
    }
}

/// A theme giving every listed role a colour no other listed role carries.
///
/// This is the only way a renderer test can prove a role is *read*: under
/// `default` an unbound role and a correctly bound one produce the same cell,
/// so parity alone proves nothing. Each marker is unique, so a renderer that
/// reaches for the neighbouring role fails on an exact value.
///
/// The TOML shape (bare string vs `{ foreground = … }`) is looked up from
/// `ROLE_SPECS`, which is a statement about the role's *kind*, never about its
/// value — the value under test is the literal the caller wrote here.
pub(crate) fn role_marker_theme(id: &str, roles: &[RoleMarker]) -> ResolvedTheme {
    use crate::theme::catalog::{RoleRef, ROLE_SPECS};
    use std::collections::BTreeMap;

    let mut seen: BTreeMap<u32, &str> = BTreeMap::new();
    let mut tables: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for role in roles {
        let spec = ROLE_SPECS
            .iter()
            .find(|s| s.path == role.path)
            .unwrap_or_else(|| panic!("`{}` is not a published role", role.path));
        for value in std::iter::once(role.fg).chain(role.bg) {
            if let Some(other) = seen.insert(value, role.path) {
                panic!(
                    "{:#08x} marks both `{other}` and `{}` — a shared marker \
                     cannot tell the two roles apart",
                    value, role.path
                );
            }
        }
        let (table, key) = role.path.rsplit_once('.').expect("role paths are dotted");
        let line = match spec.role {
            RoleRef::Style(_) => match role.bg {
                Some(bg) => format!(
                    "{key} = {{ foreground = \"#{:06x}\", background = \"#{bg:06x}\" }}",
                    role.fg
                ),
                None => format!("{key} = {{ foreground = \"#{:06x}\" }}", role.fg),
            },
            _ => {
                assert!(
                    role.bg.is_none(),
                    "`{}` is not a style role and has no background",
                    role.path
                );
                format!("{key} = \"#{:06x}\"", role.fg)
            }
        };
        tables.entry(table).or_default().push(line);
    }

    let mut src = format!("schema_version = 1\nname = \"{id}\"\nextends = \"default\"\n");
    for (table, lines) in tables {
        src.push_str(&format!("\n[{table}]\n"));
        for line in lines {
            src.push_str(&line);
            src.push('\n');
        }
    }
    resolved_source(id, &src)
}

/// The foreground of the cell where `needle` starts.
pub(crate) fn fg_at_text(buf: &Buffer, needle: &str) -> ratatui::style::Color {
    let at = find_text(buf, needle);
    buf.cell(at).unwrap().fg
}

/// Same, searching from `first_row` down — for a word the panel title repeats.
pub(crate) fn fg_at_text_from(buf: &Buffer, needle: &str, first_row: u16) -> ratatui::style::Color {
    let at = find_text_from(buf, needle, first_row);
    buf.cell(at).unwrap().fg
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

/// One `components.*` family behind a panel call site.
///
/// A typed enum rather than a free string: a typo becomes a compile error, and
/// whether the family carries a badge is a static property of the family rather
/// than something each test has to remember.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PanelFamily {
    HostList,
    Details,
    SshLog,
    Agent,
    Latency,
    Recent,
    Auth,
    Ping,
    Sftp,
    Broadcast,
}

impl PanelFamily {
    /// The ten families, in the order their marker colours are generated.
    /// Broadcast appears once here but is used by two call sites.
    pub(crate) const ALL: &'static [PanelFamily] = &[
        PanelFamily::HostList,
        PanelFamily::Details,
        PanelFamily::SshLog,
        PanelFamily::Agent,
        PanelFamily::Latency,
        PanelFamily::Recent,
        PanelFamily::Auth,
        PanelFamily::Ping,
        PanelFamily::Sftp,
        PanelFamily::Broadcast,
    ];

    /// The TOML path a marker theme writes this family under.
    pub(crate) fn path(self) -> &'static str {
        match self {
            PanelFamily::HostList => "dashboard.host_list",
            PanelFamily::Details => "dashboard.details",
            PanelFamily::SshLog => "dashboard.ssh_log",
            PanelFamily::Agent => "dashboard.agent",
            PanelFamily::Latency => "dashboard.latency",
            PanelFamily::Recent => "dashboard.recent",
            PanelFamily::Auth => "dashboard.auth",
            PanelFamily::Ping => "dashboard.ping",
            PanelFamily::Sftp => "sftp.panel",
            PanelFamily::Broadcast => "broadcast.panel",
        }
    }

    /// Whether this family's productive caller really passes a badge — and so
    /// whether it publishes a `count` role at all.
    pub(crate) fn has_count(self) -> bool {
        matches!(
            self,
            PanelFamily::HostList | PanelFamily::Sftp | PanelFamily::Broadcast
        )
    }

    fn index(self) -> usize {
        PanelFamily::ALL
            .iter()
            .position(|f| *f == self)
            .expect("every family is in ALL")
    }
}

/// The marker colour of one slot of one family.
///
/// Families differ in the red channel and slots in the blue, so a bundle
/// copy-pasted from the neighbouring panel fails on an exact value rather than
/// on a vague "something changed".
pub(crate) fn panel_marker(family: PanelFamily, slot: u8) -> ratatui::style::Color {
    ratatui::style::Color::Rgb(0x11 + family.index() as u8, 0x00, slot)
}

/// A theme giving all ten panel families their own markers.
pub(crate) fn panel_marker_theme() -> ResolvedTheme {
    let mut src = String::from("schema_version = 1\nname = \"Panels\"\nextends = \"default\"\n");
    for family in PanelFamily::ALL {
        let hex = |slot: u8| {
            let ratatui::style::Color::Rgb(r, g, b) = panel_marker(*family, slot) else {
                unreachable!()
            };
            format!("#{r:02x}{g:02x}{b:02x}")
        };
        src.push_str(&format!(
            "\n[components.{}]\n\
             border = \"{}\"\n\
             border_focused = \"{}\"\n\
             title = {{ foreground = \"{}\" }}\n\
             background = \"{}\"\n",
            family.path(),
            hex(0),
            hex(1),
            hex(2),
            hex(4),
        ));
        if family.has_count() {
            src.push_str(&format!("count = {{ foreground = \"{}\" }}\n", hex(3)));
        }
    }
    resolved_source("panels", &src)
}

/// What a panel frame must look like under [`panel_marker_theme`].
pub(crate) struct PanelProof<'a> {
    pub family: PanelFamily,
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
    let family_path = family.path();

    assert_eq!(
        buf.cell((area.x, area.y)).unwrap().fg,
        panel_marker(family, if focused { 1 } else { 0 }),
        "{family_path} ({state}): top-left border corner"
    );

    let (tx, ty) = find_text(buf, title);
    assert_eq!(
        buf.cell((tx, ty)).unwrap().fg,
        panel_marker(family, 2),
        "{family_path} ({state}): title `{title}`"
    );

    if let Some(c) = count {
        assert!(
            family.has_count(),
            "{family_path} has no published count role, so a badge cannot be proved"
        );
        let (cx, cy) = find_text(buf, c);
        assert_eq!(
            buf.cell((cx, cy)).unwrap().fg,
            panel_marker(family, 3),
            "{family_path} ({state}): count `{c}`"
        );
    }

    assert_eq!(
        buf.cell(body).unwrap().bg,
        panel_marker(family, 4),
        "{family_path} ({state}): panel background at {body:?}"
    );
}
