#!/bin/sh
# 82-alert: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=82-alert
skeleton "$NAME" "sync-status exit code 3 and the CLI alert while a recovery is open" "T alert surfaces and C open_for_alert"
