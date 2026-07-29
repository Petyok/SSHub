use crate::store::{LauncherStore, Tunnel};

use super::ReconnectEvent;

/// Log tunnel reconnect lifecycle events to the auth audit table.
pub fn log_tunnel_reconnect_events(
    store: &LauncherStore,
    events: &[ReconnectEvent],
    tunnels: &[Tunnel],
) {
    for ev in events {
        let tunnel = tunnels.iter().find(|t| t.id == ev.tunnel_id());
        let (host_name, port, label) = tunnel
            .map(|t| {
                let name = t
                    .host_id
                    .and_then(|hid| store.get_host(hid).ok().flatten())
                    .map(|h| h.name)
                    .unwrap_or_else(|| "unknown".into());
                (name, t.local_port, t.label.clone().unwrap_or_default())
            })
            .unwrap_or_else(|| ("unknown".into(), 0, String::new()));

        match ev {
            ReconnectEvent::Attempt { attempt, .. } => {
                let _ = store.log_auth_event(
                    &host_name,
                    None,
                    "tunnel",
                    "retry",
                    &format!("tunnel reconnecting :{port} {label} attempt {attempt}"),
                    None,
                );
            }
            ReconnectEvent::Reconnected { .. } => {
                let _ = store.log_auth_event(
                    &host_name,
                    None,
                    "tunnel",
                    "launched",
                    &format!("tunnel reconnected :{port} {label}"),
                    None,
                );
            }
            ReconnectEvent::GaveUp {
                attempts, error, ..
            } => {
                let _ = store.log_auth_event(
                    &host_name,
                    None,
                    "tunnel",
                    "fail",
                    &format!("tunnel gave up :{port} {label} after {attempts} attempts — {error}"),
                    None,
                );
            }
        }
    }
}
