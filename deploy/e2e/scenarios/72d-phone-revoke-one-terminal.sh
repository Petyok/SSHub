#!/bin/sh
# 72d-phone-revoke-one-terminal: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=72d-phone-revoke-one-terminal
skeleton "$NAME" "phone-revoke completion in a one-terminal ring" "C revoke_mode and the phone proof (3.5)"
