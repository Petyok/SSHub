#!/bin/sh
# 72f-no-paper-refusal: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=72f-no-paper-refusal
skeleton "$NAME" "a revoke without paper is refused with the reason" "C paper rule (3.1) and revoke_mode Refused"
