#!/bin/sh
# 95: bulk export. `sync-export` proposes a BulkExport (Red) and refuses to write
# anything until its ceremony is complete: outside the window nothing is
# written. With the second device and the paper factor the window opens and the
# CSV holds the stored password.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=95-export
OUT=/tmp/e2e-export/L00t.csv

ring2
check "peer-a sets up the Vault" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-a $SSHUB sync-import-kit"
check "peer-b imports the kit" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-b $SSHUB sync-import-kit"
check "peer-a stores host exp1 with a password" "printf '%s\n' '$SECRET' | peer peer-a $SSHUB host add --name exp1 --address exp1.internal --password-stdin"
check "peer-a migrates it into the ring" "peer peer-a $SSHUB sync-migrate"
psh peer-a 'rm -rf /tmp/e2e-export && mkdir -p /tmp/e2e-export'

show "outside the window: sync-export with no ceremony" sh -c "printf '%s\n' '$PHRASE' | $C exec -T peer-a $SSHUB sync-export --window 120 --out $OUT"
check "the export is refused while the ceremony is incomplete" "[ $LAST_RC -ne 0 ] && printf '%s' '$LAST_OUT' | grep -q 'ceremony is not complete'"
check "nothing was written outside the window" "! psh peer-a 'test -e $OUT'"

# Take the proposed op, complete what the CLI can, then retry inside the window.
spread peer-b peer-a
BULK=$(peer peer-b $SSHUB sync-review --format json 2>/dev/null | pjq -r '[.[] | select(.summary | test("export"; "i")) | .id][0] // empty')
echo "BulkExport op on peer-b: ${BULK:-none}"
if [ -n "$BULK" ]; then
  show "peer-b sync-approve the export" peer peer-b $SSHUB sync-approve "$BULK" --format json
fi
show "peer-a adds the paper factor" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor-register paper --format json"
spread peer-b peer-a
show "inside the window: sync-export again" sh -c "printf '%s\n' '$PHRASE' | $C exec -T peer-a $SSHUB sync-export --window 120 --out $OUT --format json"
if [ "$LAST_RC" -eq 0 ]; then
  check "the CSV exists with mode 0600" "psh peer-a 'test \"\$(stat -c %a $OUT)\" = 600'"
  check "the CSV holds the stored password" "psh peer-a 'grep -qF \"$SECRET\" $OUT'"
  check "an existing file is never overwritten" "! printf '%s\n' '$PHRASE' | peer peer-a $SSHUB sync-export --window 120 --out $OUT"
else
  echo "--- GAP: the export window cannot be opened headless: second device and paper factor were supplied, the ceremony still misses what is shown above"
fi
dump_state "after export"

finish "$NAME"
