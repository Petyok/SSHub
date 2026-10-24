//! The Sync tab's admin menu: one entry per operation that changes the ring.
//! Each flow lives in its own module and is listed here.

use super::sync_ui::MenuFn;
use super::*;

/// The menu entries, in order.
pub(crate) fn entries() -> Vec<(String, MenuFn)> {
    vec![("Ring overview".to_string(), Box::new(overview) as MenuFn)]
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
