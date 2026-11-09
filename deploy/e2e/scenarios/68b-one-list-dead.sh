#!/bin/sh
# timeout: 2400
# 68b: two terminals, one dead (design 7: "a dead or away terminal never blocks the restore"). peer-a and peer-b are
# full devices; peer-b never answers after the setup. The restored phone takes ONE list from peer-a: the card says how
# many terminals the ring has and that only this one could be reached; with the tick it adopts. The Hub is removed
# first so the count is exactly two.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=68b-one-list-dead

phone_prep_page
ring2
enroll_phone peer-a
phone_signs peer-a "$ADD"
show "peer-a sync-revoke hub" peer peer-a $SSHUB sync-revoke "$(devid peer-a hub)" --format json
check "the Hub removal is proposed" "[ '$LAST_RC' -eq 0 ]"
if [ "$(pf "$LAST_OUT" tier)" = Red ]; then step "removal ceremony of the Hub (phone, paper)" red_phone_paper "$(opid "$LAST_OUT")"; fi
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "the Hub is removed" "$st" '[.devices[] | select(.name=="hub") | .status] == ["revoked"]'
TERMS=$(printf '%s' "$st" | pjq '[.devices[] | select(.name != "my phone" and .status != "revoked")] | length')
echo "terminals in the ring: $TERMS"
check "two terminals" "[ '$TERMS' = 2 ]"
echo "--- peer-b goes silent: nothing below talks to it"

restore_phone
check "peer-a reports two terminals in the list" "[ \"$(chain_terminals peer-a)\" = 2 ]"
show "the phone takes the list from peer-a alone" phone chain-take "{\"frame\":\"$(chain_of peer-a)\"}"
expect "one list is enough, with the tick" "$LAST_OUT" '.ok == true and .agreed == true and .needs_tick == true'
expect "the card names two terminals and that only one could be reached" "$LAST_OUT" '(.card | test("2 terminals")) and (.card | test("only this one"))'
show "the phone adopts with the tick" phone chain-adopt '{"tick":true}'
expect "the chain is taken" "$LAST_OUT" '.ok == true'
finish "$NAME"
