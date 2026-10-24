#!/bin/sh
# 10: peer-a founds the ring with hub, peer-b joins through hub and is approved
# on peer-a. peer-b then records peer-a directly (a repeat ceremony), so a host
# added on peer-a reaches peer-b. Address changes are held for review (Dangerous
# field, see tier.rs): one held change is approved and must apply, another is
# rejected and must not.
. "$E2E_DIR/lib.sh"
NAME=10-enroll-and-sync

# id of the pending review entry whose change sets address to $1 (empty if none)
rid() {
  peer peer-b $SSHUB sync-review --format json 2>/dev/null \
    | pjq -r --arg v "$1" '[.[] | select(any(.changes[]; .to == $v)) | .id][0] // empty'
}

reset_peers
check "peer-a founds the ring with hub" "enroll_peer peer-a hub"
check "peer-b joins via hub" "enroll_peer peer-b hub"
check "peer-a approves peer-b's admission" "approve_all peer-a"
check "peer-b records peer-a directly" "enroll_peer peer-b peer-a"
dump_state "after enrollment"
check "peer-a adds host web1" "peer peer-a $SSHUB host add --name web1 --address web1.internal"

out=$(snow peer-b)
expect "peer-b sync-now reaches peer-a ok" "$out" '[.[] | select(.name=="peer-a") | .status] == ["ok"]'

st=$(peer peer-b $SSHUB sync-status --format json 2>/dev/null || true)
expect "address change held as pending on peer-b" "$st" '.pending >= 1'
# Raw review queue, so a failing rid lookup shows what peer-b actually reports.
echo "--- peer-b sync-review (plain)"
peer peer-b $SSHUB sync-review 2>&1 || true
echo "--- peer-b sync-review (json)"
peer peer-b $SSHUB sync-review --format json 2>&1 || true
check "web1 address not applied before approval" "! peer peer-b $SSHUB host list 2>/dev/null | grep -q web1.internal"
check "held web1 change is in the review queue" "test -n '$(rid web1.internal)'"
check "approve held web1 change on peer-b" "peer peer-b $SSHUB sync-review --approve $(rid web1.internal)"
check "web1 address applied after approval" "peer peer-b $SSHUB host list 2>/dev/null | grep -q web1.internal"

check "peer-a adds host web2" "peer peer-a $SSHUB host add --name web2 --address web2.internal"
snow peer-b >/dev/null
check "held web2 change is in the review queue" "test -n '$(rid web2.internal)'"
check "reject held web2 change on peer-b" "peer peer-b $SSHUB sync-review --reject $(rid web2.internal)"
check "web2 address absent after rejection" "! peer peer-b $SSHUB host list 2>/dev/null | grep -q web2.internal"
dump_state "after review"

finish "$NAME"
