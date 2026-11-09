#!/bin/sh
# 75c-renew-late: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=75c-renew-late
skeleton "$NAME" "a renewal received late on a second device (a held delivery) gives the same effective expiry" "P CertRenew op and C cert.rs effective_expiry"
