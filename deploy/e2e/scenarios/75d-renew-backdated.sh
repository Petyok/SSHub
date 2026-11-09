#!/bin/sh
# 75d-renew-backdated: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=75d-renew-backdated
skeleton "$NAME" "a backdated renewal does not revive a certificate beyond the caps" "P CertRenew op and C validity (clock-free predicate 6.1)"
