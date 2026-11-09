#!/bin/sh
# 71-paper-gate: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=71-paper-gate
skeleton "$NAME" "Quorum switch and third holder refuse without paper; the wizard registers it; the revoke screen names it" "C paper rule (3.1) and T paper wizard"
