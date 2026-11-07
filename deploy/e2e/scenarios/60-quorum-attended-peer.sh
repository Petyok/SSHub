#!/bin/sh
# timeout: 1800
# 60: Quorum mode with a phone. The ring is two attended holders plus the phone. peer-a switches the ring from Vault
# to Quorum (a Red change: phone, peer-b, paper), then the credential is opened twice: by peer-b asking the attended
# holder peer-a (sync-review --approve), and by peer-a asking the phone (sync-share-request: kind 1 out, kind 2 in).
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=60-quorum-attended-peer

step "ring with the phone (ring2)" ring_with_phone ring2
check "peer-a sets up the Vault" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-a $SSHUB sync-import-kit"
check "peer-b imports the kit" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-b $SSHUB sync-import-kit"
check "peer-a stores host quorum1 with a password" "printf '%s\n' '$SECRET' | peer peer-a $SSHUB host add --name quorum1 --address quorum1.internal --password-stdin"
check "peer-a migrates it into the ring" "peer peer-a $SSHUB sync-migrate"

show "peer-a sync-mode quorum, first call proposes it (phrase on stdin)" sh -c "printf '%s\n' '$PHRASE' | $C exec -T peer-a $SSHUB sync-mode quorum --format json"
expect "the mode change is a Red op with a ceremony" "$LAST_OUT" '.tier == "Red" and (.ceremony != null)'
MODE_OP=$(opid "$LAST_OUT")
step "mode change ceremony (phone, peer-b, paper)" red_complete "$MODE_OP"
show "peer-a sync-mode quorum again: the ring agreed, credentials are re-encrypted" sh -c "printf '%s\n' '$PHRASE' | $C exec -T peer-a $SSHUB sync-mode quorum --format json"
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-a reports Quorum mode" "$st" '.credential_mode == "quorum"'
spread peer-b peer-a peer-b peer-a
st=$(peer peer-b $SSHUB sync-status --format json 2>/dev/null || true)
echo "peer-b reports mode: $(printf '%s' "$st" | pjq -r .credential_mode)"
approve_all peer-b >/dev/null 2>&1 || true
spread peer-b peer-a peer-b
echo "peer-b op states (kind|state|reason): $(sq peer-b "SELECT kind||'|'||state||'|'||coalesce(reason,'') FROM sync_ops WHERE state<>'applied'" | tr '\n' ';')"

ring_view

# ---- the attended peer: peer-b asks, peer-a approves on the command line
psh peer-b 'rm -f /tmp/e2e-resolve.out'
$C exec -T peer-b env SSHUB_UNLOCK_WAIT=150 $SSHUB host resolve quorum1 --format json > /tmp/e2e-resolve-b.out 2>&1 &
RPID=$!
REL=""
i=0
while [ -z "$REL" ] && [ "$i" -lt 40 ]; do
  i=$((i + 1)); sleep 3
  snow peer-a >/dev/null
  REL=$(peer peer-a $SSHUB sync-review --format json 2>/dev/null | pjq -r '[.[] | select(.summary | test("share|release"; "i")) | .id][0] // empty')
done
echo "ShareRelease seen by peer-a: ${REL:-none}"
check "peer-a received peer-b's share request" "test -n '$REL'"
if [ -n "$REL" ]; then show "peer-a sync-review --approve the share release" peer peer-a $SSHUB sync-review --approve "$REL"; fi
wait $RPID; RC=$?
echo "--- peer-b host resolve quorum1 (exit $RC):"; sed 's/^/    | /' /tmp/e2e-resolve-b.out
check "peer-b's resolve ended with the credential open" "[ $RC -eq 0 ]"
if grep -qF "$SECRET" /tmp/e2e-resolve-b.out; then echo "note: resolve printed the password"; else echo "note: resolve printed no password"; fi

ring_view

# ---- the phone: peer-a asks for the phone's share (kind 1 out, kind 2 reply in, one process)
enr_open peer-a sync-share-request quorum1 --wait 400 --format json
# sync-share-request prints three events before it waits: host (name and id), words, challenge; the reply makes the fourth
CH=$(enr_get 3); echo "terminal: $CH" | cut -c1-200
show "the phone reads the challenge" phone share-open "{\"frame\":\"$(pf "$CH" challenge)\"}"
if [ "$(pf "$LAST_OUT" ok)" = true ]; then
  echo "ok   the page shows a share release"
  show "the phone approves the release" phone share-approve "{\"confirm\":\"$(devid peer-a peer-a)\"}"
  expect "the page gives a reply, text equals QR" "$LAST_OUT" '.ok == true and .qr_equals_clipboard != false'
  enr_send "$(pf "$LAST_OUT" reply)"
  OPENED=$(enr_get 4); echo "terminal: $OPENED"
  expect "the credential opened with the phone's share" "$OPENED" '.opened == true'
else
  echo "--- GAP: the page refuses the CLI's challenge: $(pf "$LAST_OUT" refusal)"
fi
if enr_close; then echo "ok   sync-share-request exits 0"; else echo "--- GAP: sync-share-request did not finish (exit $?)"; fi
dump_state "after quorum"

finish "$NAME"
