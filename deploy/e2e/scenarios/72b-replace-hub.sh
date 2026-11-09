#!/bin/sh
# 72b-replace-hub: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=72b-replace-hub
skeleton "$NAME" "one terminal plus Hub: the replace-terminal flow" "C revoke_mode and T sync-replace-terminal"
