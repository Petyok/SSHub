#!/bin/sh
# 75a-renew: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=75a-renew
skeleton "$NAME" "CertRenew: a delegate's certificate is renewed and the expiry moves" "P CertRenew op, C cert_renew_request, PH-core build_cert_renew"
