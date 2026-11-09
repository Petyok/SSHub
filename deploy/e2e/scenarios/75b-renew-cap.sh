#!/bin/sh
# timeout: 2700
# 75b: the renewal caps (CERT_MAX_RENEWALS = 2). Two renewals of one trust session go through; the third is refused,
# by the phone page (its own refusal text, no approve button) and the end does not move.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=75b-renew-cap

step "ring with the phone, three terminals and a host" trust_ring
step "trust session of peer-c on certhost (30 days)" issue_cert peer-c certhost 30
CERT_ID=$(trust_sessions peer-a | pjq -r '.id')
for n in 1 2; do
  step "renewal $n" renew_cert peer-a "$CERT_ID" 30
  S=$(trust_sessions peer-a); echo "sessions on peer-a: $S"
  expect "renewed $n time(s)" "$S" ".renewals == $n"
done
EXP2=$(trust_sessions peer-a | pjq -r '.expiry')

show "peer-a sync-trust-renew (the third request)" peer peer-a $SSHUB sync-trust-renew "$CERT_ID" --days 30 --format json
REQ3=$LAST_OUT
if [ "$LAST_RC" -eq 0 ]; then
  show "the phone reads the third renewal request" phone renew-open "{\"frame\":\"$(pf "$REQ3" request)\"}"
  expect "the page refuses (no approve button, a refusal text)" "$LAST_OUT" '(.ok == false) or (.can_approve == false and .refusal != null)'
  echo "page refusal: $(pf "$LAST_OUT" refusal)"
else
  echo "the terminal itself refused the third request: $REQ3"
fi
S=$(trust_sessions peer-a)
expect "still renewed twice" "$S" '.renewals == 2'
check "the end did not move" "[ \"$(printf '%s' "$S" | pjq -r .expiry)\" = \"$EXP2\" ]"
finish "$NAME"
