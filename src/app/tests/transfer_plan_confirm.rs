use super::{key, key_char, test_app};
use crate::app::profile_transfer::{
    group_scope_prompt, needs_explicit_confirm, plan_text, rank_profile_names, TransferDialog,
    TransferScope,
};
use crate::app::{App, AppMode};
use crate::store::transfer::{PlannedItem, TransferPlan};
use crate::tui::screens::help::{filtered_help_items, HelpItem};
use crossterm::event::KeyCode;

fn sample_plan() -> TransferPlan {
    TransferPlan {
        dest_profile: "work".to_string(),
        items: vec![
            PlannedItem {
                kind: "host",
                src_name: "web".into(),
                dest_name: "web".into(),
                renamed: false,
                blocked: None,
            },
            PlannedItem {
                kind: "identity",
                src_name: "deploy".into(),
                dest_name: "deploy (copy 1)".into(),
                renamed: true,
                blocked: None,
            },
            PlannedItem {
                kind: "host",
                src_name: "live".into(),
                dest_name: "live".into(),
                renamed: false,
                blocked: Some("active session".into()),
            },
        ],
        ..Default::default()
    }
}

#[test]
fn transfer_tui_plan_confirm_dialog() {
    let plan = sample_plan();
    let text = plan_text(&plan);
    // Golden text: the render is deterministic, so pin it byte-for-byte.
    let expected = "Transfer to profile 'work':\n  [host] 'web'\n  [identity] 'deploy' -> 'deploy (copy 1)' (renamed: name clash in destination)\n  [host] 'live' — blocked: active session\n2 to transfer, 1 blocked — y: transfer    Esc: cancel";
    assert_eq!(text, expected, "plan text golden mismatch:\n{text}");
    let dialog = TransferDialog::new(&plan);
    assert_eq!(dialog.plan_text, text);
    assert!(
        needs_explicit_confirm(&plan),
        "transfers always require explicit confirmation"
    );
    assert_eq!(plan.runnable().len(), 2);
    assert_eq!(plan.blocked().len(), 1);
    assert_eq!(plan.blocked()[0].src_name, "live");
}

#[test]
fn transfer_tui_group_only_vs_with_hosts_prompt() {
    let prompt = group_scope_prompt("dev");
    assert!(prompt.contains("dev"), "prompt names the group:\n{prompt}");
    assert!(
        prompt.contains("group-only"),
        "prompt names the group-only choice:\n{prompt}"
    );
    assert!(
        prompt.contains("with-hosts"),
        "prompt names the with-hosts choice:\n{prompt}"
    );
    assert!(
        prompt.contains("stay"),
        "group-only keeps member hosts in the source profile:\n{prompt}"
    );
    let group_only = TransferPlan {
        dest_profile: "work".into(),
        items: vec![PlannedItem {
            kind: "group",
            src_name: "dev".into(),
            dest_name: "dev".into(),
            renamed: false,
            blocked: None,
        }],
        ..Default::default()
    };
    let with_hosts = TransferPlan {
        dest_profile: "work".into(),
        items: vec![
            PlannedItem {
                kind: "group",
                src_name: "dev".into(),
                dest_name: "dev".into(),
                renamed: false,
                blocked: None,
            },
            PlannedItem {
                kind: "host",
                src_name: "web".into(),
                dest_name: "web".into(),
                renamed: false,
                blocked: None,
            },
            PlannedItem {
                kind: "host",
                src_name: "db".into(),
                dest_name: "db".into(),
                renamed: false,
                blocked: None,
            },
        ],
        ..Default::default()
    };
    let only_text = plan_text(&group_only);
    let with_text = plan_text(&with_hosts);
    assert!(
        !only_text.contains("web") && with_text.contains("web"),
        "with-hosts carries member hosts, group-only does not:\n{only_text}\n---\n{with_text}"
    );
    assert!(
        with_text.contains("db") && with_text.contains("[group] 'dev'"),
        "with-hosts lists every member plus the group shell:\n{with_text}"
    );
    assert!(
        only_text.contains("[group] 'dev'"),
        "group-only still renders the group shell:\n{only_text}"
    );
    assert!(
        with_text.lines().count() > only_text.lines().count(),
        "with-hosts renders more plan lines"
    );
}
fn manager_app(names: &[&str]) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let roots = crate::profile::RootDirs {
        data_root: dir.path().join("data"),
        config_root: dir.path().join("config"),
        compat: false,
    };
    let mut state = crate::profile::ProfileState::default();
    for name in names {
        crate::profile::create_profile(&roots, &mut state, name).unwrap();
    }
    let active = state.profiles[0].clone();
    state.last_used = Some(active.id.clone());
    state.save(&roots.data_root).unwrap();
    let mut app = test_app(vec![]);
    app.profile = Some(crate::profile::profile_paths(
        &roots,
        &active,
        dir.path().join("ssh_config"),
    ));
    app.open_profile_manager();
    assert_eq!(app.mode, AppMode::ProfilePicker);
    (dir, app)
}

fn render_to_buffer(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| crate::tui::render(frame, app))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn buffer_contains(buffer: &ratatui::buffer::Buffer, needle: &str) -> bool {
    let area = buffer.area;
    for y in area.y..area.y + area.height {
        let line: String = (area.x..area.x + area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if line.contains(needle) {
            return true;
        }
    }
    false
}

/// Regression: the `h` scope read `self.hosts[self.selected]`, but `selected`
/// is a nav-row index that counts group headers, so a grouped dashboard named
/// the wrong host (or none). It also sent the label, which the engine (keyed
/// by host name) never matches.
#[test]
fn transfer_host_scope_follows_the_selected_nav_row() {
    use crate::app::NavRow;
    use crate::store::{NewHost, NewHostGroup};

    let store = super::test_store();
    for (group, host, label) in [("alpha", "a1", None), ("beta", "b1", Some("Beta box"))] {
        let g = store
            .create_group(&NewHostGroup {
                name: group.into(),
                ..Default::default()
            })
            .unwrap();
        store
            .create_host(&NewHost {
                name: host.into(),
                label: label.map(Into::into),
                address: "10.0.0.1".into(),
                group_id: Some(g.id),
                ..Default::default()
            })
            .unwrap();
    }
    let mut app = App::new_with_deps(
        crate::config::AppConfig::default(),
        crate::app::AppDeps {
            resolver: Box::new(super::MockResolver::new(vec![])),
            metadata: std::sync::Arc::new(crate::metadata::MetadataDb::default()),
            store,
            password_store: Box::new(crate::credentials::NoopPasswordStore),
        },
    );
    app.reload_hosts().unwrap();
    // Tree: [Header alpha, Host a1, Header beta, Host b1]
    assert!(matches!(app.nav_rows[3], NavRow::Host(_)));

    app.selected = 1;
    assert_eq!(app.transfer_host_name().as_deref(), Some("a1"));
    app.selected = 3;
    assert_eq!(app.transfer_host_name().as_deref(), Some("b1"));
    app.selected = 2;
    assert_eq!(app.transfer_host_name(), None, "a group header is no host");
}

#[test]
fn transfer_dialog_lines_cover_every_stage() {
    // Oracle: the staged keymap in `App::handle_key_profile_transfer`
    // (destination → scope → group choice → plan → explicit `y`) — each stage
    // must advertise exactly the keys the handler reads, or the dialog lies.
    let (_dir, mut app) = manager_app(&["default", "work"]);
    app.handle_key(key_char('T')).unwrap();
    let pending = app.pending_transfer.as_ref().expect("T arms a transfer");
    let lines = pending.dialog_lines();
    assert!(
        lines.iter().any(|l| l.contains("Destination")),
        "destination stage names itself:\n{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("Esc")),
        "destination stage offers Esc:\n{lines:?}"
    );

    let pending = app.pending_transfer.as_mut().unwrap();
    pending.dest_name = Some("work".into());
    let lines = pending.dialog_lines();
    for key in ["h", "g", "i", "t"] {
        assert!(
            lines.iter().any(|l| l.contains(key)),
            "scope stage advertises '{key}':\n{lines:?}"
        );
    }

    let pending = app.pending_transfer.as_mut().unwrap();
    pending.scope = Some(TransferScope::Group {
        name: "dev".into(),
        with_hosts: false,
    });
    let lines = pending.dialog_lines();
    assert!(
        lines.iter().any(|l| l.contains('g') && l.contains('w')),
        "group stage offers group-only vs with-hosts:\n{lines:?}"
    );

    let pending = app.pending_transfer.as_mut().unwrap();
    pending.plan = Some(sample_plan());
    let lines = pending.dialog_lines();
    assert!(
        lines.iter().any(|l| l.contains('y') || l.contains('Y')),
        "plan stage demands explicit confirm:\n{lines:?}"
    );
}

#[test]
fn transfer_flow_renders_a_centered_dialog_not_inline_picker_text() {
    // Oracle: the real renderer (`crate::tui::render` into a TestBackend
    // buffer) — the dialog must exist as a registered popup, and the old
    // inline picker line must be gone from every cell.
    let (_dir, mut app) = manager_app(&["default", "work"]);
    app.handle_key(key_char('T')).unwrap();
    let buffer = render_to_buffer(&app, 80, 24);
    assert!(
        buffer_contains(&buffer, "Transfer profile"),
        "dialog title missing from buffer"
    );
    assert!(
        !buffer_contains(&buffer, "Transfer to profile: "),
        "inline picker transfer text must be gone"
    );
    let rect = app
        .last_popup_rect
        .get()
        .expect("dialog registers popup_open_rect like every sibling popup");
    assert!(
        rect.width > 0 && rect.height > 0,
        "dialog rect must be non-empty: {rect:?}"
    );
}

#[test]
fn transfer_dest_picker_truncates_overlong_query_inside_its_rect() {
    // Oracle: the real renderer buffer — a 100-char query must reach the
    // screen ellipsized, never painted whole past the dialog rect.
    let (_dir, mut app) = manager_app(&["default", "work"]);
    app.handle_key(key_char('T')).unwrap();
    let pending = app.pending_transfer.as_mut().unwrap();
    pending.dest_buffer = "w".repeat(100);
    pending.refresh_dest_filter();
    let buffer = render_to_buffer(&app, 80, 24);
    assert!(
        !buffer_contains(&buffer, &"w".repeat(100)),
        "overlong query must not reach the buffer whole"
    );
    assert!(
        buffer_contains(&buffer, "…"),
        "truncation must show an ellipsis"
    );
    let rect = app.last_popup_rect.get().expect("dialog rect registered");
    assert!(
        rect.width <= 80 && rect.right() <= 80,
        "dialog stays inside the frame: {rect:?}"
    );
}

#[test]
fn transfer_dest_picker_lists_every_non_active_profile() {
    // Oracle: the real profile registry (`manager_app` builds it through
    // `create_profile`, the same source the picker reads) — the picker must
    // offer every profile the transfer can target, i.e. all but the active one.
    let (_dir, mut app) = manager_app(&["default", "work", "lab"]);
    app.handle_key(key_char('T')).unwrap();
    let pending = app.pending_transfer.as_ref().expect("T arms a transfer");
    assert_eq!(
        pending.dest_candidates,
        vec!["work".to_string(), "lab".to_string()],
        "active profile is never a destination"
    );
    assert_eq!(pending.dest_filtered, vec![0, 1]);
    assert_eq!(pending.dest_highlight(), Some("work"));
    let lines = pending.dialog_lines();
    assert!(
        lines.iter().any(|l| l.contains("work")) && lines.iter().any(|l| l.contains("lab")),
        "picker rows name the candidates:\n{lines:?}"
    );
}

#[test]
fn transfer_dest_ranking_is_fuzzy_not_substring() {
    // Oracle: nucleo itself — the ranking must match a scattered abbreviation
    // (no substring present) and keep registry order on an empty query,
    // mirroring the `rank_snippets` contract.
    let names = vec!["work".to_string(), "home".to_string(), "lab".to_string()];
    assert_eq!(rank_profile_names(&names, ""), vec![0, 1, 2]);
    assert_eq!(rank_profile_names(&names, "wk"), vec![0]);
    assert!(
        rank_profile_names(&names, "zzz").is_empty(),
        "no match must yield no rows"
    );
}

#[test]
fn transfer_dest_picker_typing_narrows_to_the_fuzzy_match() {
    // Oracle: the staged keymap in `App::handle_key_profile_transfer` — typed
    // chars filter through the nucleo ranking, so even a scattered
    // abbreviation narrows the list, and a dead query renders the empty
    // state instead of dropping the dialog.
    let (_dir, mut app) = manager_app(&["default", "work", "lab"]);
    app.handle_key(key_char('T')).unwrap();
    for c in "wrk".chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    let pending = app.pending_transfer.as_ref().expect("picker still open");
    assert_eq!(pending.dest_filtered, vec![0]);
    assert_eq!(pending.dest_highlight(), Some("work"));
    for c in "zzz".chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    let pending = app.pending_transfer.as_ref().expect("picker still open");
    assert!(
        pending.dest_filtered.is_empty(),
        "dead query matches nothing"
    );
    assert!(
        pending
            .dialog_lines()
            .iter()
            .any(|l| l.contains("no matching profiles")),
        "empty state renders in-bounds:\n{:?}",
        pending.dialog_lines()
    );
}

#[test]
fn transfer_dest_picker_enter_chooses_the_highlighted_profile() {
    // Oracle: `App::choose_transfer_dest` — Enter must validate exactly the
    // highlighted name (missing database still rejected), advancing the flow
    // to the scope stage; Enter on an empty match keeps the dialog open with
    // an in-bounds notice instead.
    let (dir, mut app) = manager_app(&["default", "work"]);
    let db_dir = dir.path().join("data").join("profiles").join("work");
    std::fs::create_dir_all(&db_dir).unwrap();
    std::fs::write(db_dir.join("launcher.db"), b"").unwrap();
    app.handle_key(key_char('T')).unwrap();
    app.handle_key(key_char('w')).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    let pending = app.pending_transfer.as_ref().expect("flow continues");
    assert_eq!(pending.dest_name.as_deref(), Some("work"));
    assert!(
        pending
            .dialog_lines()
            .iter()
            .any(|l| l.contains("Transfer what to 'work'?")),
        "flow advances to the scope stage"
    );

    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert!(app.pending_transfer.is_none());
    app.handle_key(key_char('T')).unwrap();
    for c in "zzz".chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    app.handle_key(key(KeyCode::Enter)).unwrap();
    let pending = app.pending_transfer.as_ref().expect("dialog stays open");
    assert_eq!(pending.dest_name, None);
    assert!(
        pending
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("no matching profile")),
        "empty Enter explains itself: {:?}",
        pending.notice
    );
}

#[test]
fn transfer_dest_picker_arrows_move_the_highlight() {
    // Oracle: the staged keymap in `App::handle_key_profile_transfer` — ↑↓
    // move the highlight (clamped at both ends) and Enter validates
    // whichever row is highlighted.
    let (dir, mut app) = manager_app(&["default", "work", "waste", "lab"]);
    let db_dir = dir.path().join("data").join("profiles").join("waste");
    std::fs::create_dir_all(&db_dir).unwrap();
    std::fs::write(db_dir.join("launcher.db"), b"").unwrap();
    app.handle_key(key_char('T')).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(
        app.pending_transfer.as_ref().unwrap().dest_highlight(),
        Some("waste")
    );
    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(
        app.pending_transfer.as_ref().unwrap().dest_highlight(),
        Some("lab")
    );
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    assert_eq!(
        app.pending_transfer.as_ref().unwrap().dest_highlight(),
        Some("lab"),
        "selection clamps at the last match"
    );
    app.handle_key(key(KeyCode::Up)).unwrap();
    app.handle_key(key(KeyCode::Up)).unwrap();
    app.handle_key(key(KeyCode::Up)).unwrap();
    assert_eq!(
        app.pending_transfer.as_ref().unwrap().dest_highlight(),
        Some("work"),
        "selection clamps at the first match"
    );
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(
        app.pending_transfer.as_ref().unwrap().dest_name.as_deref(),
        Some("waste")
    );
}

#[test]
fn transfer_dialog_stays_inside_a_narrow_frame() {
    // Oracle: the real renderer buffer (`crate::tui::render` into a 60-column
    // TestBackend) with the long plan strings from the user report — the
    // dialog rect must sit fully inside the frame with margins, and a
    // cell-by-cell diff against the dialog-less baseline must show zero
    // dialog paint outside the rect.
    let (_dir, mut app) = manager_app(&["default", "work"]);
    app.config.appearance.disable_animation = true;
    app.handle_key(key_char('T')).unwrap();
    let plan = TransferPlan {
        dest_profile: "work".to_string(),
        items: vec![
            PlannedItem {
                kind: "identity",
                src_name: "Default".into(),
                dest_name: "Default (copy 1)".into(),
                renamed: true,
                blocked: None,
            },
            PlannedItem {
                kind: "host",
                src_name: "staging-web-01".into(),
                dest_name: "staging-web-01".into(),
                renamed: false,
                blocked: Some("active session".into()),
            },
        ],
        ..Default::default()
    };
    let pending = app.pending_transfer.as_mut().unwrap();
    pending.dest_name = Some("work".into());
    pending.scope = Some(TransferScope::Identity("Default".into()));
    pending.plan = Some(plan);
    let (width, height) = (60u16, 20u16);
    let buffer = render_to_buffer(&app, width, height);
    let rect = app.last_popup_rect.get().expect("dialog rect registered");
    assert!(
        buffer_contains(&buffer, "Transfer profile"),
        "title stays visible in a narrow frame"
    );
    assert!(
        rect.right() <= width && rect.bottom() <= height,
        "dialog fully inside the frame: {rect:?}"
    );
    assert!(
        rect.x >= 1 && rect.right() < width,
        "one-cell side margins: {rect:?}"
    );
    assert!(
        !buffer_contains(&buffer, "(renamed: name clash in destination)"),
        "long plan line must be clipped, not painted whole"
    );
    assert!(buffer_contains(&buffer, "…"), "clipping shows an ellipsis");
    // Cell-by-cell: no dialog paint outside the rect. The baseline is the
    // same frame with the dialog taken off — retried past a wall-clock
    // second tick (the dashboard header clock is the only time-varying cell
    // here, and a tick between the two renders would read as a false
    // escape). Genuine dialog paint is deterministic, so it fails every
    // retry instead.
    let stash = app.pending_transfer.take();
    let mut escapes = Vec::new();
    for _ in 0..5 {
        let dialogless = render_to_buffer(&app, width, height);
        escapes = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                x < rect.left() || x >= rect.right() || y < rect.top() || y >= rect.bottom()
            })
            .filter(|&(x, y)| buffer[(x, y)].symbol() != dialogless[(x, y)].symbol())
            .collect();
        if escapes.is_empty() {
            break;
        }
    }
    app.pending_transfer = stash;
    assert!(
        escapes.is_empty(),
        "dialog paint escaped its rect at: {escapes:?}"
    );
}

#[test]
fn transfer_dialog_esc_cancels_at_every_stage() {
    // Oracle: `App::handle_key_profile_transfer` — Esc is the only key every
    // stage handles, so the manager must survive it with no transfer armed.
    let (_dir, mut app) = manager_app(&["default", "work"]);
    app.handle_key(key_char('T')).unwrap();
    app.handle_key(key(crossterm::event::KeyCode::Esc)).unwrap();
    assert!(app.pending_transfer.is_none());
    assert_eq!(app.mode, AppMode::ProfilePicker);

    app.handle_key(key_char('T')).unwrap();
    app.pending_transfer.as_mut().unwrap().dest_name = Some("work".into());
    app.handle_key(key(crossterm::event::KeyCode::Esc)).unwrap();
    assert!(app.pending_transfer.is_none());
    assert_eq!(app.mode, AppMode::ProfilePicker);

    app.handle_key(key_char('T')).unwrap();
    let pending = app.pending_transfer.as_mut().unwrap();
    pending.dest_name = Some("work".into());
    pending.scope = Some(TransferScope::Group {
        name: "dev".into(),
        with_hosts: false,
    });
    app.handle_key(key(crossterm::event::KeyCode::Esc)).unwrap();
    assert!(app.pending_transfer.is_none());

    app.handle_key(key_char('T')).unwrap();
    let pending = app.pending_transfer.as_mut().unwrap();
    pending.dest_name = Some("work".into());
    pending.scope = Some(TransferScope::Host("web".into()));
    pending.plan = Some(sample_plan());
    app.handle_key(key(crossterm::event::KeyCode::Esc)).unwrap();
    assert!(app.pending_transfer.is_none());
    assert_eq!(app.mode, AppMode::ProfilePicker);
}

#[test]
fn profile_manager_footer_advertises_transfer_key() {
    // Oracle: the real renderer buffer — the `T` hint must be a rendered
    // footer cell in the manager, not just a handler no one can discover.
    let (_dir, app) = manager_app(&["default", "work"]);
    let buffer = render_to_buffer(&app, 80, 24);
    assert!(
        buffer_contains(&buffer, "transfer"),
        "manager footer must advertise the transfer key"
    );
}

#[test]
fn help_screen_lists_profile_transfer_key() {
    // Oracle: `HELP_ITEMS` through its own filter — `filtered_help_items`
    // walks the same list the renderer walks, so a hit here is a hit on
    // screen (empty query stays byte-identical to the rendered layout).
    let items = filtered_help_items("transfer", "Alt+P");
    assert!(
        items
            .iter()
            .any(|item| matches!(item, HelpItem::Entry { key: "t/T", .. })),
        "help must list the t/T transfer key: {items:?}"
    );
}

#[test]
fn transfer_arm_accepts_lowercase_t_from_list() {
    // Oracle: `App::pending_transfer` after one keystroke — lowercase `t`
    // must arm exactly like `T` when the picker shows the list.
    let (_dir, mut app) = manager_app(&["default", "work"]);
    app.handle_key(key_char('t')).unwrap();
    assert!(
        app.pending_transfer.is_some(),
        "lowercase t arms a transfer from the list view"
    );
}

#[test]
fn transfer_arm_ignores_t_typed_as_profile_name() {
    // Oracle: the picker view + `pending_transfer` — `T` typed while creating
    // a profile is name text (typing `Test`), so no transfer may arm and the
    // editor must stay open.
    let (_dir, mut app) = manager_app(&["default", "work"]);
    app.handle_key(key_char('n')).unwrap();
    assert_eq!(
        app.profile_picker.as_ref().map(|p| p.is_list_view()),
        Some(false),
        "n must open the create editor"
    );
    for c in ['T', 'e', 's', 't'] {
        app.handle_key(key_char(c)).unwrap();
    }
    assert!(
        app.pending_transfer.is_none(),
        "typing `Test` as a profile name must not arm a transfer"
    );
    assert_eq!(
        app.profile_picker.as_ref().map(|p| p.is_list_view()),
        Some(false),
        "editor stays open after typing the name"
    );
}

#[test]
fn transfer_confirm_on_empty_plan_refuses_with_notice() {
    // Oracle: the dialog notice + staged flow — `y` on an empty plan must
    // refuse and keep the flow (plan intact) instead of applying zero items.
    let (_dir, mut app) = manager_app(&["default", "work"]);
    app.handle_key(key_char('T')).unwrap();
    let pending = app.pending_transfer.as_mut().expect("T arms a transfer");
    pending.dest_name = Some("work".into());
    pending.scope = Some(TransferScope::Host("web".into()));
    pending.plan = Some(TransferPlan {
        dest_profile: "work".into(),
        items: Vec::new(),
        ..Default::default()
    });
    app.handle_key(key_char('y')).unwrap();
    let pending = app
        .pending_transfer
        .as_ref()
        .expect("flow survives refusal");
    assert!(
        pending
            .notice
            .as_ref()
            .is_some_and(|notice| notice.contains("nothing to transfer")),
        "refusal carries a notice, got: {:?}",
        pending.notice
    );
    assert!(
        pending.plan.is_some(),
        "refused plan stays staged for retry or Esc"
    );
    assert_eq!(app.mode, AppMode::ProfilePicker);
}

/// The guided migration end to end through real keys: destination, `m`, the
/// five checklist steps, review, `y`. Source and destination are real
/// `launcher.db` files, read back after the apply.
#[test]
fn guided_migration_moves_host_group_tunnel_identity_and_audit() {
    use crate::store::{LauncherStore, NewHost, NewIdentity, NewTunnel, TunnelType};

    let (dir, mut app) = manager_app(&["default", "work"]);
    let work_db = dir.path().join("data/profiles/work/launcher.db");
    std::fs::create_dir_all(work_db.parent().unwrap()).unwrap();
    LauncherStore::open(&work_db).unwrap();

    let src_db = app.profile.as_ref().unwrap().launcher_db();
    let src = LauncherStore::open(&src_db).unwrap();
    let identity = src
        .create_identity(&NewIdentity {
            name: "deploy".into(),
            username: None,
            private_key: None,
            certificate: None,
            sort_order: 0,
            has_password: true,
        })
        .unwrap();
    for (name, identity_id) in [("web", Some(identity.id)), ("db", None)] {
        src.create_host(&NewHost {
            identity_id,
            // `web` has a saved password, `db` does not.
            has_password: name == "web",
            ..NewHost::launcher(name, "10.0.0.1")
        })
        .unwrap();
    }
    let web = src.get_host_by_name("web").unwrap().unwrap().id;
    src.create_tunnel(&NewTunnel {
        host_id: Some(web),
        tunnel_type: TunnelType::Local,
        local_port: 8080,
        remote_host: "db".into(),
        remote_port: 5432,
        label: Some("pg".into()),
        auto_connect: false,
    })
    .unwrap();
    src.log_auth_event("web", None, "tui", "ok", "hello", None)
        .unwrap();
    src.log_auth_event("db", None, "tui", "ok", "stays", None)
        .unwrap();
    drop(src);
    app.store = std::sync::Arc::new(LauncherStore::open(&src_db).unwrap());
    // Saved secrets in the source; the destination store is a file in the
    // temp profile, never the real OS keyring.
    let secrets = std::sync::Arc::new(super::MemoryPasswordStore::default());
    {
        use crate::credentials::{host_key, identity_key, PasswordStore};
        secrets.set(&host_key(web), "host-pw").unwrap();
        secrets.set(&identity_key(identity.id), "key-pass").unwrap();
    }
    app.password_store = std::sync::Arc::new(super::SharedStore(secrets.clone()));
    app.open_profile_secrets = |paths| {
        Box::new(crate::credentials::NamespacedPasswordStore::new(
            Box::new(crate::credentials::FilePasswordStore::new(
                paths.credentials_file(),
            )),
            paths.credential_prefix(),
        ))
    };

    app.handle_key(key_char('T')).unwrap();
    for c in "work".chars() {
        app.handle_key(key_char(c)).unwrap();
    }
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key_char('m')).unwrap();
    let lines = |app: &App| app.pending_transfer.as_ref().unwrap().dialog_lines();
    assert!(app.pending_transfer.as_ref().unwrap().screen.is_some());
    // `c` with nothing staged is refused in place.
    app.handle_key(key_char('c')).unwrap();
    assert!(app
        .pending_transfer
        .as_ref()
        .unwrap()
        .screen
        .as_ref()
        .unwrap()
        .notice
        .as_deref()
        .is_some_and(|n| n.contains("nothing staged")));
    // Rows: [(no group) 2], db, web. Stage `web` with Enter.
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key_char('c')).unwrap();
    assert!(
        lines(&app)[0].contains("step 3/5: Tunnels"),
        "{:?}",
        lines(&app)
    );
    for step in ["step 4/5: Identities", "step 5/5: Audit"] {
        app.handle_key(key(KeyCode::Enter)).unwrap();
        assert!(lines(&app)[0].contains(step), "{:?}", lines(&app));
    }
    app.handle_key(key(KeyCode::Enter)).unwrap();
    let review = lines(&app).join("\n");
    for needle in [
        "[host] 'web'",
        "[tunnel] 'pg'",
        "[identity] 'deploy'",
        "[audit] events of 'web'",
        "b: back",
    ] {
        assert!(review.contains(needle), "{needle} missing:\n{review}");
    }
    assert!(!review.contains("'db'"), "unchosen host planned:\n{review}");
    assert!(buffer_contains(
        &render_to_buffer(&app, 100, 30),
        "[audit] events of"
    ));

    // Move, then confirm.
    app.handle_key(key_char('m')).unwrap();
    app.handle_key(key_char('y')).unwrap();
    assert!(app.pending_transfer.is_none(), "applied and closed");

    let dest = LauncherStore::open(&work_db).unwrap();
    let dest_host = dest.get_host_by_name("web").unwrap().expect("moved host");
    let dest_identity = dest.get_identity_by_name("deploy").unwrap().unwrap();
    assert!(dest_host.has_password && dest_identity.has_password);
    {
        // The secrets followed the rows (under the destination's new ids)
        // and are gone from the source store.
        use crate::credentials::{host_key, identity_key, PasswordStore};
        let dest_secrets = (app.open_profile_secrets)(&app.transfer_dest_paths("work").unwrap());
        assert_eq!(
            dest_secrets
                .get(&host_key(dest_host.id))
                .unwrap()
                .as_deref(),
            Some("host-pw")
        );
        assert_eq!(
            dest_secrets
                .get(&identity_key(dest_identity.id))
                .unwrap()
                .as_deref(),
            Some("key-pass")
        );
        assert_eq!(secrets.get(&host_key(web)).unwrap(), None);
        assert_eq!(secrets.get(&identity_key(identity.id)).unwrap(), None);
    }
    assert_eq!(dest.list_tunnels().unwrap().len(), 1);
    assert_eq!(dest.list_auth_events_for_host("web").unwrap().len(), 1);
    let src = LauncherStore::open(&src_db).unwrap();
    assert!(src.get_host_by_name("web").unwrap().is_none());
    assert!(src.list_auth_events_for_host("web").unwrap().is_empty());
    assert_eq!(src.list_auth_events_for_host("db").unwrap().len(), 1);
}

#[test]
fn guided_migration_back_returns_from_review_to_the_last_step() {
    use crate::store::{LauncherStore, NewHost};

    let (dir, mut app) = manager_app(&["default", "work"]);
    let work_db = dir.path().join("data/profiles/work/launcher.db");
    std::fs::create_dir_all(work_db.parent().unwrap()).unwrap();
    LauncherStore::open(&work_db).unwrap();
    let src_db = app.profile.as_ref().unwrap().launcher_db();
    let src = LauncherStore::open(&src_db).unwrap();
    src.create_host(&NewHost::launcher("web", "10.0.0.1"))
        .unwrap();
    drop(src);
    app.store = std::sync::Arc::new(LauncherStore::open(&src_db).unwrap());

    app.handle_key(key_char('T')).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap(); // only candidate: work
    app.handle_key(key_char('m')).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap(); // [(no group) 1], web
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key_char('c')).unwrap(); // no dependents: straight to review
    let pending = app.pending_transfer.as_ref().unwrap();
    assert!(matches!(pending.scope, Some(TransferScope::Migration(_))));
    app.handle_key(key_char('b')).unwrap();
    let pending = app.pending_transfer.as_ref().unwrap();
    assert!(pending.scope.is_none() && pending.plan.is_none());
    assert!(pending.dialog_lines()[0].contains("step 1/5: Hosts"));
    // `b` on the first step returns to the dual-pane screen, queue intact.
    app.handle_key(key_char('b')).unwrap();
    let pending = app.pending_transfer.as_ref().unwrap();
    assert!(pending.wizard.is_none());
    let screen = pending.screen.as_ref().expect("back on the screen");
    assert_eq!(screen.queue.len(), 1);
    assert_eq!(screen.queue[0].host, "web");
}

/// A big migration must keep its summary and the move/copy and `y` hints on
/// screen: the popup drops overflow from the bottom.
#[test]
fn long_plan_keeps_summary_and_hints_visible() {
    use crate::store::transfer::PlannedItem;

    let (_dir, mut app) = manager_app(&["default", "work"]);
    app.handle_key(key_char('T')).unwrap();
    let pending = app.pending_transfer.as_mut().unwrap();
    pending.dest_name = Some("work".into());
    pending.scope = Some(TransferScope::Host("h0".into()));
    pending.plan = Some(TransferPlan {
        dest_profile: "work".into(),
        items: (0..40)
            .map(|i| PlannedItem {
                kind: "host",
                src_name: format!("h{i}"),
                dest_name: format!("h{i}"),
                renamed: false,
                blocked: None,
            })
            .collect(),
        ..Default::default()
    });
    let lines = pending.dialog_lines();
    assert!(lines.len() <= 18, "{} lines", lines.len());
    let text = lines.join("\n");
    for needle in [
        "30 more item(s)",
        "40 to transfer",
        "y: transfer",
        "move/copy",
    ] {
        assert!(text.contains(needle), "{needle} missing:\n{text}");
    }
    let buffer = render_to_buffer(&app, 80, 24);
    assert!(buffer_contains(&buffer, "y: transfer"));
    assert!(buffer_contains(&buffer, "move/copy"));
}

#[test]
fn backspace_on_the_first_wizard_step_does_not_leave_the_wizard() {
    use crate::store::{LauncherStore, NewHost, NewTunnel, TunnelType};

    let (dir, mut app) = manager_app(&["default", "work"]);
    let work_db = dir.path().join("data/profiles/work/launcher.db");
    std::fs::create_dir_all(work_db.parent().unwrap()).unwrap();
    LauncherStore::open(&work_db).unwrap();
    let web = app
        .store
        .create_host(&NewHost::launcher("web", "10.0.0.1"))
        .unwrap()
        .id;
    app.store
        .create_tunnel(&NewTunnel {
            host_id: Some(web),
            tunnel_type: TunnelType::Local,
            local_port: 8080,
            remote_host: "db".into(),
            remote_port: 5432,
            label: None,
            auto_connect: false,
        })
        .unwrap();
    app.handle_key(key_char('T')).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key_char('m')).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key_char('c')).unwrap(); // tunnels step
    app.handle_key(key(KeyCode::Backspace)).unwrap(); // back to the hosts step
    app.handle_key(key(KeyCode::Backspace)).unwrap(); // first step: stays
    app.handle_key(key(KeyCode::Left)).unwrap();
    let pending = app.pending_transfer.as_ref().unwrap();
    assert!(pending.wizard.is_some() && pending.screen.is_none());
    assert!(pending.dialog_lines().iter().any(|l| l.contains("[x] web")));
}

/// A move whose secret cannot be written to the destination must not lose it:
/// the host stays in the source, the user is told in the popup, and a retry
/// is possible.
#[test]
fn guided_move_with_a_locked_destination_keeps_the_host_and_warns_in_a_popup() {
    use crate::store::{LauncherStore, NewHost};

    let (dir, mut app) = manager_app(&["default", "work"]);
    let work_db = dir.path().join("data/profiles/work/launcher.db");
    std::fs::create_dir_all(work_db.parent().unwrap()).unwrap();
    LauncherStore::open(&work_db).unwrap();
    let src_db = app.profile.as_ref().unwrap().launcher_db();
    let src = LauncherStore::open(&src_db).unwrap();
    let web = src
        .create_host(&NewHost {
            has_password: true,
            ..NewHost::launcher("web", "10.0.0.1")
        })
        .unwrap()
        .id;
    drop(src);
    app.store = std::sync::Arc::new(LauncherStore::open(&src_db).unwrap());
    let secrets = std::sync::Arc::new(super::MemoryPasswordStore::default());
    {
        use crate::credentials::{host_key, PasswordStore};
        secrets.set(&host_key(web), "host-pw").unwrap();
    }
    app.password_store = std::sync::Arc::new(super::SharedStore(secrets.clone()));
    // A destination store that refuses every write.
    struct Locked;
    impl crate::credentials::PasswordStore for Locked {
        fn get(&self, _: &str) -> anyhow::Result<Option<String>> {
            Ok(None)
        }
        fn set(&self, _: &str, _: &str) -> anyhow::Result<()> {
            anyhow::bail!("keyring locked")
        }
        fn delete(&self, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }
    app.open_profile_secrets = |_| Box::new(Locked);

    app.handle_key(key_char('T')).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key_char('m')).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key_char('c')).unwrap(); // review (no dependents)
    app.handle_key(key_char('m')).unwrap(); // move
    app.handle_key(key_char('y')).unwrap();

    assert_eq!(app.mode, AppMode::Notice, "warning goes to the popup");
    let popup = app.notice_popup.clone().expect("popup text");
    assert!(
        popup.contains("finished with warnings") && popup.contains("web"),
        "{popup}"
    );
    assert!(!popup.contains("host-pw"), "secret leaked into the popup");
    // Retry is possible: the source host and its secret are intact, and the
    // destination holds no half-moved copy that a retry would duplicate.
    let src = LauncherStore::open(&src_db).unwrap();
    assert!(src.get_host_by_name("web").unwrap().is_some());
    let dest = LauncherStore::open(&work_db).unwrap();
    assert!(dest.get_host_by_name("web").unwrap().is_none());
    use crate::credentials::{host_key, PasswordStore};
    assert_eq!(
        secrets.get(&host_key(web)).unwrap().as_deref(),
        Some("host-pw")
    );
}

/// Two real profiles on disk: the active one (`default`, with host `a1`) and
/// `work` (with host `p1`). `app.store` points at the active profile's file so
/// reloads see what a transfer wrote.
fn two_profile_app() -> (
    tempfile::TempDir,
    App,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    use crate::store::{LauncherStore, NewHost};

    let (dir, mut app) = manager_app(&["default", "work"]);
    let work_db = dir.path().join("data/profiles/work/launcher.db");
    std::fs::create_dir_all(work_db.parent().unwrap()).unwrap();
    let work = LauncherStore::open(&work_db).unwrap();
    work.create_host(&NewHost::launcher("p1", "10.0.0.2"))
        .unwrap();
    let src_db = app.profile.as_ref().unwrap().launcher_db();
    let src = LauncherStore::open(&src_db).unwrap();
    src.create_host(&NewHost::launcher("a1", "10.0.0.1"))
        .unwrap();
    drop(src);
    app.store = std::sync::Arc::new(LauncherStore::open(&src_db).unwrap());
    // Never the real keyring: both profiles use file stores in the temp dir.
    app.open_profile_secrets = |paths| {
        Box::new(crate::credentials::NamespacedPasswordStore::new(
            Box::new(crate::credentials::FilePasswordStore::new(
                paths.credentials_file(),
            )),
            paths.credential_prefix(),
        ))
    };
    (dir, app, work_db, src_db)
}

fn screen_of(app: &App) -> &crate::app::host_transfer::HostTransferScreen {
    app.pending_transfer
        .as_ref()
        .unwrap()
        .screen
        .as_ref()
        .expect("dual-pane screen open")
}

#[test]
fn host_screen_stages_pushes_and_pulls_with_the_right_arrows() {
    let (_dir, mut app, _work_db, _src_db) = two_profile_app();
    app.handle_key(key_char('T')).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap(); // only candidate: work
    app.handle_key(key_char('m')).unwrap();
    assert_eq!(screen_of(&app).left.profile, "default");
    assert_eq!(screen_of(&app).right.profile, "work");

    app.handle_key(key(KeyCode::Down)).unwrap(); // [(no group) 1], a1
    app.handle_key(key(KeyCode::Left)).unwrap(); // left pane cannot pull
    assert!(screen_of(&app).notice.as_deref().unwrap().contains("pulls"));
    assert!(screen_of(&app).queue.is_empty());
    app.handle_key(key(KeyCode::Right)).unwrap();
    assert_eq!(screen_of(&app).queue.len(), 1);
    assert!(!screen_of(&app).queue[0].pull && screen_of(&app).queue[0].host == "a1");
    app.handle_key(key(KeyCode::Right)).unwrap();
    assert_eq!(screen_of(&app).queue.len(), 1, "no duplicate");

    app.handle_key(key(KeyCode::Tab)).unwrap();
    app.handle_key(key(KeyCode::Right)).unwrap(); // right pane cannot push
    assert!(screen_of(&app)
        .notice
        .as_deref()
        .unwrap()
        .contains("pushes"));
    app.handle_key(key(KeyCode::Down)).unwrap(); // p1
    app.handle_key(key(KeyCode::Left)).unwrap();
    let queue = &screen_of(&app).queue;
    assert_eq!(queue.len(), 2);
    assert!(queue[1].pull && queue[1].host == "p1");

    // Both directions are on screen with their badges and the queue strip.
    let buffer = render_to_buffer(&app, 110, 30);
    for needle in [
        "Transfer hosts",
        "default",
        "work",
        "→ a1",
        "← p1",
        "to push",
        "to pull",
    ] {
        assert!(buffer_contains(&buffer, needle), "{needle} missing");
    }

    app.handle_key(key_char('u')).unwrap();
    assert_eq!(screen_of(&app).queue.len(), 1);
    app.handle_key(key_char('x')).unwrap();
    assert!(screen_of(&app).queue.is_empty());

    // `Esc` inside a search clears the filter only; outside it cancels all.
    app.handle_key(key_char('/')).unwrap();
    app.handle_key(key_char('z')).unwrap();
    assert_eq!(
        screen_of(&app)
            .pane(crate::app::host_transfer::Side::Right)
            .filter,
        "z"
    );
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert!(screen_of(&app)
        .pane(crate::app::host_transfer::Side::Right)
        .filter
        .is_empty());
    app.handle_key(key(KeyCode::Esc)).unwrap();
    assert!(app.pending_transfer.is_none());
}

/// A push and a pull staged together run one after the other, push first,
/// each with its own plan review and `y`.
#[test]
fn staged_push_and_pull_apply_in_two_reviews() {
    use crate::store::LauncherStore;

    let (_dir, mut app, work_db, src_db) = two_profile_app();
    app.handle_key(key_char('T')).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key_char('m')).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Right)).unwrap(); // push a1
    app.handle_key(key(KeyCode::Tab)).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap();
    app.handle_key(key(KeyCode::Left)).unwrap(); // pull p1
    app.handle_key(key_char('c')).unwrap();

    // Review 1: the push.
    let pending = app.pending_transfer.as_ref().unwrap();
    assert!(!pending.pull && pending.batches.len() == 1);
    let text = pending.dialog_lines().join("\n");
    assert!(
        text.contains("[host] 'a1'") && !text.contains("'p1'"),
        "{text}"
    );
    assert!(text.contains("Transfer to profile 'work'"), "{text}");
    app.handle_key(key_char('y')).unwrap();
    assert!(LauncherStore::open(&work_db)
        .unwrap()
        .get_host_by_name("a1")
        .unwrap()
        .is_some());

    // Review 2: the pull, with the roles swapped.
    assert_eq!(app.mode, AppMode::ProfilePicker, "dialog stays open");
    let pending = app.pending_transfer.as_ref().expect("next batch armed");
    assert!(pending.pull && pending.batches.is_empty());
    let text = pending.dialog_lines().join("\n");
    assert!(
        text.contains("[host] 'p1'") && !text.contains("'a1'"),
        "{text}"
    );
    assert!(text.contains("Transfer to profile 'default'"), "{text}");
    app.handle_key(key_char('y')).unwrap();
    assert!(app.pending_transfer.is_none(), "all batches done");
    assert_ne!(app.mode, AppMode::ProfilePicker);

    let active = LauncherStore::open(&src_db).unwrap();
    let pulled = active.get_host_by_name("p1").unwrap().expect("pulled host");
    assert_eq!(pulled.address, "10.0.0.2");
    // A copy: both profiles keep their own hosts.
    assert!(active.get_host_by_name("a1").unwrap().is_some());
    assert!(LauncherStore::open(&work_db)
        .unwrap()
        .get_host_by_name("p1")
        .unwrap()
        .is_some());
}

/// A pull in move mode reads the OTHER profile's credential store and writes
/// the active one's, then removes the original and its secret.
#[test]
fn pulling_a_host_in_move_mode_carries_its_secret_into_the_active_profile() {
    use crate::credentials::{host_key, PasswordStore};
    use crate::store::{HostUpdate, LauncherStore};

    let (_dir, mut app, work_db, _src_db) = two_profile_app();
    let work = LauncherStore::open(&work_db).unwrap();
    let p1 = work.get_host_by_name("p1").unwrap().unwrap().id;
    work.update_host(
        p1,
        &HostUpdate {
            has_password: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    let other_paths = app.transfer_dest_paths("work").unwrap();
    let other_secrets = (app.open_profile_secrets)(&other_paths);
    other_secrets.set(&host_key(p1), "pulled-pw").unwrap();
    let active_secrets = std::sync::Arc::new(super::MemoryPasswordStore::default());
    app.password_store = std::sync::Arc::new(super::SharedStore(active_secrets.clone()));

    app.handle_key(key_char('T')).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key_char('m')).unwrap();
    app.handle_key(key(KeyCode::Tab)).unwrap();
    app.handle_key(key(KeyCode::Down)).unwrap(); // p1
    app.handle_key(key(KeyCode::Left)).unwrap();
    app.handle_key(key_char('c')).unwrap();
    app.handle_key(key_char('m')).unwrap(); // move
    app.handle_key(key_char('y')).unwrap();
    assert!(app.pending_transfer.is_none());

    let active = LauncherStore::open(app.profile.as_ref().unwrap().launcher_db()).unwrap();
    let landed = active.get_host_by_name("p1").unwrap().expect("pulled host");
    assert!(landed.has_password);
    assert_eq!(
        active_secrets.get(&host_key(landed.id)).unwrap().as_deref(),
        Some("pulled-pw")
    );
    // Moved: gone from `work`, and so is its secret there.
    assert!(work.get_host_by_name("p1").unwrap().is_none());
    assert_eq!(other_secrets.get(&host_key(p1)).unwrap(), None);
}

/// Regression: with four or more hosts queued the strip needs five rows; on an
/// 8-row terminal it ran past the buffer and panicked.
#[test]
fn host_screen_renders_a_full_queue_on_a_tiny_terminal() {
    let (_dir, mut app, _work_db, _src_db) = two_profile_app();
    app.handle_key(key_char('T')).unwrap();
    app.handle_key(key(KeyCode::Enter)).unwrap();
    app.handle_key(key_char('m')).unwrap();
    let screen = app
        .pending_transfer
        .as_mut()
        .unwrap()
        .screen
        .as_mut()
        .unwrap();
    for i in 0..5 {
        screen.queue.push(crate::app::host_transfer::Staged {
            pull: false,
            host: format!("h{i}"),
        });
    }
    for height in 8..=12 {
        render_to_buffer(&app, 80, height);
    }
}
