# Stage 3.5 Hub, Rust side: results

Branch `poc/quorum2` (merged with `poc/hub`: `src/sync/notify.rs`, `[sync.notify]`, `deploy/hub/`).
Code: `src/sync/hub.rs` and `quorum::release_share_for`, `recovery::Event::Ack { attended }`.
`cargo test --lib sync::` on the Linux x86_64 build container: 230 passed, 0 failed, 2 ignored;
clippy `-D warnings` clean.

## Decision: there is no hub daemon and no `hub-serve` verb

A Hub is an ordinary ring device (a mirror, unattended) behind the sshd of `deploy/hub/`; terminals reach
it with `ssh hub sshub sync-serve`, the same command as any peer. `deploy/hub/README.md` already says the
image "does not serve the mirror yet because `hub-serve` is not in this build": that sentence is to be
replaced by "sync-serve over ssh is the mirror", and `main.rs` needs no new verb. Enrollment of a headless
Hub is done from the terminal side in one interactive session (`ssh -t hub sshub sync-enroll`): the same
person sees both SAS screens.

## What makes it a Hub: the boundaries, where they are enforced

| Boundary | Enforced in | Test |
|---|---|---|
| never votes: approvals of a recovery count only from attended devices | `recovery::Event::Approve { attended }` | recovery table (126 cells, now 135 with the unattended acknowledgement) |
| never starts the recovery clock: an acknowledgement counts only from an attended device (an unattended device has seen nothing) | `recovery::Event::Ack { attended }` | table column `ack-unattended` |
| a mirror holds no shares | `share_recipients` takes holders only | quorum tests |
| an unattended holder releases a share only under a valid trust certificate; on approval alone it refuses; under a certificate the first release of a host is recorded (`CertHostOpened`) once; another requester or another host is refused | `quorum::release_share_for` + `cert_allows` | `hub::tests::an_unattended_holder_releases_only_under_a_valid_certificate` |
| announces recovery events out of band, at most once per op, never with a code, marked only after the send succeeded, a failing mail server blocks nobody | `hub::announce_recovery_ops`, `hub::run_sync_serve_notify` | `hub::tests::recovery_ops_from_others_are_announced_once_...` |
| relays phone courier ops without being able to alter them (signatures are the phone's), strict decode, at most 8 ops | `hub::courier_ingest` | `hub::tests::courier_ops_are_ingested_strictly_...` |
| never signs checkpoints | NOT in this package: checkpoint ops belong to the op-log; `quorum::is_unattended(store, me)` is the check to call there | not run |

## Notification hook

`hub::run_sync_serve_notify(store, id, link, Option<&NotifyConfig>)` runs `run_sync_serve`, then, when
`[sync.notify].command` is set, announces `PhoneReenroll`, `RecoveryApprove` and `PhoneReenrollCancel` ops
this device received from others (`local = 0`, not rejected) and has not announced. `RecoveryAck` is the
automatic "seen" record and is not announced. The body names the author device, the 8 hex of the op hash,
the receive time in UTC and what to do; it never contains a code and says so. Failures go to stderr (stdout
carries frames) and are retried at the next round. The TUI worker wires `sync-serve` in `src/main.rs` to
call it with `config.sync.notify`.

## Not run

* No real mail command (`send_notification` is covered by the fast worker's tests; here the sender is a closure).
* No end-to-end with sshd and the container.
* `RecoveryApprove` has no op variant yet: announcements for it use the generic text until the op-log adds it.
