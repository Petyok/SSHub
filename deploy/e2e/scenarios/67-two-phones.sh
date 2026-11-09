#!/bin/sh
# timeout: 1500
# 67: the second phone (compose service `phone2`: its own Chromium, key and profile) works like the first, and the product
# keeps one phone per ring. This is the infrastructure gate of the PhoneHandover scenarios 78a-d, which put the old and the new
# phone side by side once handover exists. Today:
#  1. the first phone sets up a ring; the second phone, enrolling into that ring, is refused ("replacing it is a recovery");
#  2. in a fresh ring the second phone sets everything up alone (restore, enroll, own admission, promotion, paper).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=67-two-phones

check "the first phone daemon answers" phone_ready
check "the second phone daemon answers" as_phone2 phone_ready
show "phone info" phone info
expect "the first phone runs the pinned page" "$LAST_OUT" '.fingerprint_ok == true'
P1=$(pf "$LAST_OUT" expected_device_id)
show "phone2 info" phone2 info
expect "the second phone runs the pinned page" "$LAST_OUT" '.fingerprint_ok == true'
P2=$(pf "$LAST_OUT" expected_device_id)
check "the two phones have different keys" "[ -n '$P1' ] && [ -n '$P2' ] && [ '$P1' != '$P2' ]"

step "ring with the first phone (ring2)" ring_with_phone ring2
# the second phone tries to enroll into the same ring: the terminal refuses after the four words
step "phone2 restores its own key" as_phone2 phone_prep_page
enrol_second_phone() {
  enr_open peer-a sync-phone-enroll --format json
  E1=$(enr_get 1)
  show "phone2 opens the enrollment" phone2 enroll-start '{"name":"new phone"}'
  show "phone2 reads the terminal commitment" phone2 enroll-commit "{\"frame\":\"$(pf "$E1" frame)\"}"
  enr_send "$(pf "$LAST_OUT" frame)"
  E2=$(enr_get 2)
  show "phone2 reads the terminal reveal" phone2 enroll-reveal "{\"frame\":\"$(pf "$E2" frame)\"}"
  enr_send "$(pf "$LAST_OUT" frame)"
  enr_get 3 >/dev/null
  show "phone2 taps: words match" phone2 enroll-match
  enr_send "$(pf "$LAST_OUT" frame)"
  enr_get 4 >/dev/null
  enr_send y
  E5=$(enr_get 5)
  echo "terminal: $E5"
  if enr_close; then return 1; fi
  printf '%s' "$E5" | grep -q 'already has a phone'
}
step "the ring refuses a second phone (replacing it is a recovery)" enrol_second_phone

# a fresh ring: the second phone alone, through the same helpers
step "ring with the second phone only (ring2)" as_phone2 ring_with_phone ring2
for p in peer-a peer-b; do
  st=$(peer "$p" $SSHUB sync-status --format json 2>/dev/null || true)
  expect "$p lists the second phone's key as a holder" "$st" "[.devices[] | select(.id==\"$P2\") | .role] == [\"holder\"]"
  expect "$p does not know the first phone's key" "$st" "[.devices[] | select(.id==\"$P1\")] == []"
done
finish "$NAME"
