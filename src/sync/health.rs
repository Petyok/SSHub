//! What a person should know about the ring without opening the Sync tab: the one source for the status bar,
//! the Sync tab header and the line `sshub host list` prints on stderr.

use anyhow::Result;

use super::limits::{PHONE_FRESH_SECS, PHONE_FRESH_WARN_SECS, RED_CEREMONY_TTL_SECS};
use super::oplog::DeviceKind;
use crate::store::LauncherStore;

/// Facts about the ring at one moment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Health {
    /// Seconds until the soonest big change that still waits for approvals is dropped.
    pub red_expires_in: Option<i64>,
    /// Days since the phone last signed anything a computer here saw, when a trust session needs it.
    pub phone_unseen_days: Option<i64>,
    /// True when that is past the warning point (7 days) and a trust session is live.
    pub phone_warn: bool,
    /// True when the phone is past the point where trust sessions stop working (14 days).
    pub phone_expired: bool,
    /// Days since this computer last reached any other device, when other devices exist.
    pub offline_days: Option<i64>,
    /// Changes waiting for approval.
    pub waiting: usize,
}

/// Whole days, rounded down.
fn days(secs: i64) -> i64 {
    secs.max(0) / 86_400
}

/// Seconds until the soonest unfinished big change expires, if any.
pub fn red_expires_in(store: &LauncherStore, now: i64) -> Result<Option<i64>> {
    let mut soonest: Option<i64> = None;
    for op in store.sync_pending()? {
        if !super::ceremony::is_gated(store, &op)? {
            continue;
        }
        let st = super::ceremony::red_status(store, &op, now)?;
        if st.expired || st.complete {
            continue;
        }
        let left = RED_CEREMONY_TTL_SECS - (now - super::ceremony::base_time(store, &op));
        soonest = Some(soonest.map_or(left, |s| s.min(left)));
    }
    Ok(soonest)
}

fn live_trust_session(store: &LauncherStore, now: i64) -> Result<bool> {
    for op in super::quorum::ops_of(store, "CertIssue", None)? {
        if let Ok(cert) = super::cert::DelegationCert::from_op(&op) {
            if cert.expiry > now && !store.sync_cert_revoked(&cert.id)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// The ring's health as seen from this device. `me` is this device's id, when known.
pub fn health(store: &LauncherStore, me: Option<&str>, now: i64) -> Result<Health> {
    let mut h = Health {
        waiting: store.sync_pending()?.len(),
        red_expires_in: red_expires_in(store, now)?,
        ..Health::default()
    };
    let devices = super::ceremony::effective_devices(store, None)?;
    let others: Vec<_> = devices
        .iter()
        .filter(|d| Some(d.id.as_str()) != me && d.kind != DeviceKind::Phone)
        .filter(|d| {
            !matches!(
                d.status,
                super::oplog::DeviceStatus::Revoked | super::oplog::DeviceStatus::Compromised
            )
        })
        .collect();
    if me.is_some() && !others.is_empty() {
        // A device that never answered has last_seen 0: say nothing rather than "offline 20000d".
        if let Some(newest) = others.iter().map(|d| d.last_seen).filter(|s| *s > 0).max() {
            let gap = now - newest;
            if gap >= 86_400 {
                h.offline_days = Some(days(gap));
            }
        }
    }
    if let Some(phone) = super::quorum::phone_device(store)? {
        if live_trust_session(store, now)? {
            let age = now - super::quorum::phone_signed_at(store, &phone, now)?;
            h.phone_unseen_days = Some(days(age));
            h.phone_warn = age >= PHONE_FRESH_WARN_SECS;
            h.phone_expired = age > PHONE_FRESH_SECS;
        }
    }
    Ok(h)
}

/// "20 h" or "2 days": a duration a person can read, never below one hour.
pub fn human_left(secs: i64) -> String {
    let h = (secs.max(0) + 3_599) / 3_600;
    if h >= 48 {
        format!("{} days", h / 24)
    } else {
        format!("{h} h")
    }
}

/// The one line for the status bar, most urgent first; `None` when nothing needs a look.
pub fn status_line(h: &Health) -> Option<String> {
    if let Some(left) = h.red_expires_in {
        if left <= 86_400 {
            return Some(format!("Red change expires in {}", human_left(left)));
        }
    }
    if h.phone_expired {
        return Some("phone not seen for 14 days: trust sessions stopped".into());
    }
    if h.phone_warn {
        return Some(format!(
            "phone not seen for {} days: trust sessions stop at 14",
            h.phone_unseen_days.unwrap_or(7)
        ));
    }
    if let Some(d) = h.offline_days {
        return Some(format!("sync: offline {d}d"));
    }
    if h.waiting > 0 {
        return Some(format!("sync: {} waiting", h.waiting));
    }
    None
}

/// The line `sshub host list` prints on stderr when a big change expires within 24 h.
pub fn cli_line(store: &LauncherStore, now: i64) -> Option<String> {
    let left = red_expires_in(store, now).ok()??;
    (left <= 86_400).then(|| {
        format!(
            "sshub: a big sync change expires in {}: approve it or propose it again (sshub sync-review)",
            human_left(left)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_line_picks_the_most_urgent_fact() {
        let mut h = Health {
            waiting: 2,
            ..Health::default()
        };
        assert_eq!(status_line(&h).as_deref(), Some("sync: 2 waiting"));
        h.offline_days = Some(6);
        assert_eq!(status_line(&h).as_deref(), Some("sync: offline 6d"));
        h.phone_warn = true;
        h.phone_unseen_days = Some(8);
        assert!(status_line(&h).unwrap().starts_with("phone not seen for 8"));
        h.red_expires_in = Some(20 * 3_600);
        assert_eq!(
            status_line(&h).as_deref(),
            Some("Red change expires in 20 h")
        );
        h.red_expires_in = Some(40 * 3_600);
        assert!(status_line(&h).unwrap().starts_with("phone not seen"));
        assert_eq!(status_line(&Health::default()), None);
    }

    #[test]
    fn durations_are_read_in_hours_then_days() {
        assert_eq!(human_left(60), "1 h");
        assert_eq!(human_left(20 * 3_600), "20 h");
        assert_eq!(human_left(3 * 86_400), "3 days");
    }
}
