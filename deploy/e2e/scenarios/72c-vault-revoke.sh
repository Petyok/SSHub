#!/bin/sh
# 72c-vault-revoke: SKELETON, passes trivially and prints its gap until the feature lands (design 8).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=72c-vault-revoke
skeleton "$NAME" "Vault ring revoke" "C revoke_mode and quorum (3.5 Vault cell)"
