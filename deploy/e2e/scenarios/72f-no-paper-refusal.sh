#!/bin/sh
# timeout: 1200
# 72f: removing a full computer without a registered paper is refused, and the sentence says why (design 3.2: the
# revoke screen names the paper). Two computers and no phone, so the paper is not needed to have two full devices.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=72f-no-paper-refusal

ring2
B_ID=$(devid peer-a peer-b)
show "peer-a sync-role peer-b holder" peer peer-a $SSHUB sync-role "$B_ID" holder --format json
ROLEOP=$(opid "$LAST_OUT")
if [ -n "$ROLEOP" ]; then
  spread peer-b peer-a peer-b
  show "peer-b sync-approve the promotion" peer peer-b $SSHUB sync-approve "$ROLEOP" --format json
  spread peer-a peer-b peer-a
fi
check "two full devices" "[ \"$(holders peer-a)\" -ge 2 ]"
show "peer-a sync-revoke peer-b (no paper registered)" peer peer-a $SSHUB sync-revoke "$B_ID" --format json
check "the removal is refused" "[ '$LAST_RC' -ne 0 ]"
check "the refusal names the paper phrase and why" "printf '%s' '$LAST_OUT' | grep -qi 'needs the paper phrase registered'"
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-b is still in the ring" "$st" '[.devices[] | select(.name=="peer-b") | .status] != ["revoked"]'
finish "$NAME"
