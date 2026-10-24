#!/bin/sh
# run.sh: bring up the host-sync e2e stack, run every scenario in order, dump
# compose logs, always tear the project down. Exit status = failed scenario count.
# Runs on gpu-serv via gpu-verify/e2e.sh; uses only the compose project below.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
cd "$HERE" || exit 2
P=sshub-hostsync-e2e
C="docker compose -p $P -f docker-compose.yml"
PEERS="peer-a peer-b peer-c hub"
export C PEERS

cleanup() {
  $C down -v --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

cleanup
echo "$(date -u +%H:%M:%S) compose up --build"
$C up --build -d || { echo "compose up failed"; exit 1; }

echo "$(date -u +%H:%M:%S) waiting for sshd on every peer and mailpit"
for s in $PEERS; do
  i=0
  until $C exec -T "$s" sh -c 'ssh-keyscan -p 2222 127.0.0.1 2>/dev/null | grep -q ssh-ed25519'; do
    i=$((i + 1))
    [ "$i" -le 60 ] || { echo "sshd not answering on $s"; $C logs --no-color "$s" | tail -n 40; exit 1; }
    sleep 1
  done
  echo "sshd ready: $s"
done
i=0
until $C exec -T peer-a curl -fsS http://mailpit:8025/api/v1/info >/dev/null 2>&1; do
  i=$((i + 1))
  [ "$i" -le 60 ] || { echo "mailpit not answering"; exit 1; }
  sleep 1
done
echo "mailpit ready"

fail=0
for s in "$HERE"/scenarios/*.sh; do
  name=$(basename "$s" .sh)
  echo "$(date -u +%H:%M:%S) scenario $name"
  if ! sh "$s"; then
    fail=$((fail + 1))
  fi
done

echo "$(date -u +%H:%M:%S) compose logs"
$C logs --no-color --timestamps
echo "failed scenarios: $fail"
exit "$fail"
