#!/bin/sh
# timeout: 2700
# 66: the NATIVE Android app (the debug APK with the real Rust core, on the emulator of this compose project) as the ring's phone
# against real terminals (the CLI in the peers). Needs the phone target `native` (deploy/e2e/phone-target) and the APKs built with
# gpu-verify/android.sh <checkout> hs-e2e2-app build. The app is driven through its real UI by LiveTerminalTest; frames move as text
# (paste in, Copy out). QR, sound and Bluetooth are not used.
#   1. enrollment: restore from the fixed words, enroll to peer-a over text with the four-word check, in the ring-setup order
#      (the app signs its own admission, then promotions and the paper registration)
#   2. a share release: the terminal asks (sync-share-request), the app approves, the reply text goes back, the credential opens
#   3. a Red approval from the app: the revocation of a holder
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
. "$E2E_DIR/lib-app.sh"
NAME=66-app-live-terminal

if [ "$(cat "$E2E_DIR/phone-target" 2>/dev/null)" != native ]; then
  echo "skipped: scenario 66 needs the phone target native (the emulator and the app APKs)"
  finish "$NAME"
  exit 0
fi

PHONE_PREP=app_prep
PHONE_ENROLL=enroll_app
PHONE_SIGNS=app_signs
step "ring with the app as the phone (ring3)" ring_with_phone ring3
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "the app is a holder in force on peer-a" "$st" '[.devices[] | select(.name=="my phone") | .role] == ["holder"]'

# ---- 2. a share release with the app as the phone (Quorum mode first, as in scenario 60)
check "peer-a sets up the Vault" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-a $SSHUB sync-import-kit"
check "peer-b imports the kit" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-b $SSHUB sync-import-kit"
check "peer-a stores host quorum1 with a password" "printf '%s\n' '$SECRET' | peer peer-a $SSHUB host add --name quorum1 --address quorum1.internal --password-stdin"
check "peer-a migrates it into the ring" "peer peer-a $SSHUB sync-migrate"
show "peer-a sync-mode quorum, first call proposes it" sh -c "printf '%s\n' '$PHRASE' | $C exec -T peer-a $SSHUB sync-mode quorum --format json"
MODE_OP=$(opid "$LAST_OUT")
step "mode change ceremony (app, peer-b, paper)" red_complete "$MODE_OP"
show "peer-a sync-mode quorum again: the ring agreed" sh -c "printf '%s\n' '$PHRASE' | $C exec -T peer-a $SSHUB sync-mode quorum --format json"
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-a reports Quorum mode" "$st" '.credential_mode == "quorum"'

share_release_with_app() {
enr_open peer-a sync-share-request quorum1 --wait 400 --format json
# events: 1 host, 2 words, 3 challenge; 4 is the answer after the reply
CH=$(enr_get 3); echo "terminal: $CH" | cut -c1-200
CHALLENGE=$(pf "$CH" challenge)
if [ -n "$CHALLENGE" ]; then
  app_open_frame "$CHALLENGE"
  step "app: the share release screen" app '{"op":"wait","text":"Share release","timeout":20000}'
  appdump "the share release"
  if app '{"op":"exists","text":"Typed requester id"}' && [ "$(appget exists)" = true ]; then
    step "app: the requester id is typed (never met)" apptype "Typed requester id" "$(devid peer-a peer-a)"
  fi
  step "app: approve and release" appclick "Approve and release"
  app '{"op":"auth","timeout":15000}' >/dev/null && echo "    (system prompt after Approve: $(appget auth))"
  step "app: released" app '{"op":"wait","text":"Released.","timeout":60000}'
  step "app: copy as text" appclick "Copy as text"
  app '{"op":"clip"}' >/dev/null
  REPLY=$(appget text)
  check "the clipboard holds the reply text" "[ -n '$REPLY' ]"
  enr_send "$REPLY"
  OPENED=$(enr_get 4); echo "terminal: $OPENED"
  expect "the credential opened with the app's share" "$OPENED" '.opened == true'
else
  echo "FAIL no challenge from sync-share-request"; fail=$((fail + 1))
fi
if enr_close; then echo "ok   sync-share-request exits 0"; else echo "FAIL sync-share-request exit status"; fail=$((fail + 1)); fi
step "app: back to the approvals" appclick "Back to approvals"
}
step "share release: peer-a asks, the app approves, the credential opens" share_release_with_app

# ---- 3. a Red approval from the app: the revocation of a holder
C_ID=$(devid peer-a peer-c)
check "peer-a can name peer-c" "test -n '$C_ID'"
show "peer-a sync-revoke peer-c (a holder)" peer peer-a $SSHUB sync-revoke "$C_ID" --format json
REV=$(opid "$LAST_OUT")
check "the revocation is a Red op" "test -n '$REV'"
step "revocation ceremony (app, peer-b, paper)" red_complete "$REV"
expect "the revocation ceremony is complete" "$LAST_OUT" '.ceremony.complete == true'
spread peer-b peer-a peer-b
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-c is revoked on peer-a" "$st" '[.devices[] | select(.name=="peer-c") | .status] == ["revoked"]'

step "app: the Ring tab" appclick "Ring"
appdump "Home (the ring as the app sees it)"
app_stop
dump_state "after the app's session"
finish "$NAME"
