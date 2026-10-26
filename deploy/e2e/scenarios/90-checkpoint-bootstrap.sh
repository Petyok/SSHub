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
check "peer-a sets up the Vault" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-a $SSHUB sync-import-kit"
check "peer-b imports the kit" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-b $SSHUB sync-import-kit"
check "peer-a adds host big with a password" "printf 'pw0\n' | peer peer-a $SSHUB host add --name big --address big.internal --password-stdin"
# Finding P1: `host edit` never appends an op, so the log cannot be grown with host
# edits. One `import` of an ssh config with 5300 hosts appends 5300 HostUpsert ops in a single process
# (a process per edit took far too long), which passes the 5000 op trigger.
grow='mkdir -p "$HOME/.ssh"; i=0
while [ $i -lt 5300 ]; do printf "Host grow%s\n  HostName grow%s.internal\n" $i $i; i=$((i+1)); done > "$HOME/.ssh/config"
sshub --profile default import --from ssh >/dev/null 2>&1'
check "peer-a grows its log past the checkpoint threshold" "psh peer-a '$grow'"
show "peer-a ops in the log" sq peer-a 'SELECT count(*), sum(length(canonical)) FROM sync_ops'
for round in 1 2 3; do
  snowt peer-b
  snowt peer-a
done
show "peer-a checkpoints" sq peer-a 'SELECT status, state_len FROM sync_checkpoints'
show "peer-b checkpoints" sq peer-b 'SELECT status, state_len FROM sync_checkpoints'
show "peer-b op states" sq peer-b 'SELECT state, reason, count(*) FROM sync_ops GROUP BY state, reason'
check "peer-a holds an accepted checkpoint" '[ "$(ckpts_accepted peer-a)" -gt 0 ]'
check "peer-b holds the accepted checkpoint too" '[ "$(ckpts_accepted peer-b)" -gt 0 ]'

echo "--- GAP: prune needs 7 days after acceptance and the driver reads the wall clock; no hook, so no pruned log exists to bootstrap from"
check "peer-c joins after the checkpoint and receives the ring" "enroll_peer peer-c hub"
approve_all peer-a >/dev/null 2>&1 || true
check "peer-c records peer-a" "enroll_peer peer-c peer-a"
snowt peer-c
check "peer-c has the big host after syncing" "peer peer-c $SSHUB host list 2>/dev/null | grep -q big"
dump_state "after checkpoint"

finish "$NAME"
