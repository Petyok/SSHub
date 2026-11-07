# Helpers for scenario 66: the NATIVE Android app (debug APK, real core) on the emulator of this compose project, driven by the
# instrumentation test LiveTerminalTest (android/app/src/androidTest/.../live/LiveTerminalTest.kt) through files in the app's
# external files directory. Sourced after lib.sh and lib-creds.sh. The scenario moves the frames of the real terminals (CLI verbs in
# the peers) by paste text into the app and by the clipboard text its Copy buttons fill; QR, sound and Bluetooth are not used.

APP_PKG=uk.petruha.sshub.phone.debug
APP_LIVE=/sdcard/Android/data/$APP_PKG/files/live
# android.sh builds use the fixed run name hs-e2e2-app (a warm cargo target); hs-e2e-live is the older name of the first builds.
APK_DIR=${APK_DIR:-$([ -d /root/sshub-ci/android-app/out/hs-e2e2-app ] && echo /root/sshub-ci/android-app/out/hs-e2e2-app || echo /root/sshub-ci/android-app/out/hs-e2e-live)}
APP_OUT=

# A step the rest of the scenario depends on: on failure the scenario ends here with what it has.
must() {
  step "$@" || {
    echo "--- the scenario cannot go on after: $1"
    [ -z "${APP_INSTR:-}" ] || app_stop
    finish "$NAME"
  }
}

# app JSON: one command to the live session; the answer is in APP_OUT; fails when the session says ok=false or does not answer.
app() {
  id=$(date +%s%N)
  printf '%s' "$1" | $C exec -T android adb shell "cat > $APP_LIVE/in/$id.tmp && mv $APP_LIVE/in/$id.tmp $APP_LIVE/in/$id.json" || { echo "    app: could not write the command"; return 2; }
  i=0
  while :; do
    APP_OUT=$($C exec -T android adb shell "cat $APP_LIVE/out/$id.json 2>/dev/null" | tr -d '\r')
    case $APP_OUT in '{'*'}') break ;; esac
    i=$((i + 1))
    [ "$i" -le 400 ] || { APP_OUT='{"ok":false,"error":"no answer within 400 s"}'; break; }
    sleep 1
  done
  case $APP_OUT in
    *'"ok":true'*) printf '    app %s -> %s\n' "$(printf '%s' "$1" | cut -c1-110)" "$(printf '%s' "$APP_OUT" | cut -c1-160)"; return 0 ;;
  esac
  printf '    app %s -> FAILED\n' "$(printf '%s' "$1" | cut -c1-200)"
  printf '%s\n' "$APP_OUT" | pjq -r '.error // ., .screen // empty' 2>/dev/null | sed 's/^/    | /' || printf '%s\n' "$APP_OUT" | cut -c1-3000
  return 1
}
# appdump LABEL: the whole screen text into the log (evidence for what the app shows).
appdump() {
  echo "--- the app's screen: $1"
  app '{"op":"screen"}' >/dev/null && printf '%s' "$APP_OUT" | pjq -r .screen | sed 's/^/    | /'
}
# appget FIELD: a field of the last answer, raw.
appget() { printf '%s' "$APP_OUT" | pjq -r ".$1 // empty"; }
# appclick TEXT [exact=true|false] [timeout ms]
appclick() { app "{\"op\":\"click\",\"text\":\"$1\",\"exact\":${2:-true},\"timeout\":${3:-20000}}"; }
# apptype DESC TEXT
apptype() { app "{\"op\":\"type\",\"desc\":\"$1\",\"text\":\"$2\"}"; }
# appread DESC: the text of a node, polled until it is not empty (a screen fills its text a moment after it opens).
appread() {
  j=0
  while [ "$j" -lt 15 ]; do
    app "{\"op\":\"read\",\"desc\":\"$1\"}" >/dev/null || return 1
    t=$(appget text)
    if [ -n "$t" ]; then APP_TEXT=$t; return 0; fi
    j=$((j + 1)); sleep 1
  done
  return 1
}
# appfind REGEX: the first capture of a regex over every text on the screen.
appfind() { app "{\"op\":\"find\",\"pattern\":\"$1\",\"timeout\":${2:-20000}}" && APP_MATCH=$(appget match); }

# Install both APKs, start the live session (a long instrumentation run), wait until it is ready.
app_boot() {
  for f in app-debug.apk app-debug-androidTest.apk; do
    [ -f "$APK_DIR/$f" ] || { echo "missing $APK_DIR/$f: build it with gpu-verify/android.sh <checkout> hs-e2e2-app build"; return 1; }
    $C cp "$APK_DIR/$f" "android:/tmp/$f" || return 1
    out=$($C exec -T android adb install -r "/tmp/$f" 2>&1); echo "adb install $f: $out"
    printf '%s' "$out" | grep -q Success || return 1
  done
  echo "android: $($C exec -T android adb shell getprop ro.build.version.release | tr -d '\r') sdk $($C exec -T android adb shell getprop ro.build.version.sdk | tr -d '\r'), $(sha256sum "$APK_DIR/app-debug.apk" | cut -c1-16) $(sha256sum "$APK_DIR/app-debug-androidTest.apk" | cut -c1-16)"
  $C exec -T android adb shell am instrument -w -r -e live true -e class uk.petruha.sshub.phone.live.LiveTerminalTest \
    uk.petruha.sshub.phone.debug.test/androidx.test.runner.AndroidJUnitRunner > /tmp/app-instr.log 2>&1 &
  APP_INSTR=$!
  j=0
  until $C exec -T android adb shell "cat $APP_LIVE/ready" 2>/dev/null | grep -q /; do
    j=$((j + 1))
    [ "$j" -le 120 ] || { echo "the live session did not come up; instrumentation log:"; cat /tmp/app-instr.log; return 1; }
    sleep 1
  done
  echo "the live session is ready: $($C exec -T android adb shell "cat $APP_LIVE/ready" | tr -d '\r')"
}

# End the session and print what the instrumentation said.
app_stop() {
  app '{"op":"stop"}' >/dev/null 2>&1 || true
  wait "$APP_INSTR" 2>/dev/null || true
  echo "--- instrumentation log:"; sed 's/^/    | /' /tmp/app-instr.log
}

# The app as the ring's phone, before any ring exists: screen lock, restore from the fixed words, chain taken as "never signed".
app_prep() {
  must "the phone daemon answers (it only supplies the fixed words)" phone_ready
  WORDS=$(pf "$(phone words)" words)
  check "the fixed test words are 24" "[ \$(printf '%s' '$WORDS' | wc -w) -eq 24 ]"
  must "the app boots with the real core" app_boot
  must "a screen lock PIN is set (the Keystore key needs one)" app '{"op":"setpin"}'
  must "first run offers the restore" appclick "Restore from 24 words"
  must "the 24 words are pasted" apptype "Pasted words" "$WORDS"
  must "the grid is filled" appclick "Fill the grid"
  must "the page was never used on this phone" appclick "No, never"
  must "restore this phone" appclick "Restore this phone"
  must "the system prompt is answered with the PIN" app '{"op":"auth"}'
  must "the chain screen shows the device id to type" appfind 'type ([0-9A-Fa-f]{16}) to confirm' 60000
  APP_ID=$APP_MATCH
  echo "the app's device id: $APP_ID"
  must "the device id is typed" apptype "Type the device id to confirm" "$APP_ID"
  must "this key never signed anything" appclick "This key never signed anything"
  step "the chain is taken (confirmed as never signed)" app '{"op":"wait","text":"This key never signed anything.","timeout":20000}'
  must "back on Home" appclick "Go to Home"
}

# The whole enrollment of the app to PEER over text. Sets ADD (the phone's admission op).
enroll_app() {
  enr_open "$1" sync-phone-enroll --format json
  E1=$(enr_get 1)
  must "the Enroll tab" appclick "Enroll"
  must "the phone name" apptype "Name this phone will have in the ring" "my phone"
  must "the channel is Paste text" appclick "Paste text"
  must "open the paste step" appclick "Paste the text"
  must "step 1: the terminal commitment is pasted" apptype "Text from the terminal" "$(pf "$E1" frame)"
  must "step 1: use this text" appclick "Use this text"
  must "step 2: the app's commitment" appread "Text for the terminal, the same bytes as the QR code, base64"
  enr_send "$APP_TEXT"
  E2=$(enr_get 2)
  must "step 2: next" appclick "Next"
  must "step 3: the terminal reveal is pasted" apptype "Text from the terminal" "$(pf "$E2" frame)"
  must "step 3: check the keys" appclick "Check the keys"
  must "step 4: the app shows four words" appfind '^([a-z]+, [a-z]+, [a-z]+, [a-z]+)$' 40000
  APP_WORDS=$(printf '%s' "$APP_MATCH" | sed 's/, /-/g')
  must "step 4: the app's reveal" appread "Text for the terminal, the same bytes as the QR code, base64"
  enr_send "$APP_TEXT"
  E3=$(enr_get 3); echo "terminal: $E3"
  check "the terminal and the app show the same four words" "[ \"$(pf "$E3" words)\" = \"$APP_WORDS\" ]"
  must "step 4: the words match" appclick "The words match"
  must "step 4: continue" appclick "Continue"
  must "step 5: the app's confirmation" appread "Text for the terminal, the same bytes as the QR code, base64"
  enr_send "$APP_TEXT"
  E4=$(enr_get 4); echo "terminal: $E4"
  enr_send y
  E5=$(enr_get 5)
  must "step 5: the terminal confirmation is pasted" apptype "Text from the terminal" "$(pf "$E5" frame)"
  must "step 5: check the signature" appclick "Check the signature"
  # (the app goes straight to its Enrolled screen after the signature check; a Finish button is not always there)
  appclick "Finish" true 6000 >/dev/null || echo "    (no Finish button: the app went straight to Enrolled)"
  must "enrolled: done" appclick "Done"
  if enr_close; then echo "ok   sync-phone-enroll exits 0 (the app verified and admitted)"; else echo "FAIL sync-phone-enroll exit status"; fail=$((fail + 1)); fi
  ADD=$(lastop "$1" DeviceAdd)
  check "the app's admission op exists on $1" "test -n '$ADD'"
}

# A frame of a terminal into the app through Approvals, Scan a request, paste, until the screen of what it asks is open.
app_open_frame() {
  step "app: the Approvals tab" appclick "Approvals" || return 1
  step "app: scan a request" appclick "Scan a request" || return 1
  appclick "Paste text" false 15000 >/dev/null || echo "    (no paste button: the paste box may be open already)"
  step "app: the frame is pasted" apptype "Pasted text" "$1" || return 1
  step "app: use the pasted text" appclick "Use pasted text" || return 1
  step "app: continue" appclick "Continue" || return 1
}

# The app takes part in the Red op $2 of peer $1: sync-notice -> the app's Red screen, Approve, Copy as text -> sync-phone-ops.
# Same contract as phone_signs: PHONE_OPS holds the ops text, LAST_OUT the verb's JSON.
app_signs() {
  show "$1 sync-notice $2" peer "$1" $SSHUB sync-notice "$2" --format json
  [ "$LAST_RC" -eq 0 ] || return 1
  app_open_frame "$(pf "$LAST_OUT" notice)" || return 1
  step "app: the Red operation screen" app '{"op":"wait","text":"RED OPERATION","timeout":20000}' || return 1
  [ -n "${APP_RED_SHOWN:-}" ] || { appdump "a Red operation (the first one)"; APP_RED_SHOWN=1; }
  step "app: approve" appclick "Approve" || return 1
  app '{"op":"auth","timeout":15000}' >/dev/null && echo "    (system prompt after Approve: $(appget auth))"
  step "app: signed with this phone" app '{"op":"wait","text":"Signed with this phone","timeout":60000}' || return 1
  step "app: copy as text" appclick "Copy as text" || return 1
  app '{"op":"clip"}' >/dev/null || return 1
  PHONE_OPS=$(appget text)
  [ -n "$PHONE_OPS" ] || { echo "FAIL the clipboard is empty after Copy as text"; fail=$((fail + 1)); return 1; }
  step "app: done" appclick "Done" || true
  show "$1 sync-phone-ops takes the app's ops in" sh -c "printf '%s\n' '$PHONE_OPS' | $C exec -T $1 $SSHUB sync-phone-ops --format json"
  keep=$LAST_OUT; rc=$LAST_RC
  snow "$1" >/dev/null
  LAST_OUT=$keep; LAST_RC=$rc
  [ "$rc" -eq 0 ]
}
