#!/bin/sh
# 78a-handover: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=78a-handover
skeleton "$NAME" "PhoneHandover: the old phone admits the new phone and retires" "P PhoneHandover op, C handover.rs, PH-core, and the phone2 service (done)"
