#!/bin/sh
# timeout: 3600
# 78d: a hostile recovery started BEFORE a handover cannot revoke the new phone (design 5.3, H1 rule: an in-force
# PhoneHandover supersedes every PhoneReenroll with a lower lamport). Ring of three terminals and the old phone, with the
# emailed-code factor on the Hub. peer-b (the hostile one, holding the paper and the code) starts a recovery; then the
# real handover completes through peer-a and peer-c; the recovery's verdict is Superseded and the new phone stays.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=78d-hostile-recovery

check "the second phone daemon answers" as_phone2 phone_ready
step "the second phone restores its own key" as_phone2 phone_prep_page
step "ring with the old phone (ring3)" ring_with_phone ring3
show "hub registers the emailed-code factor (it is the sender)" peer hub $SSHUB sync-factor-register email --format json
CODE=$(pf "$LAST_OUT" code)
check "the Hub printed its code" "test -n '$CODE'"
spread hub peer-a peer-b peer-c peer-a
step "email factor ceremony (phone, peer-b, paper)" red_complete "$(opid "$LAST_OUT")"
spread peer-a peer-b peer-c hub peer-a

show "peer-b (hostile) sync-recovery start --paper" sh -c "printf '%s\n%s\n' '$PAPER' '$CODE' | $C exec -T peer-b $SSHUB sync-recovery start --paper --format json"
check "the hostile recovery is started" "[ '$LAST_RC' -eq 0 ]"
RHOST=$(opid "$LAST_OUT")
spread peer-b peer-a peer-c peer-b

HO_SECOND=peer-c
step "handover: both phones signed" handover_start
show "peer-c sync-approve the handover (the second terminal)" peer peer-c $SSHUB sync-approve "$HO_OP" --format json
spread peer-a peer-c peer-a
show "peer-a adds the paper factor" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor $HO_OP --paper --format json"
expect "the handover ceremony is complete" "$LAST_OUT" '.ceremony.complete == true'
spread peer-b peer-c peer-a peer-b peer-c

for p in peer-a peer-b peer-c; do
  show "$p sync-recovery status" peer $p $SSHUB sync-recovery status --format json
  expect "$p: the hostile recovery is superseded by the handover" "$LAST_OUT" "any(.recoveries[]; (.op | startswith(\"$(printf '%s' "$RHOST" | cut -c1-8)\")) and (.verdict | test(\"Superseded\")))"
  st=$(peer $p $SSHUB sync-status --format json 2>/dev/null || true)
  expect "$p: the new phone is a full device" "$st" '[.devices[] | select(.name=="new phone") | .role] == ["holder"]'
done
finish "$NAME"
