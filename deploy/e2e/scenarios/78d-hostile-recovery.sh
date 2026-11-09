#!/bin/sh
# 78d-hostile-recovery: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=78d-hostile-recovery
skeleton "$NAME" "a hostile recovery started before a handover cannot revoke the new phone" "C H1 rules (5.3), PhoneReenroll Superseded"
