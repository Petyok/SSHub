#!/bin/sh
# timeout: 2400
# 68a: a restored phone in a ring with ONE terminal accepts one list, with the risk stated (design 7, U13-5). The ring
# is peer-a alone with the phone (the Hub, the founder's partner, is removed again). The terminal reports its terminal
# count in the list; the page asks for a tick before it adopts, says "Your ring has one terminal", and then the phone
# signs a change that peer-a ingests without a fork alarm.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=68a-one-list-terminal

step "ring of peer-a, the Hub and the phone; the phone restored" phone_ring_then_restore
H_ID=$(devid peer-a hub)
show "peer-a sync-revoke hub (peer-a is the only terminal left)" peer peer-a $SSHUB sync-revoke "$H_ID" --format json
check "the Hub is removed" "[ '$LAST_RC' -eq 0 ]"
if [ "$(pf "$LAST_OUT" tier)" = Red ]; then step "removal ceremony of the Hub" red_phone_paper "$(opid "$LAST_OUT")"; fi
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "the Hub is removed on peer-a" "$st" '[.devices[] | select(.name=="hub") | .status] == ["revoked"]'

restore_phone
check "peer-a reports one terminal in the list" "[ \"$(chain_terminals peer-a)\" = 1 ]"
CH=$(chain_of peer-a)
check "peer-a shows a list" "test -n '$CH'"
show "the phone takes the list" phone chain-take "{\"frame\":\"$CH\"}"
expect "one list agrees at once and the page asks for the tick" "$LAST_OUT" '.ok == true and .agreed == true and .needs_tick == true'
expect "the card says the ring has one terminal and states the risk" "$LAST_OUT" '(.card | test("one terminal")) and (.card | test("Risk"))'
show "the phone adopts without the tick" phone chain-adopt '{"tick":false}'
check "without the tick the page refuses" "[ \"$(pf "$LAST_OUT" ok)\" = false ]"
show "the phone adopts with the tick" phone chain-adopt '{"tick":true}'
expect "with the tick the chain is taken" "$LAST_OUT" '.ok == true'

show "peer-a registers the paper (the phone's first signature on the continued chain)" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor-register paper --format json"
phone_signs peer-a "$(opid "$LAST_OUT")"
expect "peer-a ingested the phone's approval" "$LAST_OUT" '.applied >= 1'
show "peer-a sync-status" peer peer-a $SSHUB sync-status
check "no fork alarm" "! printf '%s' \"\$LAST_OUT\" | grep -qi 'fork'"
finish "$NAME"
