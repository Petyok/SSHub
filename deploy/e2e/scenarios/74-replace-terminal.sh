#!/bin/sh
# timeout: 2700
# 74: "Replace this terminal" (design 3.3) in a ring of two terminals and the phone: peer-b is retired (a dead disk),
# peer-c is the new machine. One guided verb in three steps: enroll (it lists the enrolled candidates), promote
# (phone, the other terminal and the paper, which the verb adds from stdin) and remove the old one (phone, the other
# terminal, paper). The last step points at the heal and rekey verbs.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=74-replace-terminal

step "ring with the phone (ring2)" ring_with_phone ring2
B_ID=$(devid peer-a peer-b)
check "peer-c joins via hub" "enroll_peer peer-c hub"
check "peer-a approves peer-c's admission" "snow peer-a >/dev/null; approve_all peer-a"
check "peer-c records peer-a directly" "enroll_peer peer-c peer-a"
check "peer-b records peer-c directly" "enroll_peer peer-b peer-c"
spread peer-b peer-c peer-a peer-b peer-c

show "peer-a sync-replace-terminal peer-b (step 1)" peer peer-a $SSHUB sync-replace-terminal "$B_ID" --format json
expect "step 1 is the enrollment and lists peer-c" "$LAST_OUT" '.step == "enroll" and any(.candidates[]; . == "peer-c")'

show "peer-a sync-replace-terminal peer-b --new peer-c (step 2)" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-replace-terminal '$B_ID' --new peer-c --format json"
expect "step 2 proposes the promotion of peer-c" "$LAST_OUT" '.step == "promote" and (.op | length) > 0'
PRO=$(opid "$LAST_OUT")
phone_signs peer-a "$PRO"
spread peer-b peer-a peer-b
show "peer-b is given the phone's list" sh -c "printf '%s\n' '$PHONE_OPS' | $C exec -T peer-b $SSHUB sync-phone-ops --format json"
snow peer-b >/dev/null
show "peer-b sync-approve the promotion (the other terminal)" peer peer-b $SSHUB sync-approve "$PRO" --format json
spread peer-a peer-b peer-c peer-a
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-c is a full device now" "$st" '[.devices[] | select(.name=="peer-c") | .role] == ["holder"]'

show "peer-a sync-replace-terminal peer-b --new peer-c (step 3)" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-replace-terminal '$B_ID' --new peer-c --format json"
expect "step 3 proposes the removal of peer-b" "$LAST_OUT" '.step == "revoke" and (.op | length) > 0'
REV=$(opid "$LAST_OUT")
phone_signs peer-a "$REV"
spread peer-c peer-a peer-c
show "peer-c sync-approve the removal (the other terminal)" peer peer-c $SSHUB sync-approve "$REV" --format json
spread peer-a peer-c peer-a
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-b is removed" "$st" '[.devices[] | select(.name=="peer-b") | .status] == ["revoked"]'
expect "peer-a, peer-c and the phone are the full devices" "$st" '[.devices[] | select(.role=="holder" and .status != "revoked") | .name] | sort == ["my phone","peer-a","peer-c"]'
ring_view
finish "$NAME"
