//! The Sync tab's admin menu: one entry per operation that changes the ring.
//! Each flow lives in its own module and is listed here.

use super::sync_ui::MenuFn;
use super::*;

/// The menu entries, in order.
pub(crate) fn entries() -> Vec<(String, MenuFn)> {
    use super::sync_flows as f;
    use super::sync_flows_more as m;
    let item = |label: &str, run: fn(&mut App)| (label.to_string(), Box::new(run) as MenuFn);
    vec![
        item("Ring overview", overview),
        item("Review changes waiting for approval", |app| {
            app.sync_open_review_pub()
        }),
        item("Trust session: start one (the phone approves)", |app| {
            app.sync_open_trust_pub()
        }),
        item(
            "Trust session: extend today's budget",
            super::sync_trust::extend,
        ),
        item("Trust session: revoke one", super::sync_trust::revoke),
        item("Revoke a device", f::revoke),
        item("Change a device's role (holder or mirror)", f::role),
        item("Switch the credential mode (Vault, Quorum)", f::mode),
        item(
            "Add my phone (enroll it from here)",
            super::sync_phone::start,
        ),
        item("Paste phone ops", super::sync_phone::paste_ops),
        item("Recovery of a lost phone", f::recovery),
        item("Paper phrase: create and show", f::paper_setup),
        item(
            "Paper phrase: register what I wrote down",
            f::paper_register,
        ),
        item("Paper phrase: write a printable sheet", m::paper_sheet),
        item("Emailed code as third factor: register", f::email_factor),
        item("Move this machine's saved secrets into the ring", |app| {
            f::migrate(app, false)
        }),
        item(
            "Remove the local copies once the ring's copies open",
            |app| f::migrate(app, true),
        ),
        item("Import the Vault kit (second machine)", f::import_kit),
        item(
            "Master-phrase exemption: enable on this machine",
            m::exemption_on,
        ),
        item(
            "Master-phrase exemption: switch off for a device",
            m::exemption_off,
        ),
        item("Export every credential (bulk export)", m::export_start),
        item("Export: continue", m::export_continue),
    ]
}

/// Who is in the ring and how it is set up, as a panel.
fn overview(app: &mut App) {
    let sync = &app.sync;
    let mut lines = vec![
        format!(
            "Credential mode: {}",
            sync.cred_mode.map_or("unknown", |m| m.label())
        ),
        format!("Devices: {}", sync.devices.len()),
    ];
    for d in &sync.devices {
        let role = match d.role {
            super::sync::DeviceRoleView::Holder => "holder",
            super::sync::DeviceRoleView::Mirror => "mirror",
        };
        let verified = match d.verified_by {
            Some(crate::sync::oplog::VerifiedBy::Sas) => "verified by SAS",
            Some(crate::sync::oplog::VerifiedBy::SshHostKey) => "verified by SSH host key",
            None => "",
        };
        lines.push(format!(
            "  {}{}: {role}, {}{}{} {verified}",
            d.name,
            if d.this { " (this device)" } else { "" },
            d.status.label(),
            if d.unattended { ", unattended" } else { "" },
            if d.exempt {
                ", exemption: master phrase"
            } else {
                ""
            },
        ));
    }
    lines.push(format!("Changes waiting for review: {}", sync.pending));
    app.sync_info("Ring overview", lines);
}
