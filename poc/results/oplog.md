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

- The long property runs predate the last small delta (ShareGrant, ShareRelease key, `verified_by`); the default-seed tests ran after it.
- No NixOS/macOS run; no real keyring in the identity-store tests (in-memory credential store).
- Reshare/credential effects beyond table bookkeeping, recovery state machine and release flow are stage 3.
- Voiding does not undo effects of non-entity ops (certs, mode, devices); documented in contract 2a.
