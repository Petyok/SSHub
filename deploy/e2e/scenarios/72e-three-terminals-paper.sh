#!/bin/sh
# 72e-three-terminals-paper: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=72e-three-terminals-paper
skeleton "$NAME" "three terminals, the second away: phone plus paper revoke" "C revoke_mode and the paper rule (3.5)"
