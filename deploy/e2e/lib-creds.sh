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
  check "peer-a founds the ring with hub" "enroll_peer peer-a hub"
  check "peer-b joins via hub" "enroll_peer peer-b hub"
  check "peer-a approves peer-b's admission" "approve_all peer-a"
  check "peer-b records peer-a directly" "enroll_peer peer-b peer-a"
}

# ring2 plus peer-c admitted through hub and known to peer-a and peer-b.
ring3() {
  ring2
  check "peer-c joins via hub" "enroll_peer peer-c hub"
  check "peer-a approves peer-c's admission" "approve_all peer-a"
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
opid() { printf '%s' "$1" | pjq -r '.op'; }

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
