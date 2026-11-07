# Host-sync e2e findings

Run `e2e-stage2e` on `poc/e2e` HEAD (after the `poc/tui` merge), log
`gpu-verify/logs/e2e-stage2e-20261024T184349.log`.

| Scenario | Result |
|---|---|
| 00-smoke | PASS |
| 10-enroll-and-sync | FAIL (5 failures) |
| 20-transitive-via-hub | FAIL (1 failure) |
| 30-offline-and-hostile | PASS |
| 40-tamper | FAIL (2 failures) |

Failing assertions are kept as they are. Each finding below is a product
contradiction or a test-side bug, with the exact output observed.

## F1. Held host change never reaches the review queue (product, 10)

Command on peer-b after `peer-a host add --name web1 --address web1.internal`
and `sync-now`:

```
$ sshub sync-review --format json
[
  {"author": "hub",    "changes": [], "id": "66ca61ec842d", "summary": "add device peer-b [tier]"},
  {"author": "peer-a", "changes": [], "id": "82ee31c27f1f", "summary": "add device hub [tier]"}
]
```

Only the two device-admission items are queued. No entry mentions web1 or
`changes[].to == "web1.internal"`. `sync-status` still reported `pending >= 1`
at that point.

Consequence: `rid web1.internal` is empty, and `sync-review --approve ""` fails:

```
sshub: --approve requires an op id
       run `sshub sync-review --help` for usage
```

Expected: the address change is a Dangerous field (`tier.rs`) and is listed
with `changes[].field == "address"`, `to == "web1.internal"`, `dangerous: true`.
Actual: it is held (address not applied) but not listed. Whether the op is
pending but unrendered, or was never queued, is not determined.

Same pattern for web2 (`reject held web2 change`).

## F2. Ring device with no recorded host is omitted from `sync-now` (product, 20)

Ring: peer-a founded via hub, peer-c joined via hub. peer-c has no host
recorded for peer-a. peer-c `sync-now --format json` output:

```
[{"applied":0,"behind":[],"device":"a235ac22e39c2b14","duplicates":0,"forks":[],"name":"hub","pending":2,"received":4,"rejected":2,"sent":0,"status":"ok","suspect":[]}]
```

No peer-a row at all. `peers.json` on peer-c contains only `"hub"`.

Contract/help: `src/cli/help.rs:331-332` says the other devices are listed as
`skipped (no host known)`. The code emits `skipped` only in
`Outcome::Skipped` (`src/cli/sync_cmd.rs:435-438`), so a device absent from
the ring-to-host map is dropped instead of reported.

Expected per help text: a `peer-a` row with `status == "skipped"`.

## F3. Tamper step fails: test-side bug (40), now fixed

Verified with a debug probe on a separate compose project (`sshub-dbg`), read
only apart from the scenario's own steps. After scenario 40 as it was:

```
$ sqlite3 "$db" "SELECT local, state, kind, count(*) ... FROM sync_ops GROUP BY local, state, kind;"
0|pending|DeviceAdd|1|2067|2067
local_count=0
```

Peer-a had no ring of its own, so `host add web40` wrote no `sync_ops` row
authored locally. `local` is set only by `sync_append_local`
(`src/sync/store_impl.rs:138`, `src/sync/merge.rs:141`), so `WHERE local = 1`
matched nothing and the trailing `test ... -gt 0` failed with no output.

Test bug: scenario 40 never founded the ring on peer-a. Scenario 10 does
`enroll_peer peer-a hub` first; scenario 40 went straight to peer-b joining
peer-a. Fixed in `scenarios/40-tamper.sh` by adding the founding sequence from
scenario 10. The tamper step now reports its exit status and an explicit
message on failure, and `[ -n "$tout" ]` flags a silent failure.

Not yet rerun. Peer-b's `sync-now` in the failing run showed `rejected: 0`,
`sent: 1`, `received: 0`; that result is not valid evidence until 40 runs on
a ring that really has local ops.

## Verified passing behaviour

- Offline peer detected as `failed` within 5 s; peer-b stays `ok` (30).
- Hostile config prompts no password and leaves pending unchanged (30).
- `sync-status` reports `verified_by: "sas"` for "this device" and
  `ssh_host_key` for others on every node.

# Credential and Red-operation scenarios (50-95, branch poc/e2e-creds)

Runs: `hs-e2e2-20261024T195221.log` (first full run), `hs-e2e2-20261024T202008.log`
(second run, killed by the 5100 s bound inside 90), `hs-e2e-dbg*.log` (single-scenario runs).
Every command below is the real CLI inside the peers; no step is faked.

| Scenario | Result | What it shows |
|---|---|---|
| 50-vault-credential | FAIL 2 (kept) | kit import, host add writes a CredUpdate op and no plaintext copy, ciphertext reaches peer-b; `vault1` never reaches peer-b (P1) |
| 60-quorum-attended-peer | FAIL 1 (kept) | paper registration needed first, mode change proposed (Red), peer-b's vote not counted (P3), share release has no CLI verb (P4) |
| 70-red-revoke | FAIL 2 (kept) | revoke of a holder blocked: roles disagree between devices (P3), phone proof missing (P5) |
| 80-recovery-timelock | PASS | start, approve, ack, status (NeedsMoreTime ready_at +7d), cancel by factor (CancelledByFactor); timelock expiry not drivable (P6) |
| 90-checkpoint-bootstrap | not completed | checkpoint is proposed at the 8 MiB threshold (state `verified` on peer-a) but not accepted; very slow large-log sync (P7) |
| 95-export | PASS (first run) | refused with "ceremony is not complete", nothing written; ceremony cannot be completed headless (P3, P5) |

## P1. CLI host changes never become ops (product, blocks every replicated-host assertion)

`host add --name vault1 --address vault1.internal --password-stdin` on a ring member:

```
$ sqlite3 launcher.db 'SELECT kind,count(*) FROM sync_ops GROUP BY kind'     (after host add)
CredUpdate|1
DeviceAdd|1
$ sshub host edit --name vault1 --set-notes n1                               (no new op)
```

No `HostUpsert` is ever appended: nothing in `src/` outside tests builds one from a local change. Only the
credential op is produced (by the guarded password store). peer-b receives the ciphertext
(`sync_cred_state` has the row) and `sshub host resolve vault1` answers `Error: host 'vault1' not found`.
This also explains F1 of the earlier section (held host changes never listed).

## P2. `sync-migrate` is a no-op once a Vault exists (expected, noted)

`host add --password-stdin` replicates through the gate and removes the local copy, so
`sync-migrate` reports `{"migrated":0,"skipped":2}`. Not a bug; migration is for secrets that predate the ring.

## P3. Devices disagree about each other's role; the second-device proof cannot be given (product)

After `sync-join` (a joins via hub, b joins via hub, a approves b) `sync-status`:

```
peer-a sees:  peer-b holder, hub mirror        peer-b sees: peer-a mirror, peer-b mirror, hub holder
```

`sync-approve <op>` on peer-b for a Red op answers `have [], missing [phone, second device, third factor]`: peer-b's
vote is not counted because peer-b regards itself as a mirror. `sync-revoke peer-b` on peer-a is then
`blocked: revoking a holder needs at least two remaining terminal holders to contribute`. A ring built only
with the CLI has no second attended holder, so a Red ceremony cannot complete.

## P4. No CLI verb requests a share release (product)

`sshub host resolve quorum1` in Quorum mode: the CLI unlocker answers
`no way to ask another holder from the command line yet: open it from the TUI` (src/cli/unlock.rs `PromptShares`).
The approving side (`sync-approve` of a ShareRelease) has nothing to approve.

## P5. No verb signs a RedPhoneSign (product)

Every Red ceremony of a ring lists `phone` as missing; the e2e ring has no phone device and no CLI verb produces
`RedPhoneSign`. The phone proof therefore never completes headless.

## P6. No test clock (product)

`RECOVERY_TIMELOCK_SECS` (7 days), `CHECKPOINT_PRUNE_DELAY_SECS` (7 days) and `CHECKPOINT_MIN_INTERVAL_SECS` are
constants read against the wall clock; no env var or config moves them. Recovery completion, prune and the bootstrap
of a fresh peer from a pruned log cannot be driven.

## P7. Checkpoint stays `verified` and a large-log sync is slow (product, needs triage)

After 4502 CredUpdate ops (8.8 MB) peer-a proposes a checkpoint (`verified|14109`), peer-b never holds a row. A later
`sync-join` of a fresh peer-c against that log timed out (`offline: timed out waiting for the peer`) and the scenario
then ran into the 5100 s bound. `snowt` in lib-creds.sh now bounds and times the rounds; open question whether the
cause is peer-b's pending CredUpdates blocking the verification (`pending below the cut`), or the size of the round.

## Test-side fixes made on the way

`device_name()` makes the founder show up as "this device" (not a product contract); `host edit` takes `--name`;
roles come from the observed table.

## The phone in the e2e ring (feature/host-sync d49e4e0 and 6726118, page poc/phone 7fb1091)

The ring's phone is the committed companion page driven through the real CLI verbs (`sync-phone-enroll`,
`sync-notice`, `sync-phone-ops`, `sync-share-request`); no shim remains. Logs: `hs-e2e-phone3f-20261025T202759.log`
(65 green on d49e4e0), `hs-e2e-phone3l-20261025T221224.log` (95 green on d49e4e0), `hs-e2e-phone4b-20261026T001102.log`
and `hs-e2e-phone4e-20261026T010122.log` (6726118).

### Q1. A ring built with the CLI cannot reach "two terminal holders plus the phone" (product, blocks 60, 65, 70, 95 on 6726118)

On 6726118 `sync-join` leaves every joiner a mirror, and the earlier "everyone is a holder" views are gone (they were the
P3 bug). A second holder and the phone both need a Red change that the ceremony blocks for want of the other:

```
$ sshub sync-role peer-b holder          (ring: peer-a holder; peer-b, peer-c, hub mirrors)
proposed: ee6751cd becomes Holder (op f6ad5408496bbb36)
ceremony: have [], missing [phone, second device, third factor]
blocked: adding or promoting a holder needs two other live holders for the reshare
```

The same `blocked` line is on the phone's own admission (`DeviceAdd` Holder/Phone, printed by `sync-phone-enroll`:
`ceremony: have [], missing [phone, second device, third factor]`, then `blocked: adding or promoting a holder needs two
other live holders`). `live_terminals` (src/sync/ceremony.rs, `live_terminals < 2`) counts terminal holders only, and a
CLI ring has exactly one. So the phone cannot be admitted, no Red op ever has a phone proof, and promoting a second holder
needs that phone. Everything the phone-completed ceremonies need (60 mode change, 70 revoke of a holder, 95 export) is
unreachable. On d49e4e0 the same ring "worked" only because the two joiners showed up as holders in peer-a's table.

### Q2. The earlier `sync-revoke`: "the ring has 1" (explained)

On d49e4e0 peer-a listed 3 holders but the revoke counted 1 remaining terminal holder. On 6726118 peer-b and peer-c are
mirrors in every peer's table and `sync-revoke peer-c` is accepted with no ceremony field (`{"op":"cde093dd...","steps":[],"tier":"Red"}`,
status `revoked` on peer-a only, `Pending changes: 1`). The message is gone with the demotion bug; the Red revoke
of a holder needs Q1 solved first. peer-b and peer-c do not see the revocation until peer-a's op reaches them.

### Q3. Test-side notes

* The phone verbs print a first `{"event":"companion","fingerprint":...}` line; helpers skip it (`enr_get`, `pf`).
* A ring set up with `sync-join` needs `sync-now` on peer-a before `sync-review --approve`, or peer-a has nothing to approve.
* `sync-join` between peer-b and peer-c ("records peer-c directly") ends in `Broken pipe` on 6726118; peer-b still lists peer-c.
* Order on d49e4e0: paper registration proposed first, then the phone, then phone and peer-b complete both. On 6726118 the
  factor-free first phone makes the phone first; neither order gets past Q1.
