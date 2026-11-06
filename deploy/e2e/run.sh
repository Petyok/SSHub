#!/bin/sh
# run.sh: bring up the host-sync e2e stack, run every scenario in order, dump
# compose logs, always tear the project down. Exit status = failed scenario count.
# Runs on gpu-serv via gpu-verify/e2e.sh; uses only the compose project below.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
cd "$HERE" || exit 2
P=sshub-hostsync-e2e
# The phone's browser: desktop (headless Chromium in the phone service) or android (Chrome on the emulator service).
# A file next to this script picks it, because the runner passes no environment (gpu-verify/e2e.sh).
PHONE_TARGET=$(cat "$HERE/phone-target" 2>/dev/null || echo desktop)
export PHONE_TARGET
# An android-target run takes only e2e.lock (held by gpu-verify/e2e.sh): its emulator lives in this compose project.
C="docker compose -p $P -f docker-compose.yml"
[ "$PHONE_TARGET" = android ] && C="$C --profile android"
PEERS="peer-a peer-b peer-c hub"
E2E_DIR=$HERE
export E2E_DIR C PEERS

cleanup() {
  $C down -v --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

cleanup
if [ "$PHONE_TARGET" = android ]; then
  echo "$(date -u +%H:%M:%S) compose up --build android (the emulator boots first, the phone driver attaches to it)"
  $C up --build -d android || { echo "compose up android failed"; exit 1; }
  i=0
  until [ "$($C exec -T android adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ]; do
    i=$((i + 1))
    [ "$i" -le 120 ] || { echo "emulator not booted after 10 min"; $C logs --no-color android | tail -n 40; exit 1; }
    sleep 5
  done
  echo "$(date -u +%H:%M:%S) emulator booted"
fi
echo "$(date -u +%H:%M:%S) compose up --build"
# --no-recreate: the second up must not restart the booted emulator (the first up of an android run created it)
$C up --build -d --no-recreate || { echo "compose up failed"; exit 1; }

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
