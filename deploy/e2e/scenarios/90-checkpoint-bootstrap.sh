#!/bin/sh
# 90: checkpoint. The driver proposes a checkpoint once the log has grown by
# CHECKPOINT_OPS (5000) ops; there is no env or config knob for small limits, so
# this scenario produces them with safe edits through the real CLI. Pruning
# waits CHECKPOINT_PRUNE_DELAY_SECS (7 days) after acceptance and no clock hook
# exists: prune and the bootstrap of a fresh peer from a pruned log are FINDINGS.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=90-checkpoint-bootstrap

ring2
check "peer-a adds host big" "peer peer-a $SSHUB host add --name big --address big.internal"
snow peer-b >/dev/null
approve_all peer-b >/dev/null 2>&1 || true
check "peer-a makes 5100 safe edits" "psh peer-a 'i=0; while [ \$i -lt 5100 ]; do sshub --profile default host edit big --set-notes n\$i >/dev/null 2>&1 || exit 1; i=\$((i+1)); done'"
show "peer-a ops in the log" sq peer-a 'SELECT count(*) FROM sync_ops'
for round in 1 2 3; do
  spread peer-b peer-a peer-b peer-a
done
show "peer-a checkpoints" sq peer-a 'SELECT status, state_len FROM sync_checkpoints'
check "peer-a holds an accepted checkpoint" '[ "$(ckpts_accepted peer-a)" -gt 0 ]'
show "peer-b checkpoints" sq peer-b 'SELECT status FROM sync_checkpoints'

echo "--- GAP: prune needs 7 days after acceptance and the driver reads the wall clock; no hook, so no pruned log exists to bootstrap from"
check "peer-c joins after the checkpoint and receives the ring" "enroll_peer peer-c hub"
approve_all peer-a >/dev/null 2>&1 || true
check "peer-c records peer-a" "enroll_peer peer-c peer-a"
out=$(snow peer-c)
echo "peer-c first round: $out"
check "peer-c has the big host after syncing" "peer peer-c $SSHUB host list 2>/dev/null | grep -q big"
dump_state "after checkpoint"

finish "$NAME"
