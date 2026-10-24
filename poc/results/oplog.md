# Op-log, schema v22, checkpoints (WP1a, WP1b, C15): results

VERDICT PASS (with the caveats under "Not run")

Branch `poc/oplog` (local, no upstream). Logs are under `/home/petruha/sshub-dev/gpu-verify/logs/`.

## Claims

| Claim | Criterion | Measured | Unit | Result |
|---|---|---|---|---|
| Two stores converge on random op sequences in random orders (rename vs delete, same-name create, resurrection, duplicates) | P0 stable model | 3000 seeds x 30 steps, 0 divergent logical state hashes, 0 divergent projection hashes, 0 dangling FKs (hs-oplog-m2-20261024T161554.log) | seeds | PASS |
| Three stores, same | P0 stable model | 1500 seeds x 40 steps, 0 divergences (hs-oplog-m3-20261024T161554.log) | seeds | PASS |
| Adversarial inputs: replay, forged signature, fork proof (two ops, signed head vs chain, two heads, late chain), lamport jump over cap, revoke vs concurrent edit in both arrival orders, quarantine overflow, dangerous field hidden among safe ones, unknown device, gap, bent prev_hash, malformed fields | P0 op-log | 46 `sync::oplog` tests pass (full run hs-oplog-20261024T164932.log) | tests | PASS |
| Canonical encoder equals the committed vectors byte for byte, independently reproduced | vectors | 41 vectors (all 27 body variants + edge cases), Python oracle 129 checks 0 failures incl. self-test; Rust test compares the committed file | checks | PASS |
| Migration v21 -> v22 | migration | uid backfill unique, well-known uids, memberships and tunnels unchanged, v21 backup written, rerunnable, no FKs on sync tables | tests | PASS |
| Same cut folds to the same state on every store | C15 | 150 seeds (2 and 3 stores), 0 differences (hs-oplog-mc-20261024T161554.log) | seeds | PASS |
| Lying proposer is disputed, proposer Suspect never Compromised | C15 | `a_lying_proposer_is_disputed_...` | test | PASS |
| Prune then bootstrap equals full history | C15 | 15 seeds of pruned store, bootstrapped store and full store folding to identical bytes, equal state and projection hashes | seeds | PASS |
| Fork at the floor seq still proven after pruning; older ops covered; tail below floor is `Pruned` | C15 | test | test | PASS |
| Pruning removes superseded wrapped shares (byte scan of the DB file, raw and base64) | C15 | superseded op and resharded shares gone, live ones kept | test | PASS |
| Stale-at-checkpoint device's later ops go to pending | C15 | test | test | PASS |
| Full verify.sh | CI mirror | fmt, clippy, build PASS; test 1853 pass, 1 fail = `watcher::tests::debounce_coalesces_rapid_writes` (known flake, passes alone 14/14, hs-oplog-20261024T165913.log) | | PASS (flake) |

## Commands

- `/home/petruha/sshub-dev/gpu-verify/verify.sh <worktree> hs-oplog` (full)
- `tools.sh <worktree> hs-oplog-m2 -- 'SSHUB_PROP_SEEDS=3000 cargo test --lib --no-default-features sync::oplog::tests::two_stores'` (and `three_stores` with 1500, `the_same_cut` with 150, `prune_then` with 15)
- `tools.sh <worktree> hs-oplog-vec -- 'python3 tests/oracles/op_canonical.py'`
- Regenerate vectors: `SSHUB_EMIT_VECTORS=1 cargo test sync::oplog::vectors`

## Bugs the long runs found (fixed, regression tests kept)

Delete-before-rename ordering of rows; closure of name peers' referrers; rows of entities that lost their name to a void; incomplete entities removed before renames; tombstone `ts` not recorded.

## Not run

- No NixOS/macOS run; no real keyring in the identity-store tests (in-memory credential store).
- Reshare/credential effects beyond table bookkeeping, recovery state machine and release flow are stage 3.
- Voiding does not undo effects of non-entity ops (certs, mode, devices); documented in contract 2a.

## Update after the follow-ups

- Long reruns on tip 0658917 (after the last delta): 3000 two-store seeds PASS (1034 s, hs-oplog-f2-20261024T170106.log), 1500 three-store seeds PASS (1286 s, hs-oplog-f3-...), 150 same-cut seeds PASS and 15 prune/bootstrap seeds PASS (hs-oplog-fc-...).
- base64: canonical.rs now uses the `base64` crate (STANDARD engine: padded, canonical, no whitespace); vectors unchanged, Python oracle still passes.
- Red ceremony ops (RedApprove, RedPhoneSign, RedFactor Green and inert; FactorRegister, DeviceRoleChange, BulkExport Red), `DeviceRoleChange` effect on `sync_devices.role` and the checkpoint fold, DeviceRevoke tier by target role (`merge::effective_tier`: mirror Yellow, holder or unknown Red). 48 vectors, oracle 150 checks 0 failures. Full verify PASS on the tip (a first attempt failed only on a transient ssh denial to gpu-serv).
- Not done: C6 checkpoint wire (Pruned / WantCheckpoint / CheckpointChunk) and the propose/sign/prune driver; the store API they need already exists (`sync_checkpoint_*`, `sync_prune`, `sync_bootstrap`, `Pruned` error).

## C6 checkpoint wire, driver and replay on adoption (tip 8d56e61)

| Claim | Measured | Result |
|---|---|---|
| Fresh store bootstraps over a pipe pair from a pruned peer, same state and projection hash as a full-history store | test `a_fresh_store_bootstraps_over_the_wire_from_a_pruned_peer` | PASS |
| Peer behind past the floor gets `Pruned` and adopts the checkpoint | test `a_peer_behind_past_the_floor_...`, same state hash as the peer | PASS |
| Offline device with 3 unsynced own ops adopts and keeps them on top, unchanged, ends with the same state hash as peers that received them normally | test `an_offline_device_adopts_...` | PASS |
| Own chain that diverges from the cut is refused naming the seq; store unchanged | test `an_own_chain_that_diverges_...` | PASS |
| Lying peer's chunk fails verification, store untouched | test `a_lying_peers_checkpoint_chunk_...` | PASS |
| Scheduling: empty log, ops/bytes growth, open proposal and minimum interval, finished reshare, unattended never proposes or signs | 5 unit tests in `checkpoint_driver` | PASS |
| Long property runs on the final tip | 3000 two-store seeds (999 s, hs-oplog-g2-20261024T181356.log), 1500 three-store seeds (1242 s, ...-g3-...), 150 same-cut seeds and 15 prune/bootstrap seeds (...-gc-...): 0 divergences | PASS |
| Full verify.sh on the tip | fmt, clippy, build, test PASS (hs-oplog-20261024T183955.log) | PASS |

Replay rule as built: local ops (`local = 1`) above this device's cut seq must chain from the cut's head entry for this device (seq and `prev_hash`); they are re-ingested unchanged after the bootstrap and kept as own, approved ops. Foreign unsynced ops are not replayed (the next tail sync fetches them). An own op at the cut seq with another hash, or a gap or wrong `prev_hash` above it, refuses the adoption.

## Ops for S (ExemptionSet, CertExtend) and bulk export (C9)

| Claim | Measured | Result |
|---|---|---|
| `ExemptionSet { device_id, slot }` (Red): canonical bytes, vectors, oracle; projection sets `sync_devices.exempt` (newest op wins, also through checkpoint fold and bootstrap); invalid slot key refused | test `exemption_set_projects_the_flag_and_the_newest_op_wins` | PASS |
| `CertExtend { cert_id, extra_hosts }` (Green, inert), 1..=`CERT_NEW_HOSTS_PER_DAY` | test `cert_extend_is_green_inert_and_bounded` | PASS |
| Vectors / oracle for every body | 51 vectors, 35 body variants, oracle 159 checks 0 failures | PASS |
| Bulk export: window check (before, inside, after, other device, mid-export expiry), 0600 file in the importer's `L00t.csv` format (commas, quotes, newlines in passwords survive `parse_loot_csv`), never overwrites, window over 3600 s refused, announced once per op through the notify hook, failing mail retried | 4 tests in `sync::export` | PASS |
| C2 revocation driver | not built here: it exists as `src/sync/revocation.rs` on poc/quorum2 (S) | n/a |

## Join anchor, unknown-author review, CLI admin verbs

- `sync_join_anchor_build(id)`: the inviter's resolved state at its chain heads, a detached
  proposal plus one sign by the inviter alone (not entered in any log, `prev_checkpoint` is the
  sentinel `JOIN_ANCHOR_PREV`). `sync_join_adopt(bundle, BootstrapTrust::Enrollment{inviter})`
  installs it as the starting state; it is not recorded as a checkpoint and `sync_bootstrap`
  with the other trusts, `record` and `verify` refuse it.
- Unknown author: `ingest` stores such an op as `pending` with reason `unknown-author` (no chain
  head, no fold, never served, no fork evidence, kind marker `UnknownAuthor`); caps 256 overall.
  Approve is refused until the author is a device, then the op goes through the ordinary ingest;
  reject forgets it. `sync_pending_with_reasons()` lists every pending op with its reason.
- Tests: 502-host joiner has 0 pending; later dangerous change pending `tier`; stranger op
  pending `unknown-author`. Full verify PASS (2114 tests).
- CLI: `src/cli/sync_admin.rs` (11 verbs), smoke tests in `tests/smoke/cli_commands.rs`.
