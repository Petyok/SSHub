#!/bin/sh
# timeout: 3000
# 75d: a backdated renewal does not buy more than the caps allow (design 6.1: the verdict reads the log, never the clock
# an op was signed with). The phone's date is set 100 days into the past (clock-set, the page sees an earlier Date, as
# if the user had changed the phone's date) and the two allowed renewals are signed with it. The host accepts them as
# renewals 1 and 2 and the end moves by exactly two terms, no more. A third renewal signed with the backdated clock is
# refused, and so is one signed with the clock back to normal; the end and the count never change again.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=75d-renew-backdated
TERM=$((30 * 86400))
BACK=-8640000

step "ring with the phone, three terminals and a host" trust_ring
step "trust session of peer-c on certhost (30 days)" issue_cert peer-c certhost 30
S0=$(trust_sessions peer-a)
CERT_ID=$(printf '%s' "$S0" | pjq -r '.id')
EXP0=$(printf '%s' "$S0" | pjq -r '.expiry')

show "the phone's date moves 100 days back" phone clock-set "{\"offset_secs\":$BACK}"
check "the page sees the earlier date" "[ \"$(pf "$LAST_OUT" page_now)\" -lt \"$(( $(date +%s) - 8000000 ))\" ]"
step "renewal 1 signed with the backdated clock" renew_cert peer-a "$CERT_ID" 30
echo "page answer: $RENEW_OPEN"
step "renewal 2 signed with the backdated clock" renew_cert peer-a "$CERT_ID" 30
for p in peer-a peer-b peer-c; do
  S=$(trust_sessions $p); echo "sessions on $p: $S"
  expect "$p: two renewals" "$S" '.renewals == 2'
  check "$p: the end moved by exactly two terms, not more" "[ \"$(printf '%s' "$S" | pjq -r .expiry)\" = \"$((EXP0 + 2 * TERM))\" ]"
done
EXP2=$((EXP0 + 2 * TERM))

echo "--- a third renewal, still backdated"
show "peer-a sync-trust-renew (the third request)" peer peer-a $SSHUB sync-trust-renew "$CERT_ID" --days 30 --format json
if [ "$LAST_RC" -eq 0 ]; then
  show "the backdated phone reads the third request" phone renew-open "{\"frame\":\"$(pf "$LAST_OUT" request)\"}"
  expect "the page refuses it (no approve button)" "$LAST_OUT" '(.ok == false) or (.can_approve == false and .refusal != null)'
else
  echo "the terminal refused the third request: $LAST_OUT"
fi
show "the phone's date is restored" phone clock-set '{"offset_secs":0}'
echo "--- a third renewal with the normal clock"
show "peer-a sync-trust-renew (the third request)" peer peer-a $SSHUB sync-trust-renew "$CERT_ID" --days 30 --format json
if [ "$LAST_RC" -eq 0 ]; then
  show "the phone reads the third request" phone renew-open "{\"frame\":\"$(pf "$LAST_OUT" request)\"}"
  expect "the page refuses it (no approve button)" "$LAST_OUT" '(.ok == false) or (.can_approve == false and .refusal != null)'
else
  echo "the terminal refused the third request: $LAST_OUT"
fi
spread peer-a peer-b peer-c peer-a
for p in peer-a peer-b peer-c; do
  S=$(trust_sessions $p)
  expect "$p: still two renewals" "$S" '.renewals == 2'
  check "$p: the end did not move" "[ \"$(printf '%s' "$S" | pjq -r .expiry)\" = \"$EXP2\" ]"
done
finish "$NAME"
