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
