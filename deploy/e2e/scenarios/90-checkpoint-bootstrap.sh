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
show "peer-a op kinds before a password edit" sq peer-a 'SELECT kind, count(*) FROM sync_ops GROUP BY kind'
show "peer-a edits the password of big (T1: one CredUpdate expected)" sh -c "printf 'pw1\n' | $C exec -T peer-a $SSHUB host edit --name big --set-password-stdin"
show "peer-a op kinds after the password edit" sq peer-a 'SELECT kind, count(*) FROM sync_ops GROUP BY kind'
# Finding P1 (settled): a password edit appends no op and the remote command of a host does not travel in its op (an op is
# about 350 bytes), so only the op trigger (5000) can be reached: 8 workers add 650 hosts each (one HostUpsert op per
# add, about 0.4 s a process), a progress line per 100 adds, so a silent log means a hang.
grow='w() { i=0; while [ $i -lt 650 ]; do sshub --profile default host add --name g$1_$i --address a$i.internal >/dev/null 2>&1 || echo "worker $1: add $i failed"; i=$((i+1)); [ $((i % 100)) -eq 0 ] && echo "worker $1: $i adds"; done; }
for k in 1 2 3 4 5 6 7 8; do w $k & done; wait'
echo "--- growing peer-a's log: 8 workers x 650 host adds"
psh peer-a "$grow" 2>&1 | sed 's/^/    | /'
check "peer-a's log passed the checkpoint op threshold (5000)" '[ "$(sq peer-a "SELECT count(*) FROM sync_ops")" -ge 5000 ]'
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
