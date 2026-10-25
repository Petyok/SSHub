#!/bin/sh
# 70: Red revocation of a holder. Three attended holders; peer-a proposes
# `sync-revoke peer-c`. The ceremony wants a second attended device (peer-b),
# the paper factor and, when the ring has a phone, the phone. Every step is the
# real CLI; what the ceremony still misses is printed after each one.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=70-red-revoke

ring3
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "three devices are in the ring" "$st" '(.devices | length) >= 3'
echo "holders as peer-a sees them: $(holders peer-a)"
C_ID=$(devid peer-a peer-c)
check "peer-a can name peer-c" "test -n '$C_ID'"

# The paper factor is registered first (the registration itself is a Red op).
show "peer-a registers the paper factor" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor-register paper --format json"
spread peer-b peer-c peer-a

# Roles as peer-a sees them: peer-b is a holder, peer-c a mirror (finding F4: the
# devices disagree about each other's role). The Red path needs a holder target.
B_ID=$(devid peer-a peer-b)
show "peer-a sync-revoke peer-b (a holder)" peer peer-a $SSHUB sync-revoke "${B_ID:-peer-b}" --format json
expect "revoking a holder is a Red change with a ceremony" "$LAST_OUT" '.tier == "Red" and (.ceremony != null)'
OP=$(opid "$LAST_OUT")
echo "missing right after the proposal: $(printf '%s' "$LAST_OUT" | pjq -c '.ceremony.missing')"

spread peer-c peer-a
show "peer-c sees the revocation as pending" peer peer-c $SSHUB sync-review
show "peer-c sync-approve" peer peer-c $SSHUB sync-approve "$OP" --format json
expect "peer-c's vote is counted as the second device" "$LAST_OUT" '.ceremony.have | any(. == "second device")'
echo "missing after peer-c: $(printf '%s' "$LAST_OUT" | pjq -c '.ceremony.missing')"

spread peer-a peer-b
show "peer-a adds the paper factor to the revocation" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor $OP --paper --format json"
echo "missing after the paper factor: $(printf '%s' "$LAST_OUT" | pjq -c '.ceremony.missing')"
spread peer-b peer-a peer-c

if printf '%s' "$LAST_OUT" | pjq -e '.ceremony.missing | any(. == "phone")' >/dev/null 2>&1; then
  echo "--- GAP: the ceremony wants the phone proof; the e2e ring has no phone device and no CLI verb signs a RedPhoneSign"
fi
for p in peer-a peer-c; do
  echo "$p sees peer-b as: $(peer $p $SSHUB sync-status --format json | pjq -c '[.devices[] | select(.name=="peer-b") | {status, role}]')"
done
st=$(peer peer-c $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-b is revoked on peer-c once the ceremony is complete" "$st" '[.devices[] | select(.name=="peer-b") | .status] == ["revoked"]'
show "peer-b after the revocation: a round with peer-a" peer peer-b $SSHUB sync-now --format json
RV_OUT=$LAST_OUT
check "the revoked device is refused by peer-a" 'printf "%s" "$RV_OUT" | grep -q Revoked'
echo "--- GAP: 'the revoked device's old share opens nothing new' has no CLI verb (share release is TUI only, see 60)"
dump_state "after revoke"

finish "$NAME"
