#!/bin/sh
# timeout: 1800
# 82: the recovery alert (design U13-7). A recovery started on one terminal shows on the others: `sync-status` prints
# "RECOVERY STARTED" lines first, lists `recovery_alerts` in JSON and EXITS 3 while it is open. The alert has no
# acknowledge key: reading the status twice gives the same exit. A cancelled recovery closes it (exit 0 again).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=82-alert

step "ring with the phone (ring3)" ring_with_phone ring3
for p in peer-b peer-c; do
  show "$p sync-status before any recovery" peer "$p" $SSHUB sync-status
  check "$p exits 0 without a recovery" "[ '$LAST_RC' -eq 0 ]"
done

show "peer-a sync-recovery start" sh -c "printf '\n\n' | $C exec -T peer-a $SSHUB sync-recovery start --format json"
check "peer-a started a recovery" "[ '$LAST_RC' -eq 0 ]"
OP=$(opid "$LAST_OUT")
spread peer-b peer-c peer-a peer-b peer-c
for p in peer-b peer-c; do
  show "$p sync-status (plain) with the recovery open" peer "$p" $SSHUB sync-status
  check "$p exits 3 while a recovery started elsewhere is open" "[ '$LAST_RC' -eq 3 ]"
  check "$p prints RECOVERY STARTED first" "[ \"\$(printf '%s' \"\$LAST_OUT\" | sed -n 1p | cut -c1-17)\" = 'RECOVERY STARTED' ]"
  show "$p sync-status (json)" peer "$p" $SSHUB sync-status --format json
  check "$p json exits 3 too" "[ '$LAST_RC' -eq 3 ]"
  expect "$p lists the open recovery" "$LAST_OUT" '(.recovery_alerts | length) >= 1 and .alert.open >= 1'
  show "$p sync-status again (no acknowledge key)" peer "$p" $SSHUB sync-status
  check "$p still exits 3" "[ '$LAST_RC' -eq 3 ]"
done

show "peer-b cancels with the paper factor" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-b $SSHUB sync-recovery cancel $OP --factor paper --format json"
spread peer-a peer-b peer-c peer-a
for p in peer-b peer-c; do
  show "$p sync-status after the cancel" peer "$p" $SSHUB sync-status --format json
  check "$p exits 0 once the recovery is cancelled" "[ '$LAST_RC' -eq 0 ]"
done
finish "$NAME"
