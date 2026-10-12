//! Click to focus (#116): a click focuses the form field or picker row the
//! last frame drew under the pointer.

use super::*;
use crate::test_support::find_text;

fn draw(app: &mut App, w: u16, h: u16) -> ratatui::buffer::Buffer {
    app.terminal_area = ratatui::layout::Rect::new(0, 0, w, h);
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
    term.draw(|f| crate::tui::render(f, app)).unwrap();
    term.backend().buffer().clone()
}

#[test]
fn a_click_focuses_the_host_form_field_and_places_the_cursor() {
    let mut app = test_app(vec![]);
    app.enter_host_form(None, false).unwrap();
    app.host_form.as_mut().unwrap().address = "10.0.0.1".into();
    let buf = draw(&mut app, 100, 30);

    let (x, y) = find_text(&buf, "Port: ");
    app.click_at(x + 2, y);
    assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::Port);

    // Three cells into the value: the cursor lands before the fourth char.
    let (x, y) = find_text(&buf, "Address: ");
    let value_x = x + "Address: ".len() as u16;
    app.click_at(value_x + 3, y);
    let form = app.host_form.as_ref().unwrap();
    assert_eq!(form.field, HostFormField::Address);
    assert_eq!(form.cursor, 3);

    // Past the end clamps to the end, and typing goes where the click put it.
    app.click_at(value_x + 40, y);
    assert_eq!(app.host_form.as_ref().unwrap().cursor, 8);
    app.click_at(value_x, y);
    app.handle_key(key_char('x')).unwrap();
    assert_eq!(app.host_form.as_ref().unwrap().address, "x10.0.0.1");
}

#[test]
fn a_click_on_an_unfocused_field_places_the_cursor_under_the_pointer() {
    // An unfocused row starts with a blank marker that the form's trimming
    // wrap drops on screen, so its value sits two cells left of where the
    // line's spans would put it.
    let mut app = test_app(vec![]);
    app.enter_host_form(None, false).unwrap();
    app.host_form.as_mut().unwrap().username = "deploy".into();
    let buf = draw(&mut app, 100, 30);

    let (x, y) = find_text(&buf, "Username: ");
    app.click_at(x + "Username: ".len() as u16 + 3, y);
    let form = app.host_form.as_ref().unwrap();
    assert_eq!(form.field, HostFormField::Username);
    assert_eq!(form.cursor, 3);
}

#[test]
fn a_click_right_of_the_cursor_glyph_lands_on_the_clicked_char() {
    // The field being edited draws `_` at its cursor, one cell that is not
    // part of the value: `_10.0.0.1` with the cursor at the start.
    let mut app = test_app(vec![]);
    app.enter_host_form(None, false).unwrap();
    app.host_form.as_mut().unwrap().address = "10.0.0.1".into();
    let buf = draw(&mut app, 100, 30);

    let (x, y) = find_text(&buf, "_10.0.0.1");
    // The last `1` (char 7) is drawn in cell 8.
    app.click_at(x + 8, y);
    assert_eq!(app.host_form.as_ref().unwrap().cursor, 7);
    app.handle_key(key_char('x')).unwrap();
    assert_eq!(app.host_form.as_ref().unwrap().address, "10.0.0.x1");
}

#[test]
fn a_click_on_a_wide_char_lands_on_that_char() {
    // CJK chars take two cells each: `サ` (char 2) is drawn in cells 4 and 5.
    let mut app = test_app(vec![]);
    app.enter_host_form(None, false).unwrap();
    app.host_form.as_mut().unwrap().label = "日本サーバ".into();
    let buf = draw(&mut app, 100, 30);

    let (x, y) = find_text(&buf, "Label: ");
    let value_x = x + "Label: ".len() as u16;
    for cell in [4, 5] {
        app.click_at(value_x + cell, y);
        let form = app.host_form.as_ref().unwrap();
        assert_eq!(form.field, HostFormField::Label);
        assert_eq!(form.cursor, 2, "cell {cell}");
        // Back to a field without the glyph before the next click.
        app.host_form.as_mut().unwrap().field = HostFormField::Address;
        draw(&mut app, 100, 30);
    }
}

#[test]
fn a_wrapped_host_form_value_does_not_shift_the_rows_below_it() {
    // The host form wraps long values, pushing every later field down a row
    // or more. The click map follows the wrap rather than assuming one row
    // per field.
    let mut app = test_app(vec![]);
    app.enter_host_form(None, false).unwrap();
    app.host_form.as_mut().unwrap().address = "a".repeat(150);
    let buf = draw(&mut app, 100, 30);

    let (x, y) = find_text(&buf, "Username: ");
    app.click_at(x + 3, y);
    assert_eq!(
        app.host_form.as_ref().unwrap().field,
        HostFormField::Username
    );
}

#[test]
fn a_click_focuses_a_tunnel_form_field() {
    let mut app = test_app(vec![("edge", host("edge"))]);
    app.active_tab = 2;
    app.handle_key(key(KeyCode::Char('a'))).unwrap();
    assert_eq!(app.mode, AppMode::TunnelForm);
    app.tunnel_form.as_mut().unwrap().remote_port = "5432".into();
    let buf = draw(&mut app, 100, 30);

    let (_, y) = find_text(&buf, "Dest port");
    let (x, _) = find_text(&buf, "5432");
    app.click_at(x + 2, y);
    let form = app.tunnel_form.as_ref().unwrap();
    assert_eq!(form.active_field, TunnelFormField::RemotePort);
    assert_eq!(form.cursor, 2);
}

#[test]
fn a_click_highlights_a_picker_row_without_choosing_it() {
    let mut app = test_app(vec![]);
    for (name, address) in [
        ("alpha", "10.0.0.1"),
        ("bravo", "10.0.0.2"),
        ("charlie", "10.0.0.3"),
    ] {
        app.store
            .create_host(&crate::store::NewHost::launcher(name, address))
            .unwrap();
    }
    app.reload_hosts().unwrap();
    app.active_tab = 2;
    app.handle_key(key(KeyCode::Char('a'))).unwrap();
    app.open_tunnel_host_picker();
    assert_eq!(app.mode, AppMode::TunnelHostPicker);
    let buf = draw(&mut app, 100, 30);

    // Inside the picker: the dashboard behind it lists the same names.
    let popup = app.last_popup_rect.get().expect("the picker drew");
    let (x, y) = (popup.top()..popup.bottom())
        .find_map(|y| {
            let row: String = (popup.left()..popup.right())
                .map(|x| buf[(x, y)].symbol())
                .collect();
            row.find("charlie")
                .map(|b| (popup.left() + row[..b].chars().count() as u16, y))
        })
        .expect("charlie is listed in the picker");
    app.click_at(x, y);
    assert_eq!(app.mode, AppMode::TunnelHostPicker, "a click is not Enter");
    let picked = app.tunnel_host_picker.as_ref().unwrap().selected;
    assert_eq!(app.tunnel_host_matches()[picked].1, "charlie");
}

#[test]
fn a_click_on_the_form_under_an_open_picker_does_nothing() {
    let mut app = test_app(vec![]);
    app.enter_host_form(None, false).unwrap();
    app.open_field_picker(PickerKind::Identity);
    assert_eq!(app.mode, AppMode::FieldPicker);
    let buf = draw(&mut app, 100, 30);

    let before = app.host_form.clone();
    let (x, y) = find_text(&buf, "Port: ");
    app.click_at(x + "Port: ".len() as u16 + 1, y);
    assert_eq!(app.host_form, before);
    assert_eq!(app.mode, AppMode::FieldPicker);
}

#[test]
fn a_click_places_the_cursor_in_a_single_field_prompt() {
    let mut app = test_app(vec![]);
    app.import_prompt = Some(ImportPromptEdit {
        path: "/tmp/export".into(),
        cursor: 11,
        error: None,
    });
    app.mode = AppMode::ImportPrompt;
    let buf = draw(&mut app, 100, 30);

    let (x, y) = find_text(&buf, "/tmp/export");
    app.click_at(x + 4, y);
    assert_eq!(app.import_prompt.as_ref().unwrap().cursor, 4);
}

#[test]
fn a_click_on_a_word_wrapped_prompt_row_lands_on_the_char_drawn_there() {
    // The broadcast prompt wraps at word boundaries without trimming, so its
    // second row starts at the first word that did not fit on the first, not
    // one full row into the command.
    let mut app = test_app(vec![]);
    let command = format!(
        "{}ZZZtail",
        (0..8)
            .map(|i| format!("word{i:02}xxxx "))
            .collect::<String>()
    );
    app.broadcast_setup = Some(BroadcastSetup {
        options: vec![],
        menu_selected: 0,
        target_label: "all".into(),
        cursor: command.chars().count(),
        command,
        candidates: vec![],
        preview_selected: 0,
        edit_targets: false,
    });
    app.mode = AppMode::BroadcastCommand;
    let buf = draw(&mut app, 100, 30);

    let (_, first_row) = find_text(&buf, "cmd> ");
    let (x, y) = find_text(&buf, "ZZZtail");
    assert!(y > first_row, "the command wrapped");
    app.click_at(x, y);
    assert_eq!(app.broadcast_setup.as_ref().unwrap().cursor, 88);
    app.handle_key(key_char('Q')).unwrap();
    assert!(app
        .broadcast_setup
        .as_ref()
        .unwrap()
        .command
        .ends_with("xxxx QZZZtail"));
}

#[test]
fn a_click_on_a_host_form_value_wrapped_below_its_label_lands_on_the_char_drawn_there() {
    // The host form's trimming word wrap moves a value that does not fit
    // after its label to the next row whole, and drops the blank it breaks
    // the value at again further on.
    let mut app = test_app(vec![]);
    app.enter_host_form(None, false).unwrap();
    app.host_form.as_mut().unwrap().remote_command = format!("QQQ{} ZZZ", "x".repeat(62));
    let buf = draw(&mut app, 100, 30);

    let (_, label_row) = find_text(&buf, "Startup command:");
    let (x, y) = find_text(&buf, "QQQ");
    assert_eq!(y, label_row + 1, "the value starts on the next row");
    app.click_at(x + 1, y);
    let form = app.host_form.as_ref().unwrap();
    assert_eq!((form.field, form.cursor), (HostFormField::RemoteCommand, 1));

    // `ZZZ` wrapped to a row of its own, its blank dropped. The field now has
    // focus and draws `_` after the first `Q`, so draw again first.
    let buf = draw(&mut app, 100, 30);
    let (x, z_row) = find_text(&buf, "ZZZ");
    assert_eq!(z_row, y + 1, "the last word wrapped");
    app.click_at(x + 1, z_row);
    assert_eq!(app.host_form.as_ref().unwrap().cursor, 67);
}

#[test]
fn a_click_while_a_popup_slides_open_lands_on_the_field_drawn_there() {
    let mut app = test_app(vec![]);
    app.enter_host_form(None, false).unwrap();
    assert!(app.motion_enabled());
    let (_, resting) = find_text(&draw(&mut app, 100, 30), "Port: ");

    // 100 ms into the 260 ms slide the form is still drawn rows higher up.
    app.mode_entered_at = std::time::Instant::now() - std::time::Duration::from_millis(100);
    let buf = draw(&mut app, 100, 30);
    let (x, y) = find_text(&buf, "Port: ");
    assert!(
        y < resting,
        "drawn mid-slide: row {y}, resting row {resting}"
    );
    app.click_at(x + "Port: ".len() as u16 + 1, y);
    assert_eq!(app.host_form.as_ref().unwrap().field, HostFormField::Port);
}
