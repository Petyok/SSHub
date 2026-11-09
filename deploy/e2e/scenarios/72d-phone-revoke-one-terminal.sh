#!/bin/sh
# timeout: 2400
# 72d: a ring with ONE terminal and the phone, paper registered. Removing the only terminal is refused ("add a
# replacement first"); the replacement is promoted with phone and paper (the second device is waived because nobody else
# can vote: the promotion judges without the promoted subject, design 3.2 rule 3); then the removal of the old terminal
# completes with phone and paper alone.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=72d-phone-revoke-one-terminal

phone_prep_page
reset_peers
check "peer-a founds the ring with hub" "enroll_peer peer-a hub || enroll_peer peer-a hub"
check "peer-b joins via hub" "enroll_peer peer-b hub"
check "peer-a approves peer-b's admission" "snow peer-a >/dev/null; approve_all peer-a"
check "peer-b records peer-a directly" "enroll_peer peer-b peer-a"
enroll_phone peer-a
phone_signs peer-a "$ADD"
show "peer-a registers the paper (phone only)" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor-register paper --format json"
phone_signs peer-a "$(opid "$LAST_OUT")"
A_ID=$(devid peer-a peer-a)
B_ID=$(devid peer-a peer-b)
check "peer-a names both terminals" "test -n '$A_ID' && test -n '$B_ID'"

show "peer-a sync-revoke peer-a (the only terminal)" peer peer-a $SSHUB sync-revoke "$A_ID" --format json
check "the removal of the last terminal is refused" "[ '$LAST_RC' -ne 0 ]"
check "the refusal says to add a replacement first" "printf '%s' \"\$LAST_OUT\" | grep -qi 'replacement'"

show "peer-a sync-role peer-b holder (the replacement)" peer peer-a $SSHUB sync-role "$B_ID" holder --format json
check "the promotion is proposed" "[ '$LAST_RC' -eq 0 ]"
PRO=$(opid "$LAST_OUT")
step "promotion with phone and paper" red_phone_paper "$PRO"
# the ring is still in setup (no second terminal yet): the promoted device's own approval stands in for the second device
spread peer-b peer-a peer-b
show "peer-b is given the phone's list" sh -c "printf '%s\n' '$PHONE_OPS' | $C exec -T peer-b $SSHUB sync-phone-ops --format json"
snow peer-b >/dev/null
show "peer-b sync-approve its own promotion (ring setup)" peer peer-b $SSHUB sync-approve "$PRO" --format json
spread peer-a peer-b peer-a
show "peer-a sync-status" peer peer-a $SSHUB sync-status --format json
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
spread peer-b peer-a peer-b
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-b is a full device" "$st" '[.devices[] | select(.name=="peer-b") | .role] == ["holder"]'

show "peer-b sync-revoke peer-a (the old terminal, now that a replacement exists)" peer peer-b $SSHUB sync-revoke "$A_ID" --format json
check "the removal is proposed" "[ '$LAST_RC' -eq 0 ]"
REV=$(opid "$LAST_OUT")
# the phone signs on the author's terminal (peer-b), the paper is added there too
phone_signs peer-b "$REV"
show "peer-b adds the paper factor" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-b $SSHUB sync-factor $REV --paper --format json"
expect "the removal ceremony is complete" "$LAST_OUT" '.ceremony.complete == true'
spread peer-b peer-b
st=$(peer peer-b $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-a is removed" "$st" '[.devices[] | select(.name=="peer-a") | .status] == ["revoked"]'
ring_view
finish "$NAME"
