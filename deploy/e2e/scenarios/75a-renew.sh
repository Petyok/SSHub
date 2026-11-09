#!/bin/sh
# timeout: 2400
# 75a: CertRenew (design 6, contract 4j/4l). A trust session of peer-c on host certhost is issued with the full
# ceremony; peer-a (not the delegate) asks for a renewal: the request carries the opened-hosts block (contract 4l: the
# hosts the holders recorded as opened under the session, newest first, with the total), the phone alone signs it, and
# the effective end moves by one term on every terminal. The delegate's own computer cannot ask for its renewal.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=75a-renew

step "ring with the phone, three terminals and a host" trust_ring
step "trust session of peer-c on certhost (30 days)" issue_cert peer-c certhost 30
S0=$(trust_sessions peer-a)
echo "sessions on peer-a: $S0"
expect "one trust session in force on peer-a" "$S0" '.id != null and .renewals == 0'
CERT_ID=$(printf '%s' "$S0" | pjq -r '.id')
EXP0=$(printf '%s' "$S0" | pjq -r '.expiry')

show "peer-c sync-trust-renew (on the delegate's own computer)" peer peer-c $SSHUB sync-trust-renew "$CERT_ID" --days 30 --format json
check "the delegate's own computer refuses" "[ '$LAST_RC' -ne 0 ]"
check "it says to renew from another full computer" "printf '%s' \"\$LAST_OUT\" | grep -qi 'another full computer'"

step "renewal of the session on peer-a" renew_cert peer-a "$CERT_ID" 30
expect "the request carries the opened-hosts block" "$RENEW_REQ_OUT" 'select(.event=="opened") | (.opened | type) == "array" and (.opened_total | type) == "number"'
expect "the page shows a renewal screen with an approve button" "$RENEW_OPEN" '.ok == true and .can_approve == true and .refusal == null'
expect "the page lists the hosts it opened" "$RENEW_OPEN" 'any(.rows[]; .[0] == "Hosts it opened")'
expect "the page shows the new end" "$RENEW_OPEN" 'any(.rows[]; .[0] == "New end")'

for p in peer-a peer-b peer-c; do
  S=$(trust_sessions $p)
  echo "sessions on $p: $S"
  expect "$p: the session was renewed once" "$S" '.renewals == 1'
  expect "$p: the end moved by the term (30 days)" "$S" ".expiry > $EXP0"
done
S1=$(trust_sessions peer-a)
check "the three terminals agree on the effective end" "[ \"$(trust_sessions peer-b | pjq -r .expiry)\" = \"$(printf '%s' "$S1" | pjq -r .expiry)\" ] && [ \"$(trust_sessions peer-c | pjq -r .expiry)\" = \"$(printf '%s' "$S1" | pjq -r .expiry)\" ]"
finish "$NAME"
