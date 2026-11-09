#!/bin/sh
# 75b-renew-cap: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=75b-renew-cap
skeleton "$NAME" "the third renewal of a certificate is refused" "P CertRenew op and C validity (CERT_MAX_RENEWALS)"
