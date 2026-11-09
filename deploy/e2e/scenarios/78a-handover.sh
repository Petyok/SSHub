#!/bin/sh
# timeout: 3000
# 78a: PhoneHandover (design 5, contract 4j/4l). Ring of two terminals and the old phone with the paper. The NEW phone
# (second service) enrolls with `sync-phone-enroll --replace`; the terminal prints the kind 12 request and eight words;
# the OLD phone shows the same eight words and signs; peer-a's `sync-phone-ops` prints the second kind 12 frame (with the
# op hash) which the NEW phone shows and signs as its own admission; a second terminal (peer-b) and the paper complete the
# ceremony. Then the new phone is the ring's phone and the old one is not.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=78a-handover

check "the second phone daemon answers" as_phone2 phone_ready
step "the second phone restores its own key" as_phone2 phone_prep_page
P1_ID=$(pf "$(phone info)" expected_device_id)
step "ring with the old phone (ring2)" ring_with_phone ring2
step "handover: both phones signed" handover_start
show "peer-b sync-approve the handover (the second terminal)" peer peer-b $SSHUB sync-approve "$HO_OP" --format json
spread peer-a peer-b peer-a
show "peer-a adds the paper factor" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor $HO_OP --paper --format json"
expect "the handover ceremony is complete" "$LAST_OUT" '.ceremony.complete == true'
spread peer-b peer-a peer-b
for p in peer-a peer-b; do
  st=$(peer $p $SSHUB sync-status --format json 2>/dev/null || true)
  echo "$p devices: $(printf '%s' "$st" | pjq -c '[.devices[] | {name, role, status}]')"
  expect "$p: the new phone is a full device in force" "$st" '[.devices[] | select(.name=="new phone") | .role] == ["holder"]'
  expect "$p: the old phone is no longer a phone in force" "$st" '[.devices[] | select(.name=="my phone") | (.role != "holder" or .status == "revoked")] == [true]'
done
ring_view
finish "$NAME"
