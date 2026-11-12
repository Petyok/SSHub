#!/bin/sh
# timeout: 2700
# 84: heal (design 4.1). A Quorum ring with the phone gets a new terminal (peer-c, promoted with the full ceremony).
# The credential written before it is "not held by" peer-c (the exposure list says healing on use). `sync-heal` on a
# terminal that holds a share (paper phrase on stdin) re-seals it to every current holder: the list empties and peer-c
# opens the credential, asking peer-a for its share like any attended peer.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=84-heal

step "Quorum ring with the phone (ring2)" quorum_ring ring2
check "peer-c joins via hub" "enroll_peer peer-c hub"
check "peer-a approves peer-c's admission" "snow peer-a >/dev/null; approve_all peer-a"
check "peer-c records peer-a directly" "enroll_peer peer-c peer-a"
check "peer-b records peer-c directly" "enroll_peer peer-b peer-c"
show "peer-a sync-role peer-c holder (the third terminal)" peer peer-a $SSHUB sync-role "$(devid peer-a peer-c)" holder --format json
check "the promotion is proposed" "[ '$LAST_RC' -eq 0 ]"
step "promotion ceremony (phone, peer-b, paper)" red_complete "$(opid "$LAST_OUT")"
spread peer-b peer-c peer-a peer-b peer-c
approve_all peer-c >/dev/null 2>&1 || true
spread peer-c peer-a peer-b peer-c

st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-c is a full device" "$st" '[.devices[] | select(.name=="peer-c") | .role] == ["holder"]'
expect "quorum1 is not shared with peer-c (healing on use)" "$st" 'any(.exposure[]; test("not shared with") and test("quorum1"))'
check "peer-c imports the kit (a new holder has no ring secret yet)" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-c $SSHUB sync-import-kit"
show "peer-a sync-heal (paper phrase on stdin)" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-heal --format json"
check "sync-heal succeeds" "[ '$LAST_RC' -eq 0 ]"
expect "it healed quorum1" "$LAST_OUT" '(.healed | length) >= 1 and (.planned | length) >= 1'
spread peer-b peer-c peer-a peer-b peer-c
approve_all peer-c >/dev/null 2>&1 || true
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "the exposure list is empty on peer-a" "$st" '(.exposure | length) == 0'
st=$(peer peer-c $SSHUB sync-status --format json 2>/dev/null || true)
expect "the exposure list is empty on peer-c" "$st" '(.exposure | length) == 0'

# ---- peer-c opens the credential: it asks peer-a for its share, peer-a approves on the command line
psh peer-c 'rm -f /tmp/e2e-resolve.out'
$C exec -T peer-c env SSHUB_UNLOCK_WAIT=150 $SSHUB host resolve quorum1 --format json > /tmp/e2e-resolve-c.out 2>&1 &
RPID=$!
REL=""
i=0
while [ -z "$REL" ] && [ "$i" -lt 40 ]; do
  i=$((i + 1)); sleep 3
  snow peer-a >/dev/null
  REL=$(peer peer-a $SSHUB sync-review --format json 2>/dev/null | pjq -r '[.[] | select(.summary | test("saved password|share|release"; "i")) | .id][0] // empty')
done
echo "ShareRelease seen by peer-a: ${REL:-none}"
check "peer-a received peer-c's share request" "test -n '$REL'"
if [ -n "$REL" ]; then show "peer-a sync-review --approve the share release" peer peer-a $SSHUB sync-review --approve "$REL"; fi
wait $RPID; RC=$?
echo "--- peer-c host resolve quorum1 (exit $RC):"; sed 's/^/    | /' /tmp/e2e-resolve-c.out
check "peer-c's resolve ended with the credential open" "[ $RC -eq 0 ]"
ring_view
finish "$NAME"
