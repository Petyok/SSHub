//! Sync tab body and the Sync modals. Everything here draws
//! [`crate::app::sync::SyncView`]; a modal whose state is absent draws nothing.

use ratatui::layout::{Margin, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::sync::{DeviceStatusView, TRUST_TTL_DAYS};
use crate::app::App;
use crate::config::KeyAction;
use crate::sync::oplog::CertScope;
use crate::theme::catalog::{ColorRole, StyleRole};

fn key_of(app: &App, action: KeyAction) -> String {
    super::map::key_label(app.config.keybinds.primary(action))
}

fn color(app: &App, role: ColorRole) -> Style {
    Style::default().fg(app.theme().color(role))
}

fn ago(last_seen: Option<i64>) -> String {
    let Some(at) = last_seen else {
        return "never".into();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    match now - at {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

/// Sync tab body: the ring control panel, or how to start / join a ring.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if area.height < 4 || area.width < 20 {
        return;
    }
    let theme = app.theme();
    let text = theme.style(StyleRole::TextPrimary);
    let muted = theme.style(StyleRole::TextMuted);
    let sync = &app.sync;
    let inner = area.inner(Margin::new(if area.width >= 80 { 2 } else { 0 }, 0));

    let mut lines: Vec<Line> = vec![Line::styled(
        "Your devices",
        theme.style(StyleRole::TextBright),
    )];
    if !sync.configured {
        let enroll = key_of(app, KeyAction::SyncEnroll);
        lines.extend([
            Line::raw(""),
            Line::styled("Sync is not set up on this profile.", text),
            Line::raw(""),
            Line::styled(
                "Sync keeps your hosts and saved passwords the same on your own computers, \
                 encrypted end to end, with no server in the middle.",
                muted,
            ),
            Line::raw(""),
            Line::styled(
                format!(
                    "Start: on the hosts tab select a computer that runs sshub \
                     and press {enroll} to add it as a sync device."
                ),
                text,
            ),
            Line::styled(
                "Join: on a blank computer run `sshub sync-join <a reachable device>`.",
                text,
            ),
        ]);
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }

    let mode = sync.cred_mode.map_or("-", |m| m.label());
    let trust = sync.trust.as_ref().map_or(String::new(), |t| {
        format!(
            "   Trust session: {}, {} days, {} hosts opened",
            t.scope, t.term_days, t.hosts_opened
        )
    });
    lines.push(Line::styled(
        format!(
            "Passwords: {mode}   Waiting for approval: {}{trust}",
            sync.pending
        ),
        muted,
    ));
    // U13-1: can a stolen computer be cut off?
    lines.push(match sync.cut_off_missing {
        None => Line::styled("Can cut off a stolen device: yes", muted),
        Some(n) => Line::styled(
            format!(
                "Can cut off a stolen device: no, add {n} more computer{}",
                if n == 1 { "" } else { "s" }
            ),
            color(app, ColorRole::StatusWarning),
        ),
    });
    if let Some(error) = &sync.last_error {
        lines.push(Line::styled(
            format!("Last sync failed: {error}"),
            color(app, ColorRole::StatusError),
        ));
    } else if sync.syncing {
        let step = sync.progress.as_deref().unwrap_or("connecting");
        lines.push(Line::styled(format!("Syncing: {step}"), muted));
    }
    for (line, urgent) in crate::app::sync::attention_lines(sync) {
        lines.push(Line::styled(
            line,
            color(
                app,
                if urgent {
                    ColorRole::StatusError
                } else {
                    ColorRole::StatusWarning
                },
            ),
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        format!(
            "  {:<18} {:<16} {:<22} {}",
            "device", "kind", "state", "last in sync"
        ),
        muted,
    ));
    let selected = sync.selected.min(sync.devices.len().saturating_sub(1));
    for (i, d) in sync.devices.iter().enumerate() {
        let (glyph, role) = d.status.glyph();
        let mut row = format!(
            " {:<18} {:<16} {:<22} {}",
            if d.this {
                format!("{} (this)", d.name)
            } else {
                d.name.clone()
            },
            d.role_word(),
            d.status.label(),
            ago(d.last_seen),
        );
        if d.unattended {
            row.push_str("  runs on its own");
        }
        let style = if i == selected {
            theme.style(StyleRole::PickerRowSelected)
        } else if d.status == DeviceStatusView::Compromised {
            color(app, ColorRole::StatusError)
        } else {
            text
        };
        // The glyph keeps its status colour even on the selected row.
        lines.push(Line::from(vec![
            Span::styled(glyph, color(app, role)),
            Span::styled(row, style),
        ]));
    }
    if let Some(d) = sync.devices.get(selected) {
        if let Some(note) = d.explain(crate::store::now_ts()) {
            lines.push(Line::raw(""));
            lines.push(Line::styled(note, color(app, ColorRole::StatusWarning)));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Size of a popup for `body` plus `extra` rows below it, before the terminal
/// clamps it: `(width, height, wrapped rows + 1 slack)`.
fn popup_wh(area: Rect, width: u16, body: &[Line], extra: u16) -> (u16, u16, usize) {
    let w = width.min(area.width.saturating_sub(2));
    // Rows after wrapping, so a long paragraph is not cut off by its popup.
    let text_w = usize::from(w.saturating_sub(4)).max(1);
    let rows: usize = 1 + body
        .iter()
        .map(|l| l.width().div_ceil(text_w).max(1))
        .sum::<usize>();
    let h = (rows as u16)
        .saturating_add(extra)
        .saturating_add(4)
        .min(area.height.saturating_sub(2));
    (w, h, rows)
}

/// A centered popup with wrapped `body` and a one-line `hint` on its last row.
fn modal(frame: &mut Frame, app: &App, title: &str, width: u16, body: Vec<Line>, hint: &str) {
    modal_with_extra(frame, app, title, width, body, hint, 0);
}

/// [`modal`] with `extra` rows reserved under the body; returns that area.
fn modal_with_extra(
    frame: &mut Frame,
    app: &App,
    title: &str,
    width: u16,
    body: Vec<Line>,
    hint: &str,
    extra: u16,
) -> Option<Rect> {
    let area = frame.area();
    let (w, h, rows) = popup_wh(area, width, &body, extra);
    if w < 8 || h < 4 {
        return None;
    }
    let popup = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    );
    let popup = crate::tui::popup_open_rect(popup, app);
    let theme = app.theme();
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                format!(" {title} "),
                theme.style(StyleRole::PopupTitle),
            ))
            .border_style(crate::tui::popup_border_style(theme, popup)),
        popup,
    );
    let inner = popup.inner(Margin::new(2, 1));
    let usable = inner.height.saturating_sub(1);
    let body_h = ((rows - 1) as u16).min(usable);
    let body_area = Rect::new(inner.x, inner.y, inner.width, body_h);
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), body_area);
    frame.buffer_mut().set_string(
        inner.x,
        inner.bottom().saturating_sub(1),
        crate::tui::text::ellipsize(hint, inner.width as usize),
        theme.style(StyleRole::PopupHint),
    );
    crate::tui::paint_popup_border(frame, popup, theme);
    // The slack row sits between the body and the extra area.
    let top = inner.y + body_h + 1;
    let extra_h = usable.saturating_sub(body_h + 1).min(extra);
    (extra > 0 && extra_h > 0 && top < inner.bottom())
        .then(|| Rect::new(inner.x, top, inner.width, extra_h))
}

fn row(selected: bool, text: String, app: &App) -> Line<'static> {
    let theme = app.theme();
    if selected {
        Line::styled(
            format!("\u{203a} {text}"),
            theme.style(StyleRole::PickerRowSelected),
        )
    } else {
        Line::styled(format!("  {text}"), theme.style(StyleRole::PickerRow))
    }
}

pub fn render_review(frame: &mut Frame, app: &App) {
    let sync = &app.sync;
    if sync.review.is_empty() {
        return;
    }
    let selected = sync.review_selected.min(sync.review.len() - 1);
    let mut body: Vec<Line> = sync
        .review
        .iter()
        .enumerate()
        .map(|(i, op)| row(i == selected, format!("{}: {}", op.device, op.summary), app))
        .collect();
    body.push(Line::raw(""));
    for c in &sync.review[selected].changes {
        let style = if c.dangerous {
            color(app, ColorRole::StatusError)
        } else {
            app.theme().style(StyleRole::TextPrimary)
        };
        let mark = if c.dangerous { "! " } else { "  " };
        body.push(Line::styled(
            format!("{mark}{}: {} -> {}", c.field, c.from, c.to),
            style,
        ));
    }
    let op = &sync.review[selected];
    if let Some(reason) = &op.reason {
        body.push(Line::styled(
            format!("  waiting because: {reason}"),
            app.theme().style(StyleRole::TextMuted),
        ));
    }
    if let Some(red) = &op.red {
        body.push(Line::raw(""));
        body.push(Line::styled(
            "Big change: it takes effect only with all three approvals",
            color(app, ColorRole::StatusWarning),
        ));
        for have in &red.have {
            body.push(Line::styled(
                format!("  \u{2713} {have}"),
                color(app, ColorRole::StatusSuccess),
            ));
        }
        for missing in &red.missing {
            body.push(Line::styled(
                format!("  \u{25cb} {missing}: missing"),
                color(app, ColorRole::StatusWarning),
            ));
        }
        if red.single_machine {
            body.push(Line::styled(
                "  only one computer has a person at the keyboard: the second device is not needed, the paper phrase is",
                app.theme().style(StyleRole::TextMuted),
            ));
        }
        if let Some(why) = &red.blocked {
            body.push(Line::styled(
                format!("  cannot go through as it stands: {why}"),
                color(app, ColorRole::StatusError),
            ));
        }
        if let Some(note) = &red.note {
            body.push(Line::styled(
                format!("  note: {note}"),
                app.theme().style(StyleRole::TextMuted),
            ));
        }
        if let Some(why) = &red.rule_violation {
            body.push(Line::styled(
                format!("  not allowed: {why}"),
                color(app, ColorRole::StatusError),
            ));
        }
        if red.expired {
            body.push(Line::styled(
                "  the time for this approval round ran out: reject it and propose it again",
                color(app, ColorRole::StatusError),
            ));
        }
    }
    let hint = if op.red.is_some() {
        "\u{2191}\u{2193} select  a add my approval  p paper phrase  e emailed code  r reject  Esc close"
    } else {
        "\u{2191}\u{2193} select  a approve  r reject  Esc close"
    };
    modal(frame, app, "Review changes", 96, body, hint);
}

pub fn render_unlock(frame: &mut Frame, app: &App) {
    let Some(u) = app.sync.unlock.as_ref() else {
        return;
    };
    let theme = app.theme();
    let mut body = vec![
        Line::styled(
            format!("Unlocking {}", u.host),
            theme.style(StyleRole::TextBright),
        ),
        Line::styled(
            format!("{} of {} shares", u.have, u.need),
            theme.style(StyleRole::TextPrimary),
        ),
    ];
    // What the phone's release screen must show too: compare both before approving there.
    if !u.host_id.is_empty() {
        body.push(Line::styled(
            format!("host id  {}", u.host_id),
            theme.style(StyleRole::TextPrimary),
        ));
        body.push(Line::styled(
            format!("words    {}  (compare with the phone)", u.words),
            theme.style(StyleRole::TextPrimary),
        ));
    }
    for p in &u.paths {
        body.push(Line::styled(
            format!("  {}: {}", p.kind.label(), p.status),
            theme.style(StyleRole::TextMuted),
        ));
    }
    body.push(Line::raw(""));
    body.push(Line::styled(
        format!("code or words \u{203a} {}\u{2588}", u.code),
        theme.style(StyleRole::PickerQuery),
    ));

    qr_modal(frame, app, "Unlock", body, &u.frames, &unlock_hint(app, u));
}

/// The footer of the unlock modal: the drop-phone key only while a Bluetooth channel runs.
fn unlock_hint(app: &App, u: &crate::app::sync::UnlockView) -> String {
    let mut hint = String::from("Enter submit  Esc cancel");
    if u.paths
        .iter()
        .any(|p| p.kind == crate::app::sync::UnlockPathKind::Bluetooth)
    {
        hint.push_str(&format!(
            "  {} drop phone",
            key_of(app, KeyAction::SyncBleDrop)
        ));
    }
    hint
}

/// The footer of a generic prompt; the phone enrollment's has the drop-phone key too.
fn prompt_hint(app: &App) -> String {
    let mut hint = String::from("Enter submit  Esc cancel");
    if crate::app::sync_phone::ble_active(app) {
        hint.push_str(&format!(
            "  {} drop phone",
            key_of(app, KeyAction::SyncBleDrop)
        ));
    }
    hint
}

/// A modal with `body` and, when it fits, the looping QR of `frames` under it.
fn qr_modal(
    frame: &mut Frame,
    app: &App,
    title: &str,
    mut body: Vec<Line<'static>>,
    frames: &[Vec<u8>],
    hint: &str,
) {
    use crate::tui::widgets::qr_frames::{cells_needed, needed_version, render as render_qr};
    let theme = app.theme();
    let mut width = 70;
    let mut extra = 0;
    if let Ok(version) = needed_version(frames) {
        let (cw, ch) = cells_needed(version);
        width = width.max(cw + 4);
        extra = ch + 1; // the code and the "frame i/n" line
        let area = frame.area();
        let (w, h, rows) = popup_wh(area, width, &body, extra);
        if w != width || h != rows as u16 + extra + 4 {
            // The popup would be clamped: say what is needed, draw no code.
            let need_w = width + 2;
            let need_h = rows as u16 + extra + 4 + 2;
            body.push(Line::raw(""));
            body.push(Line::styled(
                format!(
                    "The QR code needs a terminal of at least {need_w}x{need_h} (this one is {}x{}).",
                    area.width, area.height
                ),
                app.theme().style(StyleRole::PopupWarning),
            ));
            extra = 0;
        }
    } else if needed_version(frames) == Err(crate::tui::widgets::qr_frames::QrPlan::TooLarge) {
        body.push(Line::raw(""));
        body.push(Line::styled(
            "The challenge is too large for a QR code: use the clipboard or the words.",
            app.theme().style(StyleRole::PopupWarning),
        ));
    }
    let Some(qr_area) = modal_with_extra(frame, app, title, width, body, hint, extra) else {
        return;
    };
    let code_rows = qr_area.height.saturating_sub(1);
    let index = app.sync.qr_frame % frames.len().max(1);
    render_qr(
        frame.buffer_mut(),
        Rect::new(qr_area.x, qr_area.y, qr_area.width, code_rows),
        frames,
        index,
    );
    frame.buffer_mut().set_string(
        qr_area.x,
        qr_area.bottom() - 1,
        format!("frame {}/{}", index + 1, frames.len()),
        theme.style(StyleRole::TextMuted),
    );
}

pub fn render_sas(frame: &mut Frame, app: &App) {
    let Some(s) = app.sync.sas.as_ref() else {
        return;
    };
    let theme = app.theme();
    if !s.peer_has_screen {
        // A server nobody sits at: no words to compare. The question is whether this is your server.
        let mut body = vec![
            Line::styled(
                format!("Is {} your server?", s.peer),
                theme.style(StyleRole::TextBright),
            ),
            Line::raw(""),
        ];
        match &s.ssh_fingerprint {
            Some(fp) => {
                body.push(Line::styled(
                    "Its SSH key fingerprint is:",
                    theme.style(StyleRole::TextPrimary),
                ));
                body.push(Line::styled(fp.clone(), theme.style(StyleRole::TextBright)));
                body.push(Line::raw(""));
                body.push(Line::styled(
                    "Check it the way you trust: your provider's panel, or `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` on the server.",
                    theme.style(StyleRole::TextMuted),
                ));
            }
            None => body.push(Line::styled(
                "ssh already refused any key it did not know when it connected, but sshub could not read the fingerprint to show it here.",
                theme.style(StyleRole::PopupWarning),
            )),
        }
        body.push(Line::raw(""));
        body.push(Line::styled(
            format!("Add {} as a device of yours?", s.peer_name),
            theme.style(StyleRole::TextPrimary),
        ));
        modal(
            frame,
            app,
            "Add this server",
            76,
            body,
            "y yes, it is my server  n / Esc no",
        );
        return;
    }
    let mut body = vec![
        Line::styled(
            format!("Adding {}", s.peer),
            theme.style(StyleRole::TextPrimary),
        ),
        Line::raw(""),
        Line::styled(s.words.clone(), theme.style(StyleRole::TextBright)),
        Line::raw(""),
    ];
    body.push(Line::styled(
        "Both screens must show exactly these four check words. If they differ, someone is in the middle: answer no.",
        theme.style(StyleRole::TextMuted),
    ));
    modal(
        frame,
        app,
        "Compare the four check words",
        70,
        body,
        "y the words match  n / Esc they differ",
    );
}

/// Full-screen alert; takes the whole frame.
pub fn render_fork_alert(frame: &mut Frame, app: &App) {
    let Some(f) = app.sync.fork.as_ref() else {
        return;
    };
    let area = frame.area();
    let theme = app.theme();
    let red = color(app, ColorRole::StatusError);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(red)
            .title(Span::styled(" COMPROMISED DEVICE ", red)),
        area,
    );
    let text = theme.style(StyleRole::TextPrimary);
    let body = vec![
        Line::styled(format!("{} forked its history", f.device), red),
        Line::styled(f.detail.clone(), theme.style(StyleRole::TextMuted)),
    ];
    let mut body = body;
    body.extend(
        f.evidence
            .iter()
            .map(|l| Line::styled(l.clone(), theme.style(StyleRole::TextMuted))),
    );
    let body_rest = vec![
        Line::raw(""),
        Line::styled(
            "It signed two different histories under one identity. Treat everything it has sent as untrusted.",
            text,
        ),
        Line::raw(""),
        Line::styled("What to do now:", text),
        Line::styled(
            "  1. Do not approve changes from this device or release shares to it (review rejects its operations: R, then r).",
            text,
        ),
        Line::styled(
            "  2. Revoke it from a trusted device: `sshub sync-revoke <device>` there, or the Sync tab on a device that holds shares.",
            text,
        ),
        Line::styled(
            "  3. Change every credential it could read (they were open on it), then save them again so they replicate under the new ring.",
            text,
        ),
        Line::styled(
            "  4. If it is yours and was cloned, re-enroll it afterwards as a new device.",
            text,
        ),
        Line::raw(""),
        Line::styled(
            "Esc closes this alert; the sync badge stays red. Enter on its row in the Sync tab shows this again.",
            theme.style(StyleRole::PopupHint),
        ),
    ];
    body.extend(body_rest);
    frame.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: false }),
        area.inner(Margin::new(3, 2)),
    );
}

/// The generic Sync modals: a prompt, a menu or a text panel.
pub fn render_sync_ui(frame: &mut Frame, app: &App) {
    use crate::app::sync_ui::SyncUi;
    let Some(ui) = app.sync_ui.as_ref() else {
        return;
    };
    let theme = app.theme();
    match ui {
        SyncUi::Prompt {
            title,
            label,
            masked,
            value,
            lines,
            frames,
            ..
        } => {
            let shown = if *masked {
                "\u{2022}".repeat(value.chars().count())
            } else {
                // Long pasted answers show their tail, never a wall of text.
                let n = value.chars().count();
                if n > 60 {
                    format!("\u{2026}{}", value.chars().skip(n - 59).collect::<String>())
                } else {
                    value.clone()
                }
            };
            let mut body: Vec<Line> = lines
                .iter()
                .map(|l| Line::styled(l.clone(), theme.style(StyleRole::TextPrimary)))
                .collect();
            if !lines.is_empty() {
                body.push(Line::raw(""));
            }
            body.push(Line::styled(
                label.clone(),
                theme.style(StyleRole::TextPrimary),
            ));
            body.push(Line::styled(
                format!("\u{203a} {shown}\u{2588}"),
                theme.style(StyleRole::PickerQuery),
            ));
            let hint = prompt_hint(app);
            if frames.is_empty() {
                modal(frame, app, title, 76, body, &hint);
            } else {
                qr_modal(frame, app, title, body, frames, &hint);
            }
        }
        SyncUi::Menu {
            title,
            items,
            selected,
        } => {
            let selected = (*selected).min(items.len().saturating_sub(1));
            let body: Vec<Line> = items
                .iter()
                .enumerate()
                .map(|(i, (text, _))| {
                    let mut line = row(i == selected, text.clone(), app);
                    // An entry that does not apply now is greyed.
                    if i != selected && text.contains("(not now: ") {
                        line = Line::styled(
                            format!("  {text}"),
                            app.theme().style(StyleRole::TextMuted),
                        );
                    }
                    line
                })
                .collect();
            modal(
                frame,
                app,
                title,
                70,
                body,
                "\u{2191}\u{2193} select  Enter run  Esc close",
            );
        }
        SyncUi::Info {
            title,
            lines,
            frames,
            then,
        } => {
            let body: Vec<Line> = lines
                .iter()
                .map(|l| Line::styled(l.clone(), theme.style(StyleRole::TextPrimary)))
                .collect();
            let hint = if then.is_some() {
                "Enter go on  Esc stop"
            } else {
                "Esc close"
            };
            if frames.is_empty() {
                modal(frame, app, title, 84, body, hint);
            } else {
                qr_modal(frame, app, title, body, frames, hint);
            }
        }
    }
}

pub fn render_trust_scope(frame: &mut Frame, app: &App) {
    let sync = &app.sync;
    if sync.trust_scopes.is_empty() {
        return;
    }
    let selected = sync.trust_scope_selected.min(sync.trust_scopes.len() - 1);
    let mut body = vec![
        Line::styled(
            "Which hosts may this device open without asking? There is no \"everything\".",
            app.theme().style(StyleRole::TextMuted),
        ),
        Line::raw(""),
    ];
    body.extend(
        sync.trust_scopes
            .iter()
            .zip(&sync.trust_scope_values)
            .enumerate()
            .map(|(i, (s, v))| {
                let marked = matches!(v, CertScope::Hosts(h) if h.len() == 1 && sync.trust_marked.contains(&h[0]));
                let box_ = if marked { "[x]" } else if matches!(v, CertScope::Hosts(_)) { "[ ]" } else { "   " };
                row(i == selected, format!("{box_} {s}"), app)
            }),
    );
    modal(
        frame,
        app,
        "Trust session: scope",
        72,
        body,
        "\u{2191}\u{2193} select  Space mark hosts (max 64)  Enter next  Esc cancel",
    );
}

pub fn render_trust_ttl(frame: &mut Frame, app: &App) {
    let sync = &app.sync;
    let selected = sync.trust_ttl_selected.min(TRUST_TTL_DAYS.len() - 1);
    let scope = crate::app::sync_trust::chosen(sync).map_or("-".to_string(), |(_, l)| l);
    let days = TRUST_TTL_DAYS[selected];
    let me = sync
        .this_device
        .as_ref()
        .and_then(|id| sync.devices.iter().find(|d| d.id == *id))
        .map_or("this computer".to_string(), |d| d.name.clone());
    let mut body = vec![
        Line::styled(
            format!(
                "Let {me} open {scope} without asking, for {days} days, up to {} new hosts a day.",
                sync.trust_budget
            ),
            app.theme().style(StyleRole::TextBright),
        ),
        Line::styled(
            "The phone must approve it. A new host is one it has not opened under this trust session yet.",
            app.theme().style(StyleRole::TextMuted),
        ),
        Line::raw(""),
    ];
    body.extend(TRUST_TTL_DAYS.iter().enumerate().map(|(i, d)| {
        let mark = if i == selected { "(\u{2022})" } else { "( )" };
        row(i == selected, format!("{mark} {d} days"), app)
    }));
    modal(
        frame,
        app,
        "Trust session",
        76,
        body,
        "\u{2191}\u{2193} days  \u{2190}\u{2192} new hosts a day  Enter ask the phone  Esc back",
    );
}

pub fn render_release_confirm(frame: &mut Frame, app: &App) {
    let Some(r) = app.sync.release.as_ref() else {
        return;
    };
    let theme = app.theme();
    let body = vec![
        Line::styled(
            format!("{} ({}) asks for a share", r.requester, r.requester_id),
            theme.style(StyleRole::TextBright),
        ),
        Line::styled(
            format!("to open {} (asked {} s ago)", r.host, r.age_secs),
            theme.style(StyleRole::TextPrimary),
        ),
        Line::raw(""),
        Line::styled(
            "Release only if you recognise that device and expected this request.",
            theme.style(StyleRole::TextMuted),
        ),
    ];
    modal(
        frame,
        app,
        "Release share",
        72,
        body,
        "y release  n / Esc refuse",
    );
}

pub fn render_onboarding(frame: &mut Frame, app: &App) {
    use crate::app::sync::OnboardingStep;
    let Some(o) = app.sync.onboarding.as_ref() else {
        return;
    };
    let theme = app.theme();
    let text = theme.style(StyleRole::TextPrimary);
    let muted = theme.style(StyleRole::TextMuted);
    let bright = theme.style(StyleRole::TextBright);
    let mut body = vec![Line::styled("Mode: Vault", bright), Line::raw("")];
    let hint = match o.step {
        // The one screen that shows the words.
        OnboardingStep::Show => {
            for (i, word) in o.words().iter().enumerate() {
                body.push(Line::styled(format!("  {}  {word}", i + 1), bright));
            }
            body.extend([
                Line::raw(""),
                Line::styled(
                    "These six words are the credential store. There is no account to recover it from: lose them and the credentials are gone.",
                    text,
                ),
                Line::styled(
                    "Write them on paper now. They are not shown again after this screen, and the printed kit is a separate sheet. Do not photograph it: a photo in a phone gallery defeats what the sheet is for.",
                    text,
                ),
                Line::styled(
                    "Vault means a stolen computer reduces the protection to guessing the phrase. Quorum, where one device is not enough, needs a second full device.",
                    muted,
                ),
            ]);
            "Enter: I have written the words down"
        }
        OnboardingStep::Retype => {
            let n = o.quiz[o.quiz_done.min(1)] + 1;
            body.extend([
                Line::styled(
                    "To be sure the paper is right, type two of the words from it.",
                    text,
                ),
                Line::raw(""),
                Line::styled(
                    format!(
                        "word {n} ({} of 2) \u{203a} {}\u{2588}",
                        o.quiz_done + 1,
                        o.typed
                    ),
                    theme.style(StyleRole::PickerQuery),
                ),
            ]);
            if let Some(notice) = &o.notice {
                body.push(Line::raw(""));
                body.push(Line::styled(
                    notice.clone(),
                    theme.style(StyleRole::PopupError),
                ));
            }
            "Enter check  Esc show the words again"
        }
        OnboardingStep::Kit => {
            let field = |i: usize, label: &str, path: &str| {
                let mark = if o.path_field == i { "\u{203a}" } else { " " };
                let cursor = if o.path_field == i { "\u{2588}" } else { "" };
                Line::styled(
                    format!("{mark} {label}: {path}{cursor}"),
                    if o.path_field == i { bright } else { text },
                )
            };
            body.extend([
                Line::styled(
                    "The kit is two printed sheets: the master phrase, and the ring secret as a QR code. A new computer needs both to open saved passwords.",
                    text,
                ),
                Line::styled(
                    "If you lose both sheets and every device that has them set up, saved passwords cannot be recovered. Anyone who finds both sheets can open every saved password: keep them in two places.",
                    muted,
                ),
                Line::raw(""),
                field(0, "phrase sheet", &o.phrase_path),
                field(1, "ring secret sheet", &o.ring_path),
            ]);
            if let Some(notice) = &o.notice {
                body.push(Line::raw(""));
                body.push(Line::styled(
                    notice.clone(),
                    theme.style(StyleRole::PopupError),
                ));
            }
            "Tab other field  Enter write both sheets  Ctrl+S no printer: show the ring secret"
        }
        OnboardingStep::ShowRing => {
            let hex = hex::encode(o.secrets().ring_secret.as_slice());
            let groups: Vec<&str> = (0..hex.len()).step_by(4).map(|i| &hex[i..i + 4]).collect();
            body.extend([
                Line::styled(
                    "The ring secret. Copy it by hand onto paper, in groups of four:",
                    text,
                ),
                Line::raw(""),
                Line::styled(format!("  {}", groups.join(" ")), bright),
                Line::raw(""),
                Line::styled(
                    "A new computer asks for it, with the six master words, when you import the Vault kit. Do not photograph this screen.",
                    muted,
                ),
            ]);
            "Enter: I have copied it, finish  Esc back"
        }
        OnboardingStep::Done => {
            body.extend([
                Line::styled("Both sheets are written (owner-only files):", text),
                Line::styled(format!("  {}", o.phrase_path), bright),
                Line::styled(format!("  {}", o.ring_path), bright),
                Line::raw(""),
                Line::styled(
                    "Print them and keep them in two places. Do not photograph the sheets. Press d after printing to delete the files from this computer.",
                    text,
                ),
            ]);
            if let Some(notice) = &o.notice {
                body.push(Line::raw(""));
                body.push(Line::styled(notice.clone(), text));
            }
            "d delete the files now  Enter finish"
        }
    };
    modal(frame, app, "Set up sync", 76, body, hint);
}

/// The whole screen as text, for tests of other modules.
#[cfg(test)]
pub(crate) fn tests_screen(app: &App, w: u16, h: u16) -> String {
    tests::screen(app, w, h)
}

#[cfg(test)]
mod tests {
    use crate::app::sync::*;
    use crate::app::tests::test_app;
    use crate::app::{App, AppMode};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    pub(crate) fn screen(app: &App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| crate::tui::render(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf.cell((x, y)).unwrap().symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn a_server_nobody_sits_at_is_asked_as_its_ssh_fingerprint_not_as_four_words() {
        let mut app = app_in(AppMode::SyncSas);
        let sas = app.sync.sas.as_mut().unwrap();
        sas.peer_has_screen = false;
        sas.peer = "hub".into();
        sas.ssh_fingerprint = Some("SHA256:Zm9vYmFyYmF6".into());
        let words = sas.words.clone();
        let s = screen(&app, 110, 36);
        for want in [
            "Is hub your server?",
            "SSH key fingerprint",
            "SHA256:Zm9vYmFyYmF6",
            "y yes, it is my server",
        ] {
            assert!(s.contains(want), "{want}: {s}");
        }
        assert!(!s.contains(&words), "no words that prove nothing: {s}");
        assert!(!s.contains("Both screens must show"), "{s}");
        // With a person at the other end the four check words are the question.
        app.sync.sas.as_mut().unwrap().peer_has_screen = true;
        let s = screen(&app, 110, 36);
        assert!(s.contains(&words) && s.contains("four check words"), "{s}");
    }

    fn app_in(mode: AppMode) -> App {
        let mut app = test_app(vec![]);
        app.sync = sample_sync_view_with_modals();
        app.mode = mode;
        app
    }

    #[test]
    fn unconfigured_tab_explains_how_to_start_or_join_a_ring() {
        let mut app = test_app(vec![]);
        app.active_tab = 6;
        let s = screen(&app, 110, 30);
        assert!(s.contains("Sync is not set up on this profile"), "{s}");
        assert!(s.contains("Start:"), "{s}");
        assert!(s.contains("sshub sync-join"), "{s}");
        assert!(s.contains("7 sync") || s.contains("[7] sync"), "{s}");
        assert!(!s.contains('\u{21c5}'), "no badge without a ring: {s}");
    }

    #[test]
    fn configured_tab_lists_devices_and_shows_the_badge() {
        let mut app = test_app(vec![]);
        app.sync = sample_sync_view();
        app.active_tab = 6;
        let s = screen(&app, 120, 30);
        for want in [
            "laptop",
            "server",
            "in sync",
            "away too long",
            "Passwords: Vault",
            "Waiting for approval: 2",
            "Can cut off a stolen device",
            "group staging",
            "\u{21c5}1",
            "2ops",
        ] {
            assert!(s.contains(want), "{want}: {s}");
        }
    }

    #[test]
    fn every_sync_modal_renders_its_content() {
        for (mode, wants) in [
            (
                AppMode::SyncReview,
                &["Review changes", "proxy_command", "nc evil 22"][..],
            ),
            (
                AppMode::SyncUnlock,
                &[
                    "1 of 2 shares",
                    "clipboard: listening",
                    "phone (QR)",
                    "ab-12",
                ][..],
            ),
            (
                AppMode::SyncSas,
                &["harbor-globe-lantern-mint", "Compare the four check words"][..],
            ),
            (
                AppMode::TrustSessionScope,
                &["Trust session: scope", "group prod"][..],
            ),
            (
                AppMode::TrustSessionTtl,
                &["without asking", "30 days", "90 days", "new hosts a day"][..],
            ),
            (
                AppMode::SyncReleaseConfirm,
                &["new laptop", "web-1", "id-9"][..],
            ),
            (
                AppMode::SyncOnboarding,
                &["Vault", "six words", "photograph"][..],
            ),
        ] {
            let app = app_in(mode);
            let s = screen(&app, 110, 36);
            for want in wants {
                assert!(s.contains(want), "{mode:?} {want}: {s}");
            }
        }
    }

    /// Every cell the QR widget drew (half block, pure black/white), as the
    /// module matrix with its quiet zone: `(version, side x side grid)`.
    fn read_qr(buf: &ratatui::buffer::Buffer) -> Option<(i16, Vec<Vec<bool>>)> {
        use ratatui::style::Color;
        let pure = |c: Color| c == Color::Rgb(0, 0, 0) || c == Color::Rgb(255, 255, 255);
        let cells: Vec<(u16, u16)> = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let c = buf.cell((x, y)).unwrap();
                c.symbol() == "\u{2580}" && pure(c.fg) && pure(c.bg)
            })
            .collect();
        let (x0, x1) = (
            cells.iter().map(|c| c.0).min()?,
            cells.iter().map(|c| c.0).max()?,
        );
        let (y0, y1) = (
            cells.iter().map(|c| c.1).min()?,
            cells.iter().map(|c| c.1).max()?,
        );
        let side = (x1 - x0 + 1) as usize;
        let mut grid = vec![vec![false; side]; (y1 - y0 + 1) as usize * 2];
        for y in y0..=y1 {
            for x in x0..=x1 {
                let c = buf.cell((x, y)).unwrap();
                let r = (y - y0) as usize * 2;
                grid[r][(x - x0) as usize] = c.fg == Color::Rgb(0, 0, 0);
                grid[r + 1][(x - x0) as usize] = c.bg == Color::Rgb(0, 0, 0);
            }
        }
        grid.truncate(side);
        Some((((side - 8 - 17) / 4) as i16, grid))
    }

    fn draw(app: &App, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| crate::tui::render(f, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn unlock_modal_shows_each_frame_as_a_decodable_code() {
        use qrcode::{Color as Module, EcLevel, QrCode, Version};
        let mut app = app_in(AppMode::SyncUnlock);
        let frames = app.sync.unlock.as_ref().unwrap().frames.clone();
        let mut seen = Vec::new();
        for i in 0..frames.len() {
            app.sync.qr_frame = i;
            let buf = draw(&app, 110, 60);
            let (version, grid) = read_qr(&buf).expect("a code is drawn");
            let want =
                QrCode::with_version(&frames[i], Version::Normal(version), EcLevel::M).unwrap();
            let n = want.width();
            for (y, row) in grid.iter().enumerate() {
                for (x, &dark) in row.iter().enumerate() {
                    let inside = (4..4 + n).contains(&x) && (4..4 + n).contains(&y);
                    let expect = inside && want.to_colors()[(y - 4) * n + (x - 4)] == Module::Dark;
                    assert_eq!(dark, expect, "frame {i} module ({x},{y})");
                }
            }
            let text: String = (0..buf.area.height)
                .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
                .map(|(x, y)| buf.cell((x, y)).unwrap().symbol().to_string())
                .collect();
            assert!(
                text.contains(&format!("frame {}/{}", i + 1, frames.len())),
                "{i}"
            );
            seen.push(grid);
        }
        assert!(seen[0] != seen[1], "frames differ on screen");
    }

    #[test]
    fn a_small_terminal_gets_the_needed_size_instead_of_a_code() {
        let app = app_in(AppMode::SyncUnlock);
        let buf = draw(&app, 100, 30);
        assert!(read_qr(&buf).is_none(), "no unreadable code is drawn");
        let text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        // The message wraps inside the popup, so match whole tokens, not phrases.
        assert!(
            text.contains("100x30"),
            "the terminal size is reported: {text}"
        );
        assert!(text.contains("QR"), "{text}");
        assert!(text.contains("least"), "{text}");
    }

    #[test]
    fn the_phrase_is_on_screen_in_the_show_step_only() {
        use crate::app::sync::OnboardingStep;
        let mut app = app_in(AppMode::SyncOnboarding);
        let words: Vec<String> = app
            .sync
            .onboarding
            .as_ref()
            .unwrap()
            .words()
            .iter()
            .map(|w| w.to_string())
            .collect();
        let shown = screen(&app, 110, 40);
        for (i, w) in words.iter().enumerate() {
            assert!(shown.contains(&format!("{}  {w}", i + 1)), "{w}: {shown}");
        }
        for (step, want) in [
            (OnboardingStep::Retype, "type two of the words"),
            (OnboardingStep::Kit, "phrase sheet"),
            (OnboardingStep::Done, "Both sheets are written"),
        ] {
            app.sync.onboarding.as_mut().unwrap().step = step;
            let s = screen(&app, 110, 40);
            assert!(s.contains(want), "{step:?}: {s}");
            for w in &words {
                // The first word is also an ordinary English word in some
                // sentences; the numbered list is what must be gone.
                assert!(!s.contains(&format!("  {w}")), "{step:?} still shows {w}");
            }
        }
    }

    #[test]
    fn fork_alert_is_full_screen_and_names_the_device() {
        let app = app_in(AppMode::SyncForkAlert);
        let s = screen(&app, 100, 30);
        assert!(s.contains("COMPROMISED DEVICE"), "{s}");
        assert!(s.contains("server forked its history"), "{s}");
        assert!(s.contains("Revoke it"), "{s}");
        // No dashboard chrome: the tab bar is gone.
        assert!(!s.contains("hosts"), "{s}");
    }

    #[test]
    fn dangerous_review_fields_are_drawn_in_the_error_colour() {
        let app = app_in(AppMode::SyncReview);
        let mut terminal = Terminal::new(TestBackend::new(110, 36)).unwrap();
        terminal.draw(|f| crate::tui::render(f, &app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let red = app
            .theme()
            .color(crate::theme::catalog::ColorRole::StatusError);
        let row_has = |y: u16, want: &str, fg| {
            let text: String = (0..buf.area.width)
                .map(|x| buf.cell((x, y)).unwrap().symbol())
                .collect();
            text.contains(want)
                && (0..buf.area.width).any(|x| {
                    buf.cell((x, y)).unwrap().symbol() != " " && buf.cell((x, y)).unwrap().fg == fg
                })
        };
        let danger = (0..buf.area.height).find(|&y| row_has(y, "proxy_command", red));
        assert!(danger.is_some(), "the dangerous field row is red");
    }
}
