#!/bin/sh
# 85-rekey: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=85-rekey
skeleton "$NAME" "after the exposed-credentials change the exposed list is empty" "C exposed_credentials, T sync-rekey"
