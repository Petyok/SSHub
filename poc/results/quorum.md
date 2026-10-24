# Stage 3 credential half (WP3a, WP3c): results

Branch `poc/quorum2` (from `poc/protocol`, with `poc/oplog` and `poc/tui` merged). Code:
`src/sync/{quorum,reshare,cred_ops,cert,modes,recovery}.rs`, plus the phone wire frames in
`wire.rs`, `share_release.rs` and `phone_fixtures.rs` taken over from the PWA worker. All numbers
below are from `cargo test --lib sync::` on a Linux x86_64 build container (debug profile, Argon2
at the cheap test profile where noted): 206 passed, 0 failed, 2 ignored; clippy `-D warnings` clean
for these files.

## P0 credential quorum

| Claim | Test | Result |
|---|---|---|
| A copy of one device (its key and the whole log, in a fresh process, no other share) cannot open a quorum credential through any path | `quorum_needs_two_shares_and_a_copy_of_one_device_has_one`, `p0_a_copy_of_one_device_opens_nothing_through_the_unlocker_and_one_more_share_opens_it` | own share only; the same share twice, a junk share, `shamir_combine` of one share, the vault path and the unlocker without a source (also with a ring secret) all fail |
| With a peer share, the phone share or the paper share it opens | same tests | peer reply sealed to the requester's ephemeral key opens it, a different key reads nothing; the paper unwrap identity derived from the phrase opens it |
| A copy of one vault machine opens nothing without both the ring secret and the phrase | `a_vault_credential_opens_only_with_the_phrase_and_the_ring_secret`, the P0 test | no ring secret: fails; ring secret without phrase: cancelled; phrase without ring secret: fails; both: opens. A stolen machine that also has the ring secret reduces to an offline search of the phrase, as designed |
| Resharing never calls `shamir_combine` | `resharing_never_reconstructs_and_the_new_sharing_opens_while_a_removed_holders_old_share_does_not` | per-thread call counter unchanged across plan, receive and confirm for 3 credentials and 3 new holders |
| The new sharing opens, the removed holder's old share does not combine with it | same test | two new shares open; old share plus a new one fails AEAD |
| Adoption is recorded only by a real unlock | same test | 3 credentials unadopted, 2 after one unlock under the new sharing |

## Other exit items

| Item | Test | Result |
|---|---|---|
| Recovery state machine, every state against every event | `recovery::tests::every_state_against_every_event` | 9 states x 14 events (126 cells), expected outcome written out per cell, all as expected |
| Replayed or second `PhoneReenroll` refused, backdated `initiated_at` refused, paper and approvals never both counted, clock starts at the earliest acknowledgement, a cancellation by a live phone needs no factor and a device's needs one | `recovery::tests::*` (5 more tests) | pass |
| Certificates enforced by the holders: wrong issuer key, wrong requester, out of scope, stale phone, expired, revoked, ring-wide budget across two holders, free re-open of an opened host, three-holder rule | `certificates_are_enforced_by_the_holders` | pass |
| Mode change Vault to Quorum re-encrypts under fresh DEKs; back to Vault needs a live co-signer other than the author | `a_mode_change_re_encrypts_under_new_deks_and_going_back_needs_a_cosigner` | pass |
| Migration reads keyring secrets and identity key files, replicates them as credentials, removes local copies only on request | `migration_reads_local_secrets_...` | pass |
| Identity key file materialised with mode 0600 and removed on drop; strict credential payload | `payload_is_strict_keys_materialise_...` | pass |
| `Unlocker` fills a `SecretGate` for a real host row; synced keys cannot be written through the local store | `the_unlocker_fills_the_gate_for_a_real_host_row_and_nothing_else` | pass |
| Challenge: binding over the whole op, wire round trip, fountain frames | `a_challenge_binds_the_whole_op_and_travels_as_frames` | pass |
| Ring secret sheet payload `sshub:ring:1:<32 hex>` parses exactly, typed groups accepted, other versions refused | `the_ring_secret_sheet_payload_parses_exactly` | pass |
| Phone frames: courier op list capped at 8 ops of 64 KiB, trailing bytes and oversize extension refused | `wire::phone_frame_tests`, `share_release::tests` | pass |

## Findings that changed the design while building

* A device that has not written anything is not in its own `sync_devices`; recipients and holder lists
  add the device itself (a fresh first device creating a credential would otherwise not get a share).
* `COMBINE_CALLS` must be per thread: tests run in parallel and a global counter saw other tests.
* One storage key per epoch per holder; one key per credential would leave all but one confirm unreadable.
* The vault decision follows the ring mode as well as the cached `vault_pub`, because that key is local:
  a fresh machine has none until it joins with the kit.

## Not done or not run

* `validate_phone_reenroll(store, op, now)` over the log (the adapter that feeds the state machine from
  ops) waits for the op-log additions agreed with Main: `RecoveryApprove`, `paper_sig` in
  `PhoneReenroll`, `DeviceKind`. The state machine is complete and tested without them.
* The interim read of the phone's id (`sync_meta` `phone_device`) is not written; nothing uses it.
* Argon2 at the real 1 GiB master profile was not run in these tests (cheap profile); the real profile
  is covered by the stage 0 vectors.
* The paper recipient takes no part in resharing (offline): after every holder adopted a reshare the paper
  opens only credentials of the sharing it was written for. Re-issuing paper needs a paper ceremony.
* Mode change does not erase history: old log entries stay openable by whoever held the old shape.
* No end-to-end run with the TUI worker or the phone page; no real SSH.
