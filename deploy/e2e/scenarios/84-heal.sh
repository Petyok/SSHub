#!/bin/sh
# 84-heal: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=84-heal
skeleton "$NAME" "heal on use leaves a credential openable by the new terminal" "C heal_plan and heal_credential, T sync-heal"
