#!/bin/sh
# 00-smoke: every peer reaches every other peer over key-only ssh and runs
# `sshub --version`; a mail sent through the peer sendmail arrives in mailpit.
# Prints @@@SCENARIO 00-smoke PASS|FAIL and exits non-zero on failure.
set -u
HERE=$(cd "$(dirname "$0")/.." && pwd)
C=${C:-docker compose -p sshub-hostsync-e2e -f "$HERE/docker-compose.yml"}
PEERS=${PEERS:-"peer-a peer-b peer-c hub"}
NAME=00-smoke
fail=0

for src in $PEERS; do
  for dst in $PEERS; do
    [ "$src" = "$dst" ] && continue
    if out=$($C exec -T "$src" ssh -o BatchMode=yes -o StrictHostKeyChecking=yes "$dst" sshub --version 2>&1) \
       && echo "$out" | grep -q '^sshub '; then
      echo "ok   $src -> $dst: $out"
    else
      echo "FAIL $src -> $dst: $out"
      fail=$((fail + 1))
    fi
  done
done

run_id="$(date -u +%s)-$$"
subject="e2e-smoke-$run_id"
printf 'From: ci@peer-a\nTo: ops@e2e.test\nSubject: %s\n\nsmoke\n' "$subject" \
  | $C exec -T peer-a /usr/sbin/sendmail -t
if [ $? -ne 0 ]; then
  echo "FAIL sendmail from peer-a exited non-zero"
  fail=$((fail + 1))
else
  found=0
  i=0
  while [ "$i" -lt 20 ]; do
    if $C exec -T peer-a curl -fsS http://mailpit:8025/api/v1/messages 2>/dev/null | grep -q "$subject"; then
      found=1
      break
    fi
    i=$((i + 1))
    sleep 1
  done
  if [ "$found" -eq 1 ]; then
    echo "ok   mail '$subject' visible in mailpit"
  else
    echo "FAIL mail '$subject' not found in mailpit"
    fail=$((fail + 1))
  fi
fi

if [ "$fail" -eq 0 ]; then
  echo "@@@SCENARIO $NAME PASS"
else
  echo "@@@SCENARIO $NAME FAIL failures=$fail"
  exit 1
fi
