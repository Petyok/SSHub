#!/bin/sh
# timeout: 2400
# 88: group approval (design 7c, W-APPROVE). `sync-review --approve-all --device <id>` approves ONLY the changes of that
# device that are held as `stale` and touch no dangerous field; everything else stays in the queue.
# A `stale` hold needs a device whose head was older than STALE_AFTER_SECS at an ACCEPTED checkpoint. The e2e image
# (--features e2e-hooks) shortens that: SSHUB_E2E_STALE_AFTER_SECS, SSHUB_E2E_CHECKPOINT_OPS and
# SSHUB_E2E_CHECKPOINT_MIN_INTERVAL_SECS (set on every call below, nowhere else). peer-b goes silent, peer-a and peer-c
# grow the log and accept a checkpoint; peer-b's later edits then arrive held as `stale`:
#   b1: a label (safe, stale)           -> approved by the group verb
#   b2: an address (dangerous, stale)   -> stays
#   c1: an address by peer-c (not stale, held for review) -> stays, even though --device names peer-b only
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=88-review-grouping
HOOKS='SSHUB_E2E_STALE_AFTER_SECS=30 SSHUB_E2E_CHECKPOINT_OPS=20 SSHUB_E2E_CHECKPOINT_MIN_INTERVAL_SECS=5'
# every sshub call of this scenario: peer P, then the sshub arguments
hp() { p=$1; shift; peer "$p" env $HOOKS $SSHUB "$@"; }
hround() { for p in "$@"; do hp "$p" sync-now --format json >/dev/null 2>&1 || true; done; }

step "ring of three terminals" ring3
for h in g1 g2 g3 g4; do check "peer-a adds host $h" "hp peer-a host add --name $h --address $h.internal"; done
hround peer-b peer-c peer-a peer-b peer-c
for p in peer-b peer-c; do for id in $(hp $p sync-review --format json 2>/dev/null | pjq -r '.[].id'); do hp $p sync-review --approve "$id" >/dev/null 2>&1 || true; done; done
hround peer-b peer-c peer-a
echo "--- peer-b goes silent now; the stale threshold is 30 s"
sleep 40
echo "--- peer-a grows the log (25 hosts) and peer-a and peer-c accept a checkpoint without peer-b"
psh peer-a "export $HOOKS; for i in \$(seq 1 25); do sshub --profile default host add --name bulk\$i --address bulk\$i.internal >/dev/null 2>&1 || echo add \$i failed; done"
n=0
while [ "$(ckpts_accepted peer-a)" -lt 1 ] && [ "$n" -lt 12 ]; do
  n=$((n + 1))
  hround peer-a peer-c peer-a
  for p in peer-a peer-c; do for id in $(hp $p sync-review --format json 2>/dev/null | pjq -r '.[].id'); do hp $p sync-review --approve "$id" >/dev/null 2>&1 || true; done; done
  sleep 6
done
show "peer-a checkpoints" sq peer-a 'SELECT status, state_len FROM sync_checkpoints'
show "peer-a stale markers" sq peer-a "SELECT key FROM sync_meta WHERE key LIKE 'stale:%'"
check "peer-a accepted a checkpoint" "[ \"$(ckpts_accepted peer-a)\" -ge 1 ]"
check "peer-b is marked stale at the cut" "[ \"$(sq peer-a "SELECT count(*) FROM sync_meta WHERE key = 'stale:$(devid peer-a peer-b)'")\" = 1 ]"

echo "--- peer-b comes back and edits; peer-c makes an ordinary held change"
hround peer-b peer-a peer-b
check "b1: peer-b sets a label on g1 (safe)" "hp peer-b host edit --name g1 --set-label lab-b1"
check "b2: peer-b changes the address of g2 (dangerous)" "hp peer-b host edit --name g2 --set-address b2.changed.internal"
check "c1: peer-c changes the address of g3 (not stale)" "hp peer-c host edit --name g3 --set-address c1.changed.internal"
hround peer-b peer-c peer-a
q=$(hp peer-a sync-review --format json 2>/dev/null || true)
echo "$q" | head -n 60
B_ID=$(devid peer-a peer-b)
expect "peer-b's label change is held as stale" "$q" 'any(.[]; .reason != null and (.reason | test("away too long|stale")) and any(.changes[]?; .field == "label" and .dangerous == false))'
expect "peer-b's address change is held as stale and dangerous" "$q" 'any(.[]; (.reason | test("away too long|stale")) and any(.changes[]?; .field == "address" and .dangerous == true))'
expect "peer-c's address change is held, not as stale" "$q" 'any(.[]; (.reason | test("away too long|stale") | not) and any(.changes[]?; .field == "address" and .to == "c1.changed.internal"))'
BEFORE=$(printf '%s' "$q" | pjq 'length')

show "peer-a sync-review --approve-all --device peer-b" hp peer-a sync-review --approve-all --device "$B_ID" --format json
expect "exactly the one safe stale change is approved, the rest is left" "$LAST_OUT" ".approved == 1 and .left == $((BEFORE - 1))"
q2=$(hp peer-a sync-review --format json 2>/dev/null || true)
expect "peer-b's label change is gone from the queue" "$q2" 'all(.[]; all(.changes[]?; .field != "label" or .to != "lab-b1"))'
expect "peer-b's dangerous address change is still waiting" "$q2" 'any(.[]; any(.changes[]?; .to == "b2.changed.internal"))'
expect "peer-c's address change is still waiting" "$q2" 'any(.[]; any(.changes[]?; .to == "c1.changed.internal"))'
check "the label is applied on peer-a" "hp peer-a host list --format json 2>/dev/null | grep -q lab-b1"
check "the dangerous address is not applied on peer-a" "! hp peer-a host list --format json 2>/dev/null | grep -q b2.changed.internal"

show "--device without --approve-all is a usage error" hp peer-a sync-review --device "$B_ID" --format json
check "usage error exit status 2" "[ '$LAST_RC' -eq 2 ]"
finish "$NAME"
