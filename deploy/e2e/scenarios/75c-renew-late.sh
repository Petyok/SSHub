#!/bin/sh
# timeout: 2400
# 75c: a renewal that reaches a terminal late gives the same effective end (design 6.1: the verdict reads the log, not
# the arrival order or the clock). peer-c is silent while peer-a has the renewal signed (a held delivery); when it
# syncs afterwards, its view of the session equals peer-a's.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=75c-renew-late

step "ring with the phone, three terminals and a host" trust_ring
step "trust session of peer-c on certhost (30 days)" issue_cert peer-c certhost 30
CERT_ID=$(trust_sessions peer-a | pjq -r '.id')
EXP0=$(trust_sessions peer-a | pjq -r '.expiry')

# the renewal without peer-c and peer-b taking part: only peer-a and the phone
show "peer-a sync-trust-renew (the request)" peer peer-a $SSHUB sync-trust-renew "$CERT_ID" --days 30 --format json
show "the phone reads the renewal request" phone renew-open "{\"frame\":\"$(pf "$LAST_OUT" request)\"}"
show "the phone signs the renewal" phone renew-approve
PHONE_OPS=$(pf "$LAST_OUT" ops)
show "peer-a sync-phone-ops takes the renewal in" sh -c "printf '%s\n' '$PHONE_OPS' | $C exec -T peer-a $SSHUB sync-phone-ops --format json"
snow peer-a >/dev/null
SA=$(trust_sessions peer-a); echo "sessions on peer-a: $SA"
expect "peer-a sees the renewal" "$SA" '.renewals == 1'
SC=$(trust_sessions peer-c); echo "sessions on peer-c (before it syncs): $SC"
expect "peer-c has not seen it yet" "$SC" '.renewals == 0'

echo "--- the held delivery arrives: peer-c syncs now"
spread peer-c peer-b peer-c
SC=$(trust_sessions peer-c); echo "sessions on peer-c: $SC"
SB=$(trust_sessions peer-b); echo "sessions on peer-b: $SB"
for p in peer-b peer-c; do
  S=$(trust_sessions $p)
  expect "$p: renewed once" "$S" '.renewals == 1'
  check "$p: the same effective end as peer-a" "[ \"$(printf '%s' "$S" | pjq -r .expiry)\" = \"$(printf '%s' "$SA" | pjq -r .expiry)\" ]"
done
check "the end moved from the original" "[ \"$(printf '%s' "$SA" | pjq -r .expiry)\" -gt \"$EXP0\" ]"
finish "$NAME"
