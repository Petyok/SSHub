#!/bin/sh
# timeout: 1500
# 80: phone recovery. A ring with the phone and three terminal holders (joiners are mirrors, and only attended holders
# count in a recovery). peer-a starts a recovery (the third factor, the paper, is proven later), peer-b and peer-c approve
# and acknowledge it, peer-b cancels with the paper factor, which is PROVEN over the cancel op (the phrase on stdin). The 7-day timelock
# (RECOVERY_TIMELOCK_SECS) is a compile-time constant read from the wall clock:
# the product has no test-clock hook, so the elapsed-timelock branch is a
# FINDING, not something this scenario fakes.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=80-recovery-timelock

ring_with_phone ring3
show "peer-a sync-recovery start" sh -c "printf '\n\n' | $C exec -T peer-a $SSHUB sync-recovery start --format json"
check "peer-a started a recovery" "[ $LAST_RC -eq 0 ]"
OP=$(opid "$LAST_OUT")
spread peer-b peer-c peer-a
for p in peer-b peer-c; do
  show "$p sees the recovery" peer "$p" $SSHUB sync-recovery status --format json
  expect "$p lists one recovery" "$LAST_OUT" '.recoveries | length == 1'
  show "$p approves" peer "$p" $SSHUB sync-recovery approve "$OP" --format json
  check "$p approved" "[ $LAST_RC -eq 0 ]"
  show "$p acknowledges" peer "$p" $SSHUB sync-recovery ack "$OP" --format json
  check "$p acknowledged" "[ $LAST_RC -eq 0 ]"
done
spread peer-a peer-b peer-c peer-a
show "peer-a recovery status" peer peer-a $SSHUB sync-recovery status --format json
expect "the recovery is waiting, not completed (7 days have not passed)" "$LAST_OUT" '.recoveries | length == 1 and (.[0].verdict | test("Completed") | not)'
echo "--- GAP: no test clock. The timelock is RECOVERY_TIMELOCK_SECS = 7 days read from the wall clock; no env var or config moves it, so completion cannot be driven"

show "peer-b cancels with the paper factor" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-b $SSHUB sync-recovery cancel $OP --factor paper --format json"
spread peer-a peer-b peer-c
show "peer-a recovery status after the cancel" peer peer-a $SSHUB sync-recovery status --format json
expect "the recovery is cancelled" "$LAST_OUT" '.recoveries[0].verdict | test("Cancel")'
dump_state "after recovery"

finish "$NAME"
