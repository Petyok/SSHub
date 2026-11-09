#!/bin/sh
# 74-replace-terminal: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=74-replace-terminal
skeleton "$NAME" "replace a terminal in one guided flow" "C revoke_mode and T sync-replace-terminal (3.3)"
