#!/bin/sh
# timeout: 2400
# 72c: a Vault ring (no Quorum credentials). peer-a removes peer-c in the full mode (two other terminals remain): the
# ceremony is phone, a second terminal (peer-b) and paper, nothing is exposed (a Vault credential is not split among
# devices), and the removed device is refused by the ring.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=72c-vault-revoke

step "ring with the phone (ring3)" ring_with_phone ring3
check "peer-a sets up the Vault" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-a $SSHUB sync-import-kit"
check "peer-b imports the kit" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-b $SSHUB sync-import-kit"
check "peer-a stores host vaultrev with a password" "printf '%s\n' '$SECRET' | peer peer-a $SSHUB host add --name vaultrev --address vaultrev.internal --password-stdin"
spread peer-b peer-c peer-a peer-b
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "the ring is in Vault mode" "$st" '.credential_mode == "vault"'
C_ID=$(devid peer-a peer-c)
check "peer-a can name peer-c" "test -n '$C_ID'"
show "peer-a sync-revoke peer-c (full: two other terminals)" peer peer-a $SSHUB sync-revoke "$C_ID" --format json
expect "it is a Red change with a ceremony" "$LAST_OUT" '.tier == "Red" and (.ceremony != null)'
REV=$(opid "$LAST_OUT")
step "removal ceremony (phone, peer-b, paper)" red_complete "$REV"
expect "the removal ceremony is complete" "$LAST_OUT" '.ceremony.complete == true'
spread peer-b peer-a peer-b
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-c is removed on peer-a" "$st" '[.devices[] | select(.name=="peer-c") | .status] == ["revoked"]'
expect "nothing is exposed in a Vault ring" "$st" '(.exposure | length) == 0'
show "peer-c after the removal: a round with peer-a" peer peer-c $SSHUB sync-now --format json
check "the removed device is refused" "printf '%s' '$LAST_OUT' | grep -q Revoked"
check "the Vault credential still opens on peer-b" "printf '%s\n' '$PHRASE' | peer peer-b $SSHUB host resolve vaultrev --format json | grep -q ."
ring_view
finish "$NAME"
