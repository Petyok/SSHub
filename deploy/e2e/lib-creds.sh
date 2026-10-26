# Helpers for the credential and Red-operation scenarios (50-95). Sourced after
# lib.sh. Every secret below is a fixed test value, never a real credential.

# Ring secret and master phrase of the test Vault (the founder picks them; a
# joiner types the same kit and words with `sync-import-kit`).
KIT='sshub:ring:1:00112233445566778899aabbccddeeff'
PHRASE='abacus abdomen abdominal abide abiding ability'
# BIP39 test vector: entropy of 32 zero bytes. A valid 24-word paper phrase.
PAPER='abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art'
SECRET='Tr0ub4dor&3-e2e'

# Founder plus peer-b (attended, admitted on peer-a), each knowing the other.
ring2() {
  reset_peers
  check "peer-a founds the ring with hub" "enroll_peer peer-a hub || enroll_peer peer-a hub"
  check "peer-b joins via hub" "enroll_peer peer-b hub"
  check "peer-a approves peer-b's admission" "snow peer-a >/dev/null; approve_all peer-a"
  check "peer-b records peer-a directly" "enroll_peer peer-b peer-a"
}

# ring2 plus peer-c admitted through hub and known to peer-a and peer-b.
ring3() {
  ring2
  check "peer-c joins via hub" "enroll_peer peer-c hub"
  check "peer-a approves peer-c's admission" "snow peer-a >/dev/null; approve_all peer-a"
  check "peer-c records peer-a directly" "enroll_peer peer-c peer-a"
  check "peer-b records peer-c directly" "enroll_peer peer-b peer-c"
}

# Evidence only: print a labelled command, its output and its exit status.
show() {
  label=$1; shift
  echo "--- $label"
  out=$("$@" 2>&1); rc=$?
  printf '%s\n' "$out" | sed 's/^/    | /'
  echo "    (exit $rc)"
  LAST_OUT=$out; LAST_RC=$rc
}

# A step the product cannot do headless: print the exact command and output as
# a GAP line (never a failure) for deploy/e2e/FINDINGS.md.
gap() {
  desc=$1; shift
  echo "--- GAP: $desc"
  out=$("$@" 2>&1); rc=$?
  printf '%s\n' "$out" | sed 's/^/    | /'
  echo "    (exit $rc)"
  LAST_OUT=$out; LAST_RC=$rc
}

# Id of the op a JSON admin verb just proposed.
opid() { printf '%s' "$1" | pjq -r '.op // empty'; }

# Sync every attended peer with the others once, quietly.
spread() {
  for p in "$@"; do snow "$p" >/dev/null; done
}

# sync-status role count: holders in the ring as peer $1 sees it.
holders() { peer "$1" $SSHUB sync-status --format json 2>/dev/null | pjq '[.devices[] | select(.role=="holder")] | length'; }

# Device id of the ring member called $2, as peer $1 sees it.
devid() { peer "$1" $SSHUB sync-status --format json 2>/dev/null | pjq -r --arg n "$2" '[.devices[] | select(.name==$n) | .id][0] // empty'; }

# Read-only SQL against peer $1's launcher.db (evidence and counts, never writes).
sq() { p=$1; shift; peer "$p" sh -c 'sqlite3 "$(find "$HOME/.local/share/sshub" -name launcher.db | head -n1)" "$1"' sh "$*"; }
ckpts_accepted() { sq "$1" "SELECT count(*) FROM sync_checkpoints WHERE status IN ('accepted','pruned')"; }

# sync-now bounded to 15 minutes, with the elapsed seconds printed (large logs).
snowt() {
  t0=$(date +%s)
  out=$(peer "$1" timeout 900 $SSHUB sync-now --format json 2>&1) || true
  echo "sync-now on $1 took $(( $(date +%s) - t0 ))s"
  printf '%s\n' "$out" | pjq -c '.[]? | {name, status, applied, pending, received, rejected}' 2>/dev/null || printf '%s\n' "$out" | cut -c1-300
}

# ---- the e2e phone (phone-driver.mjs, compose service `phone`): additive helpers --------------------------------
# phone CMD [JSON]: one command against the phone daemon; prints the JSON answer, exit 1 when the page refused.
phone() { c=$1; shift; $C exec -T phone node /driver/phone-driver.mjs "$c" "$@"; }
# Wait (bounded) until the phone daemon answers.
phone_ready() {
  i=0
  until phone info >/dev/null 2>&1; do
    i=$((i + 1)); [ "$i" -le 90 ] || return 1; sleep 1
  done
}
# Canonical bytes and signature (hex) of the op whose hash starts with $2, as peer $1 stores it. Read only.
oprow() { sq "$1" "SELECT hex(canonical) || ' ' || hex(signature) FROM sync_ops WHERE lower(hex(op_hash)) LIKE lower('$2%') LIMIT 1"; }

# ---- text sessions with the CLI's phone verbs (the runner shell is the cable between terminal and phone) -----------
# enr_open PEER VERB [ARGS]: start a phone verb on PEER; its stdin is a fifo held open on fd 9, its events go to $ENR_DIR/out.
enr_open() {
  ENR_DIR=$(mktemp -d); mkfifo "$ENR_DIR/in"; : > "$ENR_DIR/out"
  p=$1; shift
  $C exec -T "$p" $SSHUB "$@" < "$ENR_DIR/in" > "$ENR_DIR/out" 2> "$ENR_DIR/err" &
  ENR_PID=$!
  exec 9> "$ENR_DIR/in"
}
# enr_send LINE: one line to the verb (a phone frame, or y/n for the four words).
enr_send() { printf '%s\n' "$1" >&9; }
# enr_get N: the Nth event line the verb has printed (waits up to 60 s).
enr_get() {
  i=0
  while :; do
    l=$(grep -v '"event":"companion"' "$ENR_DIR/out" | sed -n "${1}p")
    if [ -n "$l" ]; then printf '%s\n' "$l"; return 0; fi
    i=$((i + 1)); [ "$i" -le 240 ] || { echo "no event $1 from the verb; so far:"; cat "$ENR_DIR/out" "$ENR_DIR/err"; return 1; }
    sleep 0.25
  done
}
# enr_close: end the verb's stdin, print the rest of its output, return its exit status.
enr_close() {
  exec 9>&-
  wait "$ENR_PID"; rc=$?
  echo "--- verb events:"; sed 's/^/    | /' "$ENR_DIR/out"; sed 's/^/    ! /' "$ENR_DIR/err"
  rm -rf "$ENR_DIR"
  return $rc
}
# pf JSON KEY: one field of a JSON line, raw.
# (the phone verbs print a first {"event":"companion"} line; // empty drops the null it gives for other keys)
pf() { printf '%s' "$1" | pjq -r ".$2 // empty"; }

# Id (full hash, hex) of the newest op of kind $2 that peer $1 stores. Read only.
lastop() { sq "$1" "SELECT lower(hex(op_hash)) FROM sync_ops WHERE kind='$2' ORDER BY lamport DESC, rowid DESC LIMIT 1"; }

# The phone takes part in the Red op $2 of peer $1: sync-notice -> the page shows and approves -> sync-phone-ops.
# Leaves the verb's JSON in LAST_OUT; PHONE_OPS holds the phone's ops text.
phone_signs() {
  show "$1 sync-notice $2" peer "$1" $SSHUB sync-notice "$2" --format json
  [ "$LAST_RC" -eq 0 ] || return 1
  show "the phone reads the notice" phone red-open "{\"frame\":\"$(pf "$LAST_OUT" notice)\"}"
  [ "$LAST_RC" -eq 0 ] || return 1
  show "the phone approves (RedPhoneSign)" phone red-approve
  [ "$LAST_RC" -eq 0 ] || return 1
  PHONE_OPS=$(pf "$LAST_OUT" ops)
  show "$1 sync-phone-ops takes the phone's ops in" sh -c "printf '%s\n' '$PHONE_OPS' | $C exec -T $1 $SSHUB sync-phone-ops --format json"
}

# The whole phone enrollment to PEER as text, with its checks. Sets ADD (the phone's admission op).
enroll_phone() {
  enr_open "$1" sync-phone-enroll --format json
  E1=$(enr_get 1)
  show "the phone opens the enrollment" phone enroll-start
  show "the phone reads the terminal commitment" phone enroll-commit "{\"frame\":\"$(pf "$E1" frame)\"}"
  expect "the page gives a commitment" "$LAST_OUT" '.ok == true'
  enr_send "$(pf "$LAST_OUT" frame)"
  E2=$(enr_get 2)
  show "the phone reads the terminal reveal" phone enroll-reveal "{\"frame\":\"$(pf "$E2" frame)\"}"
  expect "the page shows four words" "$LAST_OUT" '.ok == true and (.words | length) > 3'
  PHONE_WORDS=$(pf "$LAST_OUT" words)
  enr_send "$(pf "$LAST_OUT" frame)"
  E3=$(enr_get 3); echo "terminal: $E3"
  check "the terminal and the phone show the same words" "[ \"$(pf "$E3" words)\" = \"$PHONE_WORDS\" ]"
  show "the phone taps: words match" phone enroll-match
  enr_send "$(pf "$LAST_OUT" frame)"
  E4=$(enr_get 4); echo "terminal: $E4"
  enr_send y
  E5=$(enr_get 5)
  show "the phone reads the terminal confirmation" phone enroll-finish "{\"frame\":\"$(pf "$E5" frame)\"}"
  expect "the page ends the enrollment" "$LAST_OUT" '.ok == true'
  if enr_close; then echo "ok   sync-phone-enroll exits 0 (phone verified and admitted)"; else echo "FAIL sync-phone-enroll exit status"; fail=$((fail + 1)); fi
  ADD=$(lastop "$1" DeviceAdd)
  check "the phone's admission op exists on $1" "test -n '$ADD'"
}

# Complete the Red op $1 proposed on peer-a: the phone signs it, peer-b votes, the paper signs (unless $2 = nofactor,
# for the first factor registration, which needs only the phone and a second device).
red_complete() {
  phone_signs peer-a "$1" || return 1
  spread peer-b peer-a peer-b
  show "peer-b is given the phone's ops text too" sh -c "printf '%s\n' '$PHONE_OPS' | $C exec -T peer-b $SSHUB sync-phone-ops --format json"
  show "peer-b sync-approve $1" peer peer-b $SSHUB sync-approve "$1" --format json
  spread peer-a peer-b peer-a
  if [ "${2:-}" != nofactor ]; then
    show "peer-a adds the paper factor" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor $1 --paper --format json"
    echo "ceremony of $1: $(pf "$LAST_OUT" ceremony | tr -d '\n ')"
  fi
}

# A ring ($1 = ring2 or ring3) whose phone is in force on peer-a: the paper factor registration is proposed first (a
# factor must be registered before the op it signs), the phone enrolls over text, then phone + peer-b complete the
# registration and the phone's own admission (plus the paper).
ring_with_phone() {
  check "the phone daemon answers" phone_ready
  show "the phone restores its fixed test identity" phone setup
  "$1"
  show "peer-a registers the paper factor (Red)" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor-register paper --format json"
  REG=$(opid "$LAST_OUT")
  enroll_phone peer-a
  red_complete "$REG" nofactor
  red_complete "$ADD"
  st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
  expect "the phone is a holder in force on peer-a" "$st" '[.devices[] | select(.name=="my phone") | .role] == ["holder"]'
}
