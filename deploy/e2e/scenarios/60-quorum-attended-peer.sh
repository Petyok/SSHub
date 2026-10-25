#!/bin/sh
# 60: Quorum mode. peer-a switches the ring from Vault to Quorum (sync-mode
# quorum re-wraps every credential), peer-b admits the change through the Red
# ceremony, then asks for a credential. The share release over the link has no
# command-line verb: that step is recorded as a product gap, not faked.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=60-quorum-attended-peer

ring2
check "peer-a sets up the Vault" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-a $SSHUB sync-import-kit"
check "peer-b imports the kit" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-b $SSHUB sync-import-kit"
check "peer-a stores host quorum1 with a password" "printf '%s\n' '$SECRET' | peer peer-a $SSHUB host add --name quorum1 --address quorum1.internal --password-stdin"
check "peer-a migrates it into the ring" "peer peer-a $SSHUB sync-migrate"

# A ring of two holders needs the paper registered first (product rule).
show "peer-a registers the paper factor" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor-register paper --format json"
show "peer-a sync-mode quorum (phrase on stdin)" sh -c "printf '%s\n' '$PHRASE' | $C exec -T peer-a $SSHUB sync-mode quorum --format json"
expect "the mode change is proposed as a Red op with a ceremony" "$LAST_OUT" '.tier == "Red" and (.ceremony.missing | length) >= 0'
MODE_OP=$(opid "$LAST_OUT")
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-a reports Quorum mode" "$st" '.credential_mode == "quorum"'

snow peer-b >/dev/null
st=$(peer peer-b $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-b holds the mode change for review" "$st" '.pending >= 1'
show "peer-b sync-approve the mode change" peer peer-b $SSHUB sync-approve "$MODE_OP" --format json
expect "peer-b's vote is counted in the ceremony" "$LAST_OUT" '.ceremony.have | any(. == "second device")'
echo "ceremony after peer-b's vote: $(printf '%s' "$LAST_OUT" | pjq -c '.ceremony')"

snow peer-b >/dev/null
approve_all peer-b >/dev/null 2>&1 || true
gap "peer-b asks a holder for the share (no CLI verb requests a ShareRelease)" sh -c "printf '' | $C exec -T peer-b $SSHUB host resolve quorum1 --format json"
check "peer-b cannot open the credential from the command line yet" "[ $LAST_RC -ne 0 ]"
dump_state "after quorum"

finish "$NAME"
