#!/bin/sh
# 70: Red revocation of a holder, with the phone. Three attended holders and the phone; peer-a proposes
# `sync-revoke peer-c`. The ceremony wants the phone (notice, approve, ops text in), a second attended holder (peer-b)
# and the paper factor. Every step is the real CLI or the phone page.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=70-red-revoke

step "ring with the phone (ring3)" ring_with_phone ring3
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "three terminals and the phone are in the ring" "$st" '(.devices | length) >= 5'
C_ID=$(devid peer-a peer-c)
check "peer-a can name peer-c" "test -n '$C_ID'"
echo "holders as peer-a sees them: $(holders peer-a)"

ring_view
show "peer-a sync-revoke peer-c (a holder)" peer peer-a $SSHUB sync-revoke "$C_ID" --format json
expect "revoking a holder is a Red change with a ceremony" "$LAST_OUT" '.tier == "Red" and (.ceremony != null)'
REV=$(opid "$LAST_OUT")
echo "missing right after the proposal: $(printf '%s' "$LAST_OUT" | pjq -c '.ceremony.missing')"
step "revocation ceremony (phone, peer-b, paper)" red_complete "$REV"
expect "the revocation ceremony is complete" "$LAST_OUT" '.ceremony.complete == true'
spread peer-b peer-a peer-b
for p in peer-a peer-b; do
  echo "$p sees peer-c as: $(peer $p $SSHUB sync-status --format json | pjq -c '[.devices[] | select(.name=="peer-c") | {status, role}]')"
done
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-c is revoked on peer-a" "$st" '[.devices[] | select(.name=="peer-c") | .status] == ["revoked"]'
show "peer-c after the revocation: a round with peer-a" peer peer-c $SSHUB sync-now --format json
RV_OUT=$LAST_OUT
check "the revoked device is refused by peer-a" 'printf "%s" "$RV_OUT" | grep -q Revoked'
dump_state "after revoke"

finish "$NAME"
