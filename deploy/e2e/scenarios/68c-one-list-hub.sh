#!/bin/sh
# 68c-one-list-hub: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=68c-one-list-hub
skeleton "$NAME" "68c laptop plus Hub: the Hub counts as a terminal for the one-list rule" "PH-core U13-5 and C's terminals block fill"
