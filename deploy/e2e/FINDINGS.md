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
contradiction or an unexplained failure, with the exact output observed.

## F1. Held host change never reaches the review queue (10)

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

## F2. Ring device with no recorded host is omitted from `sync-now` (20)

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

## F3. Tamper step fails silently (40)

`40-tamper.sh` runs on peer-a:

```
db=$(find "$HOME/.local/share/sshub" -name launcher.db | head -n1)
sqlite3 "$db" "UPDATE sync_ops SET canonical = zeroblob(1) || substr(canonical, 2) WHERE local = 1;"
test "$(sqlite3 "$db" "SELECT count(*) FROM sync_ops WHERE local = 1;")" -gt 0
```

`psh peer-a "$tamper"` exits non-zero with empty stderr (`tamper peer-a local ops`
shows `     | ` only). Schema check: `sync_ops.canonical BLOB NOT NULL`,
`sync_ops.local INTEGER` (`src/store/migrate.rs:955-974`), so the UPDATE is
valid SQL.

Unverified hypothesis: the trailing `test ... -gt 0` fails silently when no
local ops are marked `local = 1`. Not confirmed; needs a DB query inside the
peer-a container.

Consequence: peer-b's `sync-now` reports `rejected: 0`, `received: 0`,
`sent: 1`. Because the tamper never ran, this does not show that the product
accepts tampered ops. The scenario 40 result is therefore inconclusive, not a
pass.

## Verified passing behaviour

- Offline peer detected as `failed` within 5 s; peer-b stays `ok` (30).
- Hostile config prompts no password and leaves pending unchanged (30).
- `sync-status` reports `verified_by: "sas"` for "this device" and
  `ssh_host_key` for others on every node.
