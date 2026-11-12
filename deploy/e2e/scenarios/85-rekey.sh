#!/bin/sh
# timeout: 2400
# 85: rekey (design 4.2). A Quorum ring with the phone and two terminals; peer-a removes peer-b (degraded, stolen). The
# credential peer-b held a piece of is on the exposed list. `sync-rekey` without options lists it; with `--stdin` the user
# says "I changed it on the server, save the new password" and the credential is rewritten to the current holders: the
# exposed list is empty afterwards.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=85-rekey
NEWPW='Ch4nged-Pw-e2e'

step "Quorum ring with the phone (ring2)" quorum_ring ring2
B_ID=$(devid peer-a peer-b)
show "peer-a sync-revoke peer-b" peer peer-a $SSHUB sync-revoke "$B_ID" --format json
check "the removal is proposed" "[ '$LAST_RC' -eq 0 ]"
step "removal ceremony (phone and paper)" red_phone_paper "$(opid "$LAST_OUT")"
spread peer-a peer-a
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "quorum1 is exposed" "$st" 'any(.exposure[]; test("quorum1"))'

show "peer-a sync-rekey (no options lists the exposed credentials)" peer peer-a $SSHUB sync-rekey --format json
check "sync-rekey lists" "[ '$LAST_RC' -eq 0 ]"
expect "the list names quorum1" "$LAST_OUT" 'any(.exposed[]; . == "quorum1")'

show "peer-a sync-rekey --stdin (host,newpassword)" sh -c "printf 'quorum1,%s\n' '$NEWPW' | $C exec -T peer-a $SSHUB sync-rekey --stdin --format json"
check "sync-rekey saves" "[ '$LAST_RC' -eq 0 ]"
expect "one credential saved" "$LAST_OUT" '.saved >= 1'
spread peer-a peer-a
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "the exposed list is empty after the change" "$st" '(.exposure | length) == 0'
show "peer-a sync-rekey again" peer peer-a $SSHUB sync-rekey --format json
expect "nothing is left to change" "$LAST_OUT" '(.exposed | length) == 0'
finish "$NAME"
