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
# PHONE_SVC names the compose service the phone helpers talk to (`phone`, or `phone2` inside as_phone2).
PHONE_SVC=phone
phone() { c=$1; shift; $C exec -T "$PHONE_SVC" node /driver/phone-driver.mjs "$c" "$@"; }
# Wait (bounded) until the phone daemon answers.
phone_ready() {
  i=0
  until phone info >/dev/null 2>&1; do
    i=$((i + 1)); [ "$i" -le 90 ] || return 1; sleep 1
  done
}
# The second phone (compose service phone2, its own key: device id differs from the first phone's).
# phone2 CMD [JSON] is one command against it; as_phone2 CMD ARGS... runs any helper below (enroll_phone, phone_signs,
# phone_prep_page, ...) with every `phone` call going to phone2, and returns the helper's status.
phone2() { PHONE_SVC=phone2 phone "$@"; }
as_phone2() { PHONE_SVC=phone2; "$@"; r=$?; PHONE_SVC=phone; return $r; }
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
# (a verb that already gave up closed the fifo: a write then must fail, not kill the scenario shell with SIGPIPE)
enr_send() { ( trap '' PIPE; printf '%s\n' "$1" >&9 ) || echo "the verb no longer reads its stdin"; }
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
  keep=$LAST_OUT; rc=$LAST_RC
  snow "$1" >/dev/null  # a device settles what it ingested in its next sync round, not at ingest
  LAST_OUT=$keep; LAST_RC=$rc
  [ "$rc" -eq 0 ]
}

# The whole phone enrollment to PEER ($1) as text, with its checks; $2 = the phone's name (default "my phone"). Sets ADD (the phone's admission op).
enroll_phone() {
  enr_open "$1" sync-phone-enroll --format json
  E1=$(enr_get 1)
  show "the phone opens the enrollment" phone enroll-start ${2:+"{\"name\":\"$2\"}"}
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
  ${PHONE_SIGNS:-phone_signs} peer-a "$1" || return 1
  spread peer-b peer-a peer-b
  show "peer-b is given the phone's ops text too" sh -c "printf '%s\n' '$PHONE_OPS' | $C exec -T peer-b $SSHUB sync-phone-ops --format json"
  snow peer-b >/dev/null
  show "peer-b sync-approve $1" peer peer-b $SSHUB sync-approve "$1" --format json
  spread peer-a peer-b peer-a
  if [ "${2:-}" != nofactor ]; then
    show "peer-a adds the paper factor" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor $1 --paper --format json"
    echo "ceremony of $1: $(pf "$LAST_OUT" ceremony | tr -d '\n ')"
  fi
}

# The page phone before a ring: answers, forgets its site data (a new ring meets a phone with an empty chain), restores.
phone_prep_page() {
  check "the phone daemon answers" phone_ready
  show "the phone forgets its site data (a new ring meets a phone with an empty chain)" phone reset
  show "the phone restores its fixed test identity" phone setup
}

# A ring ($1 = ring2 or ring3) whose phone and holders are in force, in the order ring setup allows: the phone enrolls
# first and signs its own admission (no factor exists yet); then the paper is registered (the phone signs; a third full
# device needs it first, design 3.1); then each terminal is promoted with the full ceremony (phone, second holder,
# paper), as is every later Red op (red_complete).
ring_with_phone() {
  ${PHONE_PREP:-phone_prep_page}
  "$1"
  ${PHONE_ENROLL:-enroll_phone} peer-a
  ${PHONE_SIGNS:-phone_signs} peer-a "$ADD"
  st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
  expect "the phone is a holder in force on peer-a" "$st" '[.devices[] | select(.name=="my phone") | .role] == ["holder"]'
  promote() { # $1 = terminal, $2 = setup (own approval, no paper) or full (phone, second holder, paper)
    spread "$1" peer-a "$1"
    show "peer-a sync-role $1 holder" peer peer-a $SSHUB sync-role "$(devid peer-a "$1")" holder --format json
    ROLEOP=$(opid "$LAST_OUT")
    if [ "$2" = setup ]; then
      ${PHONE_SIGNS:-phone_signs} peer-a "$ROLEOP"
      spread peer-a "$1" peer-a
      show "$1 sync-approve its own promotion" peer "$1" $SSHUB sync-approve "$ROLEOP" --format json
    else
      red_complete "$ROLEOP"
    fi
    spread peer-a "$1" peer-a
  }
  # the paper comes first (design 3.1): peer-b would be the third full device, and sync-role refuses without a paper
  show "peer-a registers the paper factor (Red)" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor-register paper --format json"
  REG=$(opid "$LAST_OUT")
  ${PHONE_SIGNS:-phone_signs} peer-a "$REG"
  promote peer-b full
  [ "$1" = ring3 ] && promote peer-c full
  spread peer-b peer-c peer-a peer-b peer-c peer-a
  ring_view
}

# Evidence: every peer's table and the op rows that decide who is a holder in force.
ring_view() {
  for p in peer-a peer-b peer-c hub; do
    echo "$p sees: $(peer $p $SSHUB sync-status --format json 2>&1 | pjq -c '[.devices[] | {name, role, status}]')"
    echo "$p ops (kind|state|reason|n): $(sq $p "SELECT kind||'|'||state||'|'||coalesce(reason,'')||'|'||count(*) FROM sync_ops WHERE kind IN ('DeviceAdd','DeviceRoleChange','FactorRegister','RedPhoneSign','RedApprove','RedFactor') GROUP BY kind,state,reason" | tr '\n' ';')"
  done
}

# ---- Phase B helpers ---------------------------------------------------------------------------------------------

# A Quorum ring with the phone ($1 = ring2 or ring3): the kit on every terminal, host quorum1 saved on peer-a with
# $SECRET, migrated into the ring, the Red switch to Quorum completed (phone, peer-b, paper) and re-encrypted.
quorum_ring() {
  ring_with_phone "$1"
  for p in peer-a peer-b $( [ "$1" = ring3 ] && echo peer-c ); do
    check "$p imports the kit" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer $p $SSHUB sync-import-kit"
  done
  check "peer-a stores host quorum1 with a password" "printf '%s\n' '$SECRET' | peer peer-a $SSHUB host add --name quorum1 --address quorum1.internal --password-stdin"
  check "peer-a migrates it into the ring" "peer peer-a $SSHUB sync-migrate"
  show "peer-a sync-mode quorum (proposes the Red switch)" sh -c "printf '%s\n' '$PHRASE' | $C exec -T peer-a $SSHUB sync-mode quorum --format json"
  expect "the mode change is a Red op with a ceremony" "$LAST_OUT" '.tier == "Red" and (.ceremony != null)'
  step "mode change ceremony (phone, peer-b, paper)" red_complete "$(opid "$LAST_OUT")"
  show "peer-a sync-mode quorum again (re-encrypts)" sh -c "printf '%s\n' '$PHRASE' | $C exec -T peer-a $SSHUB sync-mode quorum --format json"
  spread peer-b peer-c peer-a peer-b peer-c
  approve_all peer-b >/dev/null 2>&1 || true
  [ "$1" = ring3 ] && approve_all peer-c >/dev/null 2>&1
  spread peer-b peer-c peer-a peer-b peer-c
  st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
  expect "peer-a reports Quorum mode" "$st" '.credential_mode == "quorum"'
  return 0
}

# The Red op $1 of peer-a completed by the phone and the paper alone (the second device is waived for the removal of a
# holder, design 3.2 rule 1; also what a ring with one other attended terminal can give).
red_phone_paper() {
  ${PHONE_SIGNS:-phone_signs} peer-a "$1" || return 1
  show "peer-a adds the paper factor" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor $1 --paper --format json"
  echo "ceremony of $1: $(pf "$LAST_OUT" ceremony | tr -d '\n ')"
}

# Full ids of the devices a Quorum credential is wrapped to, as peer $1 stores them (read only evidence).
wraps_of() { sq "$1" "SELECT wrapped_shares FROM sync_cred_state"; }

# ---- trust sessions (certificates) -------------------------------------------------------------------------------

# A ring with the phone and three terminals (ring3, paper in force), host certhost on peer-a and applied everywhere
# (a trust session names hosts the delegate knows).
trust_ring() {
  ring_with_phone ring3
  check "peer-a adds host certhost" "peer peer-a $SSHUB host add --name certhost --address certhost.internal"
  spread peer-b peer-c peer-a peer-b peer-c
  approve_all peer-b >/dev/null 2>&1 || true
  approve_all peer-c >/dev/null 2>&1 || true
  spread peer-b peer-c peer-a peer-b peer-c
}

# A trust session ($1 = delegate terminal, $2 = host name, $3 = days): the delegate writes the kind 9 request, the phone
# signs it, the ops text goes to peer-a, a second device (peer-b) approves and the paper is added. Sets CERT_OP (the
# CertIssue change id).
issue_cert() {
  show "$1 sync-cert-request (the delegate asks)" peer "$1" $SSHUB sync-cert-request --host "$2" --days "${3:-30}" --budget 5 --format json
  [ "$LAST_RC" -eq 0 ] || return 1
  CERT_REQ=$(pf "$LAST_OUT" request)
  show "the phone reads the trust session request" phone cert-open "{\"frame\":\"$CERT_REQ\"}"
  expect "the page shows the request and an approve button" "$LAST_OUT" '.ok == true and .can_approve == true'
  show "the phone signs the trust session" phone cert-approve "{\"confirm\":\"$(devid peer-a "$1")\"}"
  expect "the page gives the ops text" "$LAST_OUT" '.ok == true'
  PHONE_OPS=$(pf "$LAST_OUT" ops)
  for p in peer-a peer-b; do
    show "$p sync-phone-ops takes the phone's list in" sh -c "printf '%s\n' '$PHONE_OPS' | $C exec -T $p $SSHUB sync-phone-ops --format json"
  done
  spread peer-a peer-b peer-c peer-a
  CERT_OP=$(lastop peer-a CertIssue)
  [ -n "$CERT_OP" ] || { echo "FAIL no CertIssue op on peer-a"; fail=$((fail + 1)); return 1; }
  snow peer-b >/dev/null
  show "peer-b sync-approve the trust session" peer peer-b $SSHUB sync-approve "$CERT_OP" --format json
  spread peer-a peer-b peer-a
  show "peer-a adds the paper factor" sh -c "printf '%s\n' '$PAPER' | $C exec -T peer-a $SSHUB sync-factor $CERT_OP --paper --format json"
  expect "the trust session ceremony is complete" "$LAST_OUT" '.ceremony.complete == true'
  spread peer-b peer-c peer-a peer-b peer-c
}

# The trust sessions peer $1 lists: one JSON object per line (id, delegate, expiry, renewals).
trust_sessions() { peer "$1" $SSHUB sync-trust-renew --format json 2>/dev/null | pjq -c 'select(.event=="session")'; }

# One renewal of the session $2 on peer-a ($3 = days): the request, the phone signs, the list goes to peer-a. Leaves the
# phone's open answer in RENEW_OPEN, the terminal's request events in RENEW_REQ_OUT.
renew_cert() {
  show "peer-a sync-trust-renew $2 (the request)" peer peer-a $SSHUB sync-trust-renew "$2" --days "${3:-30}" --format json
  RENEW_REQ_OUT=$LAST_OUT
  [ "$LAST_RC" -eq 0 ] || return 1
  show "the phone reads the renewal request" phone renew-open "{\"frame\":\"$(pf "$LAST_OUT" request)\"}"
  RENEW_OPEN=$LAST_OUT
  [ "$(pf "$RENEW_OPEN" can_approve)" = true ] || return 1
  show "the phone signs the renewal" phone renew-approve
  [ "$LAST_RC" -eq 0 ] || return 1
  PHONE_OPS=$(pf "$LAST_OUT" ops)
  show "peer-a sync-phone-ops takes the renewal in" sh -c "printf '%s\n' '$PHONE_OPS' | $C exec -T peer-a $SSHUB sync-phone-ops --format json"
  spread peer-a peer-b peer-c peer-a
}

# ---- a restored phone takes its chain (design U13-5, one-list rule) -------------------------------------------------

# A ring of peer-a and the Hub with the phone enrolled and having signed its admission (its chain is in the ring); the
# phone is then restored from its words WITHOUT the "never signed" shortcut, so its card asks for the chain.
phone_ring_then_restore() {
  phone_prep_page
  reset_peers
  check "peer-a founds the ring with hub" "enroll_peer peer-a hub || enroll_peer peer-a hub"
  enroll_phone peer-a
  phone_signs peer-a "$ADD"
  check "peer-a syncs to the Hub" "snow peer-a >/dev/null"
}
restore_phone() {
  show "the phone forgets its site data" phone reset
  show "the phone restores its words and keeps the take-chain card" phone setup '{"never_signed":false}'
}

# The kind 5 list peer $1 shows for the phone (its own newest ops, the sender and the terminals block): prints the text.
chain_of() { peer "$1" $SSHUB sync-phone-chain "$(devid "$1" 'my phone')" --format json 2>/dev/null | pjq -r 'select(.event=="chain") | .chain'; }
# The count the terminal reports (the same unsigned statement the page shows) as peer $1 prints it in JSON.
chain_terminals() { peer "$1" $SSHUB sync-phone-chain "$(devid "$1" 'my phone')" --format json 2>/dev/null | pjq -r 'select(.event=="chain") | .terminals.count'; }

# ---- PhoneHandover (design 5, contract 4j/4l): the old phone (service phone) hands the ring over to the new phone
# (service phone2) -------------------------------------------------------------------------------------------------

# The new phone enrolls with `sync-phone-enroll --replace` on terminal $1 (all steps as text). The terminal prints the kind
# 12 request for the OLD phone and the eight words. Sets HO_REQ and HO_WORDS (one line, lower case).
enroll_phone_replace() {
  enr_open "$1" sync-phone-enroll --replace --format json
  E1=$(enr_get 1)
  show "phone2 opens the enrollment" phone2 enroll-start "{\"name\":\"${PH2_NAME:-new phone}\"}"
  show "phone2 reads the terminal commitment" phone2 enroll-commit "{\"frame\":\"$(pf "$E1" frame)\"}"
  expect "the page gives a commitment" "$LAST_OUT" '.ok == true'
  enr_send "$(pf "$LAST_OUT" frame)"
  E2=$(enr_get 2)
  show "phone2 reads the terminal reveal" phone2 enroll-reveal "{\"frame\":\"$(pf "$E2" frame)\"}"
  expect "the page shows four words" "$LAST_OUT" '.ok == true and (.words | length) > 3'
  enr_send "$(pf "$LAST_OUT" frame)"
  E3=$(enr_get 3); echo "terminal: $E3"
  show "phone2 taps: words match" phone2 enroll-match
  enr_send "$(pf "$LAST_OUT" frame)"
  E4=$(enr_get 4); echo "terminal: $E4"
  enr_send y
  E5=$(enr_get 5)
  show "phone2 reads the terminal confirmation" phone2 enroll-finish "{\"frame\":\"$(pf "$E5" frame)\"}"
  expect "the page ends the enrollment" "$LAST_OUT" '.ok == true'
  E6=$(enr_get 6); E7=$(enr_get 7)
  echo "terminal: $E6" | cut -c1-200; echo "terminal: $E7"
  HO_REQ=$(pf "$E6" request)
  HO_WORDS=$(printf '%s' "$E7" | pjq -r '.words | join(" ")' | tr 'A-Z' 'a-z')
  if enr_close; then echo "ok   sync-phone-enroll --replace exits 0 (the new phone is verified, not yet admitted)"; else echo "FAIL sync-phone-enroll --replace exit status"; fail=$((fail + 1)); fi
  check "the terminal printed the request and eight words" "test -n '$HO_REQ' && [ \"\$(printf '%s' '$HO_WORDS' | wc -w)\" -eq 8 ]"
}

# The handover up to the point where both phones signed, through terminal peer-a. Sets HO_OP (the PhoneHandover change id),
# HO_ADM (the second kind 12 frame for the new phone) and OLD_OPS / NEW_OPS (the phones' ops texts). Leaves the other
# proofs (a second terminal, the paper) to the caller.
handover_start() {
  step "the new phone enrolls with --replace" enroll_phone_replace peer-a
  show "the OLD phone reads the handover request" phone handover-open "{\"frame\":\"$HO_REQ\"}"
  expect "the old phone shows eight words and an approve button" "$LAST_OUT" '.ok == true and .can_approve == true and ((.words | split(" ") | length) == 8)'
  check "the old phone's words equal the terminal's" "[ \"$(pf "$LAST_OUT" words | tr 'A-Z' 'a-z')\" = '$HO_WORDS' ]"
  show "the old phone signs the handover" phone handover-approve
  expect "the page gives the ops text" "$LAST_OUT" '.ok == true'
  OLD_OPS=$(pf "$LAST_OUT" ops)
  show "peer-a sync-phone-ops takes the old phone's list in" sh -c "printf '%s\n' '$OLD_OPS' | $C exec -T peer-a $SSHUB sync-phone-ops --format json"
  HO_ADM=$(printf '%s' "$LAST_OUT" | pjq -r 'select(.event=="handover_admission") | .frame')
  check "peer-a prints the second kind 12 frame (the op hash) for the new phone" "test -n '$HO_ADM'"
  check "the second frame carries the same eight words" "[ \"$(printf '%s' "$LAST_OUT" | pjq -r 'select(.event=="handover_admission") | .words' | tr 'A-Z' 'a-z')\" = '$HO_WORDS' ]"
  HO_OP=$(lastop peer-a PhoneHandover)
  check "the PhoneHandover op exists on peer-a" "test -n '$HO_OP'"
  show "the NEW phone reads its admission frame" phone2 handover-admit-open "{\"frame\":\"$HO_ADM\"}"
  expect "the new phone can sign its own admission" "$LAST_OUT" '.ok == true and .can_approve == true'
  show "the new phone signs its admission" phone2 handover-approve
  expect "the page gives the ops text" "$LAST_OUT" '.ok == true'
  NEW_OPS=$(pf "$LAST_OUT" ops)
  show "peer-a sync-phone-ops takes the new phone's list in" sh -c "printf '%s\n' '$NEW_OPS' | $C exec -T peer-a $SSHUB sync-phone-ops --format json"
  spread peer-a "${HO_SECOND:-peer-b}" peer-a
  for t in "$OLD_OPS" "$NEW_OPS"; do
    show "${HO_SECOND:-peer-b} is given the phone's list too" sh -c "printf '%s\n' '$t' | $C exec -T ${HO_SECOND:-peer-b} $SSHUB sync-phone-ops --format json"
  done
  snow "${HO_SECOND:-peer-b}" >/dev/null
}
