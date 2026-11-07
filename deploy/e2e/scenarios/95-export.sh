#!/bin/sh
# 95: bulk export, with the phone. `sync-export` proposes a BulkExport (Red) and writes nothing until its ceremony is
# complete: the phone signs the notice, peer-b votes, the paper signs. Then the window opens and the CSV holds the
# stored password.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=95-export
OUT=/tmp/e2e-export/L00t.csv

ring_with_phone ring2
check "peer-a sets up the Vault" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-a $SSHUB sync-import-kit"
check "peer-b imports the kit" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-b $SSHUB sync-import-kit"
check "peer-a stores host exp1 with a password" "printf '%s\n' '$SECRET' | peer peer-a $SSHUB host add --name exp1 --address exp1.internal --password-stdin"
check "peer-a migrates it into the ring" "peer peer-a $SSHUB sync-migrate"
psh peer-a 'rm -rf /tmp/e2e-export && mkdir -p /tmp/e2e-export'

show "outside the window: sync-export with no ceremony" sh -c "printf '%s\n' '$PHRASE' | $C exec -T peer-a $SSHUB sync-export --window 120 --out $OUT"
check "the export is refused while the ceremony is incomplete" "[ $LAST_RC -ne 0 ] && printf '%s' '$LAST_OUT' | grep -q 'ceremony is not complete'"
check "nothing was written outside the window" "! psh peer-a 'test -e $OUT'"

BULK=$(lastop peer-a BulkExport)
echo "BulkExport op on peer-a: ${BULK:-none}"
check "the BulkExport op exists" "test -n '$BULK'"
red_complete "$BULK"
expect "the export ceremony is complete" "$LAST_OUT" '.ceremony.complete == true'
show "inside the window: sync-export again" sh -c "printf '%s\n' '$PHRASE' | $C exec -T peer-a $SSHUB sync-export --window 120 --out $OUT --format json"
check "the export succeeded" "[ $LAST_RC -eq 0 ]"
check "the CSV exists with mode 0600" "psh peer-a 'test \"\$(stat -c %a $OUT)\" = 600'"
check "the CSV holds the stored password" "psh peer-a 'grep -qF \"$SECRET\" $OUT'"
check "an existing file is never overwritten" "! printf '%s\n' '$PHRASE' | peer peer-a $SSHUB sync-export --window 120 --out $OUT"
dump_state "after export"

finish "$NAME"
