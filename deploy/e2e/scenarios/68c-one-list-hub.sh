#!/bin/sh
# timeout: 2400
# 68c: a laptop plus the Hub (design 7: "A Hub counts in n"). The Hub is a terminal for the count, so the ring has two
# terminals: the first list (peer-a) needs the tick and says "2 terminals"; a second list from the Hub names the same
# newest op from another sender, which upgrades the proposal to the two-list default: no tick any more.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=68c-one-list-hub

step "ring of peer-a, the Hub and the phone; the phone restored" phone_ring_then_restore
snow peer-a >/dev/null
snow hub >/dev/null
check "peer-a reports two terminals (the Hub counts)" "[ \"$(chain_terminals peer-a)\" = 2 ]"
show "the phone takes peer-a's list" phone chain-take "{\"frame\":\"$(chain_of peer-a)\"}"
expect "the card says two terminals and asks for the tick" "$LAST_OUT" '.ok == true and .needs_tick == true and (.card | test("2 terminals"))'
show "the phone takes the Hub's list as the second source" phone chain-take "{\"frame\":\"$(chain_of hub)\"}"
expect "two lists agree and no tick is asked for" "$LAST_OUT" '.ok == true and .agreed == true and .needs_tick == false'
show "the phone adopts" phone chain-adopt
expect "the chain is taken" "$LAST_OUT" '.ok == true'
finish "$NAME"
