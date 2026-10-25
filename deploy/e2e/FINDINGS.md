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
