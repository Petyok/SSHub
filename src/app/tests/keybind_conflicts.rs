use crate::app::util::parse_keyspec;
use crate::keybinds::{KeyAction, KeybindsConfig};
use crossterm::event::KeyCode;

/// The actions `handle_key_map` and `try_tab_switch` test on the map tab, in
/// no particular order. `BroadcastCancel` is left out on purpose: it is claimed
/// on every tab, but only while a broadcast is running, so sharing `x` with
/// `MapToggleTunnel` is not a dead binding.
const MAP_TAB_ACTIONS: &[KeyAction] = &[
    KeyAction::Quit,
    KeyAction::Help,
    KeyAction::Connect,
    KeyAction::Edit,
    KeyAction::Search,
    KeyAction::TagFilter,
    KeyAction::Cancel,
    KeyAction::Broadcast,
    KeyAction::GroupsManage,
    KeyAction::TogglePanelZoom,
    KeyAction::AddHost,
    KeyAction::KnownHosts,
    KeyAction::PushKey,
    KeyAction::TabHosts,
    KeyAction::TabSftp,
    KeyAction::TabTunnels,
    KeyAction::TabKeys,
    KeyAction::TabAudit,
    KeyAction::TabMap,
    KeyAction::MapUp,
    KeyAction::MapDown,
    KeyAction::MapLeft,
    KeyAction::MapRight,
    KeyAction::MapBlast,
    KeyAction::MapNextDependent,
    KeyAction::MapPrevDependent,
    KeyAction::MapTunnelLens,
    KeyAction::MapToggleTunnel,
    KeyAction::MapFold,
    KeyAction::MapFoldAll,
    KeyAction::MapUseAsJump,
    KeyAction::MapTest,
    KeyAction::MapSshLog,
    KeyAction::MapCopyCmd,
];

/// Same key as the matcher sees it: chars compare case-insensitively, the
/// modifier set exactly (a bare uppercase spec carries Shift).
fn normalized(spec: &str) -> (KeyCode, crossterm::event::KeyModifiers) {
    let (code, mods) = parse_keyspec(spec).unwrap_or_else(|| panic!("unparsable key {spec:?}"));
    match code {
        KeyCode::Char(c) => (KeyCode::Char(c.to_ascii_lowercase()), mods),
        other => (other, mods),
    }
}

#[test]
fn no_two_actions_live_on_the_map_tab_share_a_default_key() {
    let binds = KeybindsConfig::default();
    let mut seen: Vec<(KeyAction, (KeyCode, crossterm::event::KeyModifiers))> = Vec::new();
    for action in MAP_TAB_ACTIONS.iter().copied() {
        for spec in binds.binds(action) {
            let key = normalized(spec);
            if let Some((other, _)) = seen.iter().find(|(a, k)| *k == key && *a != action) {
                panic!("{action:?} and {other:?} both default to {spec:?} on the map tab");
            }
            seen.push((action, key));
        }
    }
}

#[test]
fn every_map_action_has_a_default_key_that_parses() {
    let binds = KeybindsConfig::default();
    for action in KeyAction::ALL
        .into_iter()
        .filter(|a| format!("{a:?}").starts_with("Map"))
    {
        let specs = binds.binds(action);
        assert!(!specs.is_empty(), "{action:?} has no default key");
        for spec in specs {
            assert!(parse_keyspec(spec).is_some(), "{action:?}: {spec:?}");
        }
    }
}

/// The actions the Sync tab handler (`handle_key_sync`) and `try_tab_switch`
/// test on the Sync tab.
const SYNC_TAB_ACTIONS: &[KeyAction] = &[
    KeyAction::Quit,
    KeyAction::Help,
    KeyAction::MoveUp,
    KeyAction::MoveDown,
    KeyAction::Cancel,
    KeyAction::TabHosts,
    KeyAction::TabSftp,
    KeyAction::TabTunnels,
    KeyAction::TabKeys,
    KeyAction::TabAudit,
    KeyAction::TabMap,
    KeyAction::TabSync,
    KeyAction::SyncNow,
    KeyAction::SyncReview,
    KeyAction::SyncTrust,
    KeyAction::SyncAdmin,
];

#[test]
fn no_two_actions_live_on_the_sync_tab_share_a_default_key() {
    let binds = KeybindsConfig::default();
    let mut seen: Vec<(KeyAction, (KeyCode, crossterm::event::KeyModifiers))> = Vec::new();
    for action in SYNC_TAB_ACTIONS.iter().copied() {
        for spec in binds.binds(action) {
            let key = normalized(spec);
            if let Some((other, _)) = seen.iter().find(|(a, k)| *k == key && *a != action) {
                panic!("{action:?} and {other:?} both default to {spec:?} on the sync tab");
            }
            seen.push((action, key));
        }
    }
}

/// The sync keys are claimed on the Sync tab (and `SyncEnroll` on the hosts
/// tab) only; every other action that defaults to the same key must live
/// where those tabs never see it (another tab, or a modal of its own).
#[test]
fn sync_keys_do_not_shadow_a_global_or_hosts_tab_default() {
    const ELSEWHERE: &[KeyAction] = &[
        KeyAction::SyncNow,
        KeyAction::SyncReview,
        KeyAction::SyncTrust,
        KeyAction::SyncAdmin,
        KeyAction::SyncEnroll,
        // Map tab and overlay-only actions.
        KeyAction::MapTest,
        KeyAction::MapCopyCmd,
        KeyAction::YankLog,
        KeyAction::ConfirmYes,
    ];
    let binds = KeybindsConfig::default();
    for sync in [
        KeyAction::TabSync,
        KeyAction::SyncNow,
        KeyAction::SyncReview,
        KeyAction::SyncTrust,
        KeyAction::SyncAdmin,
        KeyAction::SyncEnroll,
    ] {
        for spec in binds.binds(sync) {
            let key = normalized(spec);
            for other in KeyAction::ALL {
                if other == sync || (other != KeyAction::TabSync && ELSEWHERE.contains(&other)) {
                    continue;
                }
                assert!(
                    !binds
                        .binds(other)
                        .iter()
                        .any(|s| parse_keyspec(s).is_some() && normalized(s) == key),
                    "{sync:?} default {spec:?} is also the default of {other:?}"
                );
            }
        }
    }
}
