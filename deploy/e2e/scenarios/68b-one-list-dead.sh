#!/bin/sh
# 68b-one-list-dead: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=68b-one-list-dead
skeleton "$NAME" "68b two terminals, one dead: the phone accepts one list only when the second terminal is silent" "PH-core U13-5 and C's terminals block fill (hub::phone_chain_wire)"
