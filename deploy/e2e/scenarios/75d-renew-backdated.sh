#!/bin/sh
# timeout: 2700
# 75d: a renewal cannot be replayed into more time (design 6.1, clock-free predicate). After the two allowed renewals
# the phone's earlier signed lists are delivered AGAIN (a replay, as a backdating hostile hub or courier could); every
# terminal skips them as already known, and the session keeps its renewals and its end. A genuinely new third renewal
# is refused (75b). Honest tools cannot sign a backdated op: the replay is the nearest honest-component equivalent.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=75d-renew-backdated

step "ring with the phone, three terminals and a host" trust_ring
step "trust session of peer-c on certhost (30 days)" issue_cert peer-c certhost 30
CERT_ID=$(trust_sessions peer-a | pjq -r '.id')
step "renewal 1" renew_cert peer-a "$CERT_ID" 30
OPS1=$PHONE_OPS
step "renewal 2" renew_cert peer-a "$CERT_ID" 30
SA=$(trust_sessions peer-a); echo "sessions on peer-a: $SA"
expect "renewed twice" "$SA" '.renewals == 2'
EXP2=$(printf '%s' "$SA" | pjq -r '.expiry')

for p in peer-a peer-b peer-c; do
  show "$p is given the first renewal's list again" sh -c "printf '%s\n' '$OPS1' | $C exec -T $p $SSHUB sync-phone-ops --format json"
  expect "$p: nothing new applied" "$LAST_OUT" '.applied == 0'
done
spread peer-a peer-b peer-c peer-a
for p in peer-a peer-b peer-c; do
  S=$(trust_sessions $p)
  expect "$p: still two renewals" "$S" '.renewals == 2'
  check "$p: the end did not move" "[ \"$(printf '%s' "$S" | pjq -r .expiry)\" = \"$EXP2\" ]"
done
finish "$NAME"
