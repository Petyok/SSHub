#!/bin/sh
# 10: peer-b enrolls to peer-a; a host added on peer-a reaches peer-b, where
# an address change is held for review (Dangerous field, see tier.rs).
. "$E2E_DIR/lib.sh"
NAME=10-enroll-and-sync

reset_peers
check "peer-b joins peer-a's ring" "enroll_peer peer-b peer-a"
check "peer-a adds host web1" "peer peer-a $SSHUB host add --name web1 --address web1.internal"

out=$(snow peer-b)
expect "peer-b sync-now reaches peer-a ok" "$out" '[.[] | select(.name=="peer-a") | .status] == ["ok"]'

st=$(peer peer-b $SSHUB sync-status --format json 2>/dev/null || true)
expect "address change held as pending on peer-b" "$st" '.pending >= 1'

finish "$NAME"
