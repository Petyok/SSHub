#!/bin/sh
# timeout: 3000
# 78b: a handover finished with the EMAILED CODE instead of the paper is refused (contract 4j: "the emailed code never
# counts for a handover"). Same ring as 78a plus an email factor registered on the Hub (its code is printed once). Both
# phones signed and a second terminal approved; the code as the third factor does not complete the ceremony, and the new
# phone is not a full device. The paper phrase then completes it (the proof that only the factor was missing).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=78b-thief-handover

check "the second phone daemon answers" as_phone2 phone_ready
step "the second phone restores its own key" as_phone2 phone_prep_page
step "ring with the old phone (ring2)" ring_with_phone ring2
show "hub registers the emailed-code factor (it is the sender)" peer hub $SSHUB sync-factor-register email --format json
CODE=$(pf "$LAST_OUT" code)
check "the Hub printed its code" "test -n '$CODE'"
REGE=$(opid "$LAST_OUT")
spread hub peer-a peer-b peer-a
step "email factor ceremony (phone, peer-b, paper)" red_complete "$REGE"
spread peer-a peer-b hub peer-a

step "handover: both phones signed" handover_start
show "peer-b sync-approve the handover (the second terminal)" peer peer-b $SSHUB sync-approve "$HO_OP" --format json
spread peer-a peer-b peer-a
show "peer-a adds the emailed code as the third factor" sh -c "printf '%s\n' '$CODE' | $C exec -T peer-a $SSHUB sync-factor $HO_OP --email-code --format json"
spread peer-b peer-a peer-b
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "the new phone is not a full device (the code never counts)" "$st" '[.devices[] | select(.name=="new phone") | .role] != ["holder"]'
show "peer-a sync-review (the handover waits)" peer peer-a $SSHUB sync-review --format json
echo "ceremony of the handover: $(printf '%s' "$LAST_OUT" | pjq -c '[.[] | select(.id | startswith("'"$(printf '%s' "$HO_OP" | cut -c1-8)"'")) | .ceremony]')"

show "peer-a adds the paper factor" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor $HO_OP --paper --format json"
expect "with the paper the ceremony is complete" "$LAST_OUT" '.ceremony.complete == true'
spread peer-b peer-a peer-b
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "now the new phone is a full device" "$st" '[.devices[] | select(.name=="new phone") | .role] == ["holder"]'
finish "$NAME"
