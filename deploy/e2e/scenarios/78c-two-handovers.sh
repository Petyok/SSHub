#!/bin/sh
# timeout: 3600
# 78c: two handovers of one phone: the one that COMPLETES first wins (contract 4j: among handovers by one author the
# lowest completion lamport, device, seq wins). The old phone signs two handovers (A "new phone", then B "other phone",
# both to the second phone's key). B is completed first (second terminal and paper), then A. Every terminal agrees that
# B is the ring's phone; A, completed later, is not.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=78c-two-handovers

check "the second phone daemon answers" as_phone2 phone_ready
step "the second phone restores its own key" as_phone2 phone_prep_page
step "ring with the old phone (ring2)" ring_with_phone ring2
PH2_NAME="new phone"; step "handover A: both phones signed" handover_start
HO_A=$HO_OP
PH2_NAME="other phone"; step "handover B: both phones signed" handover_start
HO_B=$HO_OP
check "two different handover ops" "[ -n '$HO_A' ] && [ -n '$HO_B' ] && [ '$HO_A' != '$HO_B' ]"

echo "--- B completes first"
show "peer-b sync-approve B" peer peer-b $SSHUB sync-approve "$HO_B" --format json
spread peer-a peer-b peer-a
show "peer-a adds the paper factor to B" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor $HO_B --paper --format json"
expect "B is complete" "$LAST_OUT" '.ceremony.complete == true'
echo "--- A completes afterwards"
spread peer-a peer-b peer-a
show "peer-b sync-approve A" peer peer-b $SSHUB sync-approve "$HO_A" --format json
spread peer-a peer-b peer-a
show "peer-a adds the paper factor to A" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor $HO_A --paper --format json"
spread peer-b peer-a peer-b
for p in peer-a peer-b; do
  st=$(peer $p $SSHUB sync-status --format json 2>/dev/null || true)
  echo "$p devices: $(printf '%s' "$st" | pjq -c '[.devices[] | {name, role, status}]')"
  expect "$p: the phone in force is B, 'other phone'" "$st" '[.devices[] | select(.name=="other phone") | .role] == ["holder"]'
  expect "$p: A's name is not a phone in force" "$st" '[.devices[] | select(.name=="new phone") | .role] != ["holder"]'
done
ring_view
finish "$NAME"
