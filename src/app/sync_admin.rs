//! The Sync tab's admin menu: one entry per operation that changes the ring.
//! Each flow lives in its own module and is listed here.

use super::sync_ui::MenuFn;
use super::*;

/// A menu entry that applies now, or says in one line why it does not.
type Entry = (&'static str, Result<fn(&mut App), String>);

fn go(label: &'static str, run: fn(&mut App)) -> Entry {
    (label, Ok(run))
}

fn unless(label: &'static str, run: fn(&mut App), reason: Option<&str>) -> Entry {
    (label, reason.map_or(Ok(run), |r| Err(r.to_string())))
}

/// Open a group of entries; one that does not apply is listed with its reason and only says so when chosen.
fn open_group(app: &mut App, title: &str, entries: Vec<Entry>) {
    let items: Vec<(String, MenuFn)> = entries
        .into_iter()
        .map(|(label, how)| match how {
            Ok(run) => (label.to_string(), Box::new(run) as MenuFn),
            Err(reason) => {
                let shown = format!("{label}  (not now: {reason})");
                (
                    shown,
                    Box::new(move |app: &mut App| app.host_notice = Some(reason)) as MenuFn,
                )
            }
        })
        .collect();
    app.sync_menu(title.to_string(), items);
}

/// The top of the Manage menu: five groups and two shortcuts. What a group offers depends on the ring.
pub(crate) fn entries() -> Vec<(String, MenuFn)> {
    let group = |label: &str, run: fn(&mut App)| (label.to_string(), Box::new(run) as MenuFn);
    vec![
        group("Your devices: overview", overview),
        group("Changes waiting for approval", |app| {
            app.sync_open_review_pub()
        }),
        group("Devices: add, remove, change what a device may do", devices),
        group("Phone: add the phone, hand over its changes", phone),
        group("Passwords: Vault or Quorum, kit, paper phrase", credentials),
        group("Recovery: a lost phone", recovery),
        group("Advanced: trust sessions, exemption, export", advanced),
    ]
}

fn devices(app: &mut App) {
    use super::sync_flows as f;
    let few = app
        .sync
        .cut_off_missing
        .map(|n| {
            format!(
                "you need {n} more computer{} you control to cut one off",
                if n == 1 { "" } else { "s" }
            )
        });
    let others = app.sync.devices.iter().filter(|d| !d.this).count();
    let none = (others == 0).then_some("there is no other device");
    open_group(
        app,
        "Devices",
        vec![
            unless("Remove a device (revoke)", f::revoke, none),
            unless(
                "Change what a device may do (full device or view-only copy)",
                f::role,
                none,
            ),
        ],
    );
    // The reason a removal would fail comes right after the menu opens, never after a typed name.
    if let Some(why) = few {
        app.host_notice = Some(format!("Removing a full device needs more computers: {why}."));
    }
}

fn phone(app: &mut App) {
    let has_phone = app.sync.devices.iter().any(|d| d.phone);
    open_group(
        app,
        "Phone",
        vec![
            unless(
                "Add the phone (enroll it from here)",
                super::sync_phone::start,
                has_phone.then_some("the ring already has a phone; replacing it is a recovery"),
            ),
            unless(
                "Take in what the phone signed (paste its text)",
                super::sync_phone::paste_ops,
                (!has_phone).then_some("the ring has no phone yet"),
            ),
            unless(
                "Show a restored phone's own changes (its history)",
                super::sync_phone::show_chain,
                (!has_phone).then_some("the ring has no phone yet"),
            ),
        ],
    );
}

pub(crate) fn credentials_group(app: &mut App) {
    credentials(app);
}

fn credentials(app: &mut App) {
    use super::sync_flows as f;
    let quorum = app.sync.cred_mode == Some(super::sync::CredModeView::Quorum);
    let switch = if quorum {
        "Switch back to Vault"
    } else {
        "Switch to Quorum"
    };
    open_group(
        app,
        "Passwords",
        vec![
            go(switch, f::mode),
            unless(
                "Import the Vault kit (second computer)",
                f::import_kit,
                quorum.then_some("only a Vault ring uses the kit"),
            ),
            go("Paper phrase: create, write down, type back", f::paper_wizard),
            go("Paper phrase: write a printable sheet", super::sync_flows_more::paper_sheet),
            go("Emailed code as the third approval: register", f::email_factor),
            go("Move this computer's saved passwords into the ring", |app| {
                f::migrate(app, false)
            }),
            go(
                "Remove the local copies once the ring's copies open",
                |app| f::migrate(app, true),
            ),
        ],
    );
}

fn recovery(app: &mut App) {
    let has_phone = app.sync.devices.iter().any(|d| d.phone);
    open_group(
        app,
        "Recovery",
        vec![unless(
            "Recovery of a lost phone",
            super::sync_flows::recovery,
            (!has_phone).then_some("the ring has no phone"),
        )],
    );
}

fn advanced(app: &mut App) {
    use super::sync_flows_more as m;
    let quorum = app.sync.cred_mode == Some(super::sync::CredModeView::Quorum);
    let has_phone = app.sync.devices.iter().any(|d| d.phone);
    let exempt = app.sync.devices.iter().any(|d| d.exempt);
    let no_phone = (!has_phone).then_some("the ring has no phone");
    open_group(
        app,
        "Advanced",
        vec![
            unless(
                "Trust session: let a computer open some hosts without asking",
                |app| app.sync_open_trust_pub(),
                no_phone,
            ),
            unless(
                "Trust session: allow more new hosts today",
                super::sync_trust::extend,
                no_phone,
            ),
            unless(
                "Trust session: end one",
                super::sync_trust::revoke,
                no_phone,
            ),
            unless(
                "Master phrase exemption: turn on for this computer",
                m::exemption_on,
                (!quorum).then_some("only a Quorum ring has it"),
            ),
            unless(
                "Master phrase exemption: turn off for a device",
                m::exemption_off,
                if !quorum {
                    Some("only a Quorum ring has it")
                } else if !exempt {
                    Some("no device has it")
                } else {
                    None
                },
            ),
            go("Export every saved password (a big change)", m::export_start),
        ],
    );
}

/// Who is in the ring and how it is set up, as a panel.
fn overview(app: &mut App) {
    let sync = &app.sync;
    let mut lines = vec![
        format!(
            "Passwords: {}",
            sync.cred_mode.map_or("unknown", |m| m.label())
        ),
        format!("Devices: {}", sync.devices.len()),
    ];
    for d in &sync.devices {
        let verified = match d.verified_by {
            Some(crate::sync::oplog::VerifiedBy::Sas) => "the four check words matched",
            Some(crate::sync::oplog::VerifiedBy::SshHostKey) => "you trusted its SSH host key",
            None => "",
        };
        lines.push(format!(
            "  {}{}: {}, {}{}{} {verified}",
            d.name,
            if d.this { " (this device)" } else { "" },
            d.role_word(),
            d.status.label(),
            if d.unattended { ", runs on its own" } else { "" },
            if d.exempt {
                ", opens passwords with the master phrase"
            } else {
                ""
            },
        ));
    }
    lines.push(format!("Changes waiting for approval: {}", sync.pending));
    app.sync_info("Your devices", lines);
}
