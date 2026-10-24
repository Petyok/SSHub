#!/bin/sh
# 20: peer-a and peer-c both enroll to hub. peer-a's change reaches peer-c only
# through the hub: peer-c has no host for peer-a, so that device is skipped.
. "$E2E_DIR/lib.sh"
NAME=20-transitive-via-hub

reset_peers
check "peer-a joins hub's ring" "enroll_peer peer-a hub"
check "peer-c joins hub's ring" "enroll_peer peer-c hub"
check "peer-a adds host web20" "peer peer-a $SSHUB host add --name web20 --address web20.internal"

out=$(snow peer-a)
expect "peer-a sync-now reaches hub ok" "$out" '[.[] | select(.name=="hub") | .status] == ["ok"]'

out=$(snow peer-c)
expect "peer-c sync-now reaches hub ok" "$out" '[.[] | select(.name=="hub") | .status] == ["ok"]'
expect "peer-c has no host for peer-a: skipped" "$out" '[.[] | select(.name=="peer-a") | .status] == ["skipped"]'
expect "web20 arrived on peer-c via hub, held pending" "$out" '[.[] | select(.name=="hub") | .pending] | add >= 1'

finish "$NAME"
