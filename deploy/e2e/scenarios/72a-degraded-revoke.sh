#!/bin/sh
# 72a-degraded-revoke: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=72a-degraded-revoke
skeleton "$NAME" "two laptops plus phone: degraded revoke with a marker; a new credential is not wrapped to the removed device" "C revoke_mode Degraded (3.2)"
