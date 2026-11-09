#!/bin/sh
# 78c-two-handovers: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=78c-two-handovers
skeleton "$NAME" "two handovers of one phone: the lowest completion wins" "C PhoneHandover validity"
