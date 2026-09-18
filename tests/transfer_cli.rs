//! Behavioral tests for cross-profile transfer CLI surface.
//!
//! These exercise the pure plan/args layer (`sshub::cli::transfer`) against
//! the engine contract (`sshub::store::transfer`): confirmation is always
//! explicit, and `group transfer --with-hosts` toggles host expansion.

use sshub::cli::transfer::{needs_confirm, parse_transfer_args, render_plan_text};
use sshub::store::transfer::{PlannedItem, TransferPlan};

fn sample_plan() -> TransferPlan {
    TransferPlan {
        items: vec![
            PlannedItem {
                kind: "host",
                src_name: "web".to_string(),
                dest_name: "web".to_string(),
                renamed: false,
                blocked: None,
            },
            PlannedItem {
                kind: "host",
                src_name: "db".to_string(),
                dest_name: "db (copy 1)".to_string(),
                renamed: true,
                blocked: None,
            },
            PlannedItem {
                kind: "host",
                src_name: "live".to_string(),
                dest_name: "live".to_string(),
                renamed: false,
                blocked: Some("tunnel running".to_string()),
            },
        ],
        dest_profile: "other".to_string(),
    }
}

#[test]
fn transfer_cli_plan_confirm_requires_explicit_yes() {
    // A runnable plan must demand explicit confirmation (no silent apply).
    assert!(needs_confirm(&sample_plan()));
    // Even an empty plan demands it: the confirm gate is unconditional.
    let empty = TransferPlan {
        items: Vec::new(),
        dest_profile: "other".to_string(),
    };
    assert!(needs_confirm(&empty));
    // A plan with only blocked items demands it too: the gate does not depend
    // on runnability. NOTE: this pins the predicate only — the `cancelled` /
    // exit-1 behavior of `run_transfer` has no unit oracle (it needs live
    // profile stores + interactive stdin; production is frozen).
    let blocked_only = TransferPlan {
        items: vec![PlannedItem {
            kind: "host",
            src_name: "live".to_string(),
            dest_name: "live".to_string(),
            renamed: false,
            blocked: Some("tunnel running".to_string()),
        }],
        dest_profile: "other".to_string(),
    };
    assert!(needs_confirm(&blocked_only));
}

#[test]
fn transfer_cli_group_with_hosts_flag() {
    let with = parse_transfer_args(&[
        "--to".to_string(),
        "other".to_string(),
        "--with-hosts".to_string(),
    ])
    .expect("--with-hosts must parse");
    assert!(with.with_hosts);
    assert_eq!(with.dest_profile, "other");
    assert!(!with.move_mode);
    assert!(!with.auto_yes);

    let without = parse_transfer_args(&["--to".to_string(), "other".to_string()])
        .expect("minimal transfer args must parse");
    assert!(!without.with_hosts);
    // Full flag set: copy flips to move, prompt flips to auto-yes.
    let full = parse_transfer_args(&[
        "--to".to_string(),
        "other".to_string(),
        "--move".to_string(),
        "--yes".to_string(),
    ])
    .expect("full transfer args must parse");
    assert!(full.move_mode);
    assert!(full.auto_yes);
    assert!(!full.with_hosts);
    assert_eq!(full.dest_profile, "other");

    // Destination profile is required.
    assert!(parse_transfer_args(&["--with-hosts".to_string()]).is_err());
}

#[test]
fn transfer_cli_plan_render_marks_renames_and_blocks() {
    let text = render_plan_text(&sample_plan());
    // Golden text: the render is deterministic, so pin it byte-for-byte.
    let expected = "Transfer to profile 'other': 3 item(s)\n  host 'web' -> 'web'\n  host 'db' -> 'db (copy 1)' (renamed)\n  host 'live' blocked: tunnel running\n";
    assert_eq!(text, expected, "plan render golden mismatch:\n{text}");
}
