# Shared helpers for the host-sync scenarios. Sourced by scenarios, never run.
# run.sh exports E2E_DIR, C and PEERS.

: "${E2E_DIR:?run.sh must export E2E_DIR}"
C=${C:-docker compose -p sshub-hostsync-e2e -f "$E2E_DIR/docker-compose.yml"}
PEERS=${PEERS:-"peer-a peer-b peer-c hub"}
SSHUB="sshub --profile default"
# jq runs inside the peer image: the gpu-serv runner has no jq installed.
pjq() { docker run --rm -i --entrypoint jq sshub-hostsync-e2e-peer:local "$@"; }
fail=0

# Run argv inside a peer as user ci.
peer() { p=$1; shift; $C exec -T "$p" "$@"; }

# Run a shell snippet inside a peer.
psh() { p=$1; shift; $C exec -T "$p" sh -c "$*"; }

# Check one condition. Usage: check "description" 'shell command'. The command
# runs with eval; output is shown on failure.
check() {
  desc=$1; cmd=$2
  if out=$(eval "$cmd" 2>&1); then
    echo "ok   $desc"
  else
    echo "FAIL $desc"
    printf '%s\n' "$out" | sed 's/^/     | /'
    fail=$((fail + 1))
  fi
}

# JSON predicate: expect "description" "$json" '.pending >= 1'
expect() {
  desc=$1; json=$2; pred=$3
  if printf '%s' "$json" | pjq -e "$pred" >/dev/null 2>&1; then
    echo "ok   $desc"
  else
    echo "FAIL $desc (predicate: $pred)"
    printf '%s\n' "$json" | sed 's/^/     | /'
    fail=$((fail + 1))
  fi
}

# Remove every sshub profile on every peer, so each scenario starts from an
# empty ring. Peers keep their keys and sshd.
reset_peers() {
  for p in $PEERS; do
    psh "$p" 'rm -rf "$HOME/.config/sshub" "$HOME/.local/share/sshub"'
  done
}

# SAS helper: the one place that answers the pairing confirmation. Without a
# tty the remote side and the local prompt both read stdin, so answer "y" here.
# If the SAS flow changes, change only this function.
enroll_peer() {
  joiner=$1; host=$2
  printf 'y\n' | peer "$joiner" $SSHUB sync-join "$host"
}

# Run sync-now on a peer and print the JSON array. Exit status of sync-now is
# not an error here: failed devices are what some scenarios assert.
snow() {
  peer "$1" $SSHUB sync-now --format json 2>/dev/null || true
}

# Approve every pending change on an attended peer. Each id from the JSON list
# of sync-review is approved in turn; an empty queue approves nothing.
approve_all() {
  p=$1
  for id in $(peer "$p" $SSHUB sync-review --format json 2>/dev/null | pjq -r '.[].id' 2>/dev/null || true); do
    peer "$p" $SSHUB sync-review --approve "$id" || return 1
  done
}

# Evidence for the run log: sync status and the raw peers.json of every peer.
# Printed only, never asserted on. peers.json holds no secrets.
dump_state() {
  echo "=== state: $1"
  for p in $PEERS; do
    echo "--- $p sync-status"
    peer "$p" $SSHUB sync-status --format json 2>&1 || true
    echo "--- $p peers.json"
    psh "$p" 'find "$HOME" -path "*/sync/peers.json" -exec cat {} \;' 2>&1 || true
  done
}

# Finish: print the scenario marker and exit with the failure status.
finish() {
  if [ "$fail" -eq 0 ]; then
    echo "@@@SCENARIO $1 PASS"
  else
    echo "@@@SCENARIO $1 FAIL failures=$fail"
    exit 1
  fi
}
