#!/bin/sh
# 20: peer-a founds the ring with hub; peer-b and peer-c join through hub and are
# approved on peer-a. Nobody records another peer directly, so peer-a's change
# reaches peer-c only through the hub. peer-c has no host for peer-a: that device
# must be listed as skipped (no host known), and the change arrives via the hub.
. "$E2E_DIR/lib.sh"
NAME=20-transitive-via-hub

reset_peers
check "peer-a founds the ring with hub" "enroll_peer peer-a hub"
check "peer-b joins via hub" "enroll_peer peer-b hub"
check "peer-c joins via hub" "enroll_peer peer-c hub"
check "peer-a approves admissions of peer-b and peer-c" "approve_all peer-a"
check "peer-a adds host web20" "peer peer-a $SSHUB host add --name web20 --address web20.internal"
dump_state "after enrollment and host add"

out=$(snow peer-a)
expect "peer-a sync-now reaches hub ok" "$out" '[.[] | select(.name=="hub") | .status] == ["ok"]'

out=$(snow peer-c)
expect "peer-c sync-now reaches hub ok" "$out" '[.[] | select(.name=="hub") | .status] == ["ok"]'
expect "peer-c has no host for peer-a: skipped" "$out" '[.[] | select(.name=="peer-a") | .status] == ["skipped"]'
expect "web20 arrived on peer-c via hub, held pending" "$out" '[.[] | select(.name=="hub") | .pending] | add >= 1'
dump_state "after peer-c sync"

finish "$NAME"
