#!/bin/sh
# timeout: 2400
# 72e: three terminals and the phone; the second terminal (peer-b) is away: it takes no part after the setup. peer-a
# removes peer-c with the phone and the paper alone (design 3.2 rule 1: the second device is waived when the paper is in
# the proofs). When peer-b comes back it learns the removal in its next round.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=72e-three-terminals-paper

step "ring with the phone (ring3)" ring_with_phone ring3
C_ID=$(devid peer-a peer-c)
check "peer-a can name peer-c" "test -n '$C_ID'"
show "peer-a sync-revoke peer-c" peer peer-a $SSHUB sync-revoke "$C_ID" --format json
check "the removal is proposed" "[ '$LAST_RC' -eq 0 ]"
REV=$(opid "$LAST_OUT")
step "removal ceremony (phone and paper; peer-b is away)" red_phone_paper "$REV"
expect "the removal ceremony is complete without peer-b" "$LAST_OUT" '.ceremony.complete == true'
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-c is removed on peer-a" "$st" '[.devices[] | select(.name=="peer-c") | .status] == ["revoked"]'
echo "--- peer-b comes back"
spread peer-b peer-a peer-b
st=$(peer peer-b $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-b learned the removal" "$st" '[.devices[] | select(.name=="peer-c") | .status] == ["revoked"]'
ring_view
finish "$NAME"
