---
type: Workflow
title: Broadcast - run one command across a group or tag
description: The hosts-tab broadcast wizard (pick a group or tag, enter a command, preview and edit targets) and the background run in src/broadcast with a bounded ssh worker pool, askpass for stored secrets, per-host timeouts, the docked live panel with error toasts and auto-dismiss, cancel, and one audit row per host.
resource: src/broadcast/mod.rs
tags: [broadcast, ssh, fan-out, workflow, tui]
---

# Broadcast

Broadcast runs one non-interactive command on every selected managed host of a group or tag, concurrently, and shows the results in a panel docked on the hosts tab. The pure engine is `src/broadcast/mod.rs` (pool, reducer, view helpers, `CommandRunner` seam); the App glue is `src/app/broadcast.rs`; the panel and wizard render from `src/tui/screens/broadcast.rs`.

## Wizard

`b` (`KeyAction::Broadcast`) on the hosts tab calls `open_broadcast`. It refuses while a run is still in progress ("A broadcast run is already in progress."); a finished panel is slid out and replaced. With no groups and no tags there is nothing to target.

1. **Pick target** (`AppMode::BroadcastPickTarget`): every user group (the reserved Favorites group is not offered) followed by every distinct host tag, sorted. `j`/`k` move, Enter resolves the target, Esc closes.
2. **Command** (`AppMode::BroadcastCommand`): one line of input with cursor keys. Enter on a non-empty command continues; Esc goes back to the target list.
3. **Preview** (`AppMode::BroadcastPreview`): the candidate list is the managed hosts in host-list order that belong to the group or carry the tag (ssh_config-only hosts are never targeted), all selected. `y` starts, `e` enters target editing (`j`/`k` move, Space toggles a host, Enter/`e`/Esc finish), `c` returns to the command with targets kept, `n`/`N`/Esc cancel. Starting with nothing selected shows "No hosts selected for broadcast."

Each candidate's argv and stored secret are resolved when the target is picked, through the same effective-connection path as a normal connect (`resolve_connection`, `resolved_session_argv`, `resolve_pending_secret`), so group defaults apply.

## Run

`spawn_broadcast` starts `min(DEFAULT_CONCURRENCY = 8, hosts)` worker threads that pull tasks from a shared channel and report `BroadcastEvent`s; the UI folds them each tick (`tick_broadcast`, `apply_event`).

`SshCommandRunner` runs `ssh -o BatchMode=… -o ConnectTimeout=8 -o StrictHostKeyChecking=accept-new <argv…> <command>` with stdin closed and both pipes drained on threads:

- a host with a stored secret runs `BatchMode=no` and gets the secret through the file-based `SSH_ASKPASS` helper (`AskpassSecret`);
- a host without one runs `BatchMode=yes`, so a password prompt fails fast instead of reaching `/dev/tty`;
- each run is killed after `RUN_TIMEOUT` (60 s) and reported as "timed out".

## Live panel

The docked panel slides from the center to the bottom-right of the hosts body and lists each host with its state; it is not focused automatically. Zoomed (`z`), it becomes a failures-first host list with the selected host's stdout and stderr below (`render_broadcast_zoomed`); `j`/`k` and the wheel move through it.

- Every failed host (non-zero exit or a runner failure) raises an error toast with its stderr for 10 s (`TOAST_TTL`).
- `x` (`KeyAction::BroadcastCancel`) cancels a run in progress: in-flight children are killed and the remaining hosts end as cancelled. With nothing running, `x` clears the error toasts.
- `Alt+Arrows` can move focus onto the panel (only while it exists) and `z` or `Alt+Enter` zooms it. Focus pauses the dismiss countdown.
- Once every host is finished, the panel auto-dismisses after 6.5 s (`DISMISS`) unless focused, sliding out to the right.

## Audit

When every host reaches a terminal state, one `auth_events` row per host is written with `via = broadcast`: status `launched` for exit 0, `fail` otherwise, and a note holding the command, the exit code and the first stderr line (or the failure reason). Unlike `sshub exec`, the broadcast command text is stored in the audit note. The rows appear in the Audit tab and in `sshub audit list` (the `--via` filter has no `broadcast` value, so use `--via all`).

A live or unfinished broadcast blocks switching profiles (`profile_switch_blocker`).

## Change guidance

- Engine behavior belongs in `src/broadcast/mod.rs` and is tested there through the `CommandRunner` seam; App flow tests are in `src/app/tests/broadcast.rs`, end-to-end in `tests/e2e/broadcast.rs`.
- The `SshCommandRunner` doc comment still says there is no `SSH_ASKPASS` in v1; the code below it does stage askpass for stored secrets.
