#!/bin/sh
# 78b-thief-handover: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=78b-thief-handover
skeleton "$NAME" "a handover started with email only (a thief) is refused" "C H1 rules (5.3) and PhoneHandover"
