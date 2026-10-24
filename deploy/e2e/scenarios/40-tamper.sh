#!/bin/sh
# 40: peer-a's stored signed ops are corrupted at rest. peer-b must reject them
# (signature check fails) and must not apply the change.
. "$E2E_DIR/lib.sh"
NAME=40-tamper

reset_peers
check "peer-b joins peer-a's ring" "enroll_peer peer-b peer-a"
check "peer-a adds host web40" "peer peer-a $SSHUB host add --name web40 --address web40.internal"

# Overwrite the first byte of every locally authored canonical payload on peer-a.
# zeroblob(1) keeps the SQL free of single quotes so it survives the shell.
tamper='db=$(find "$HOME/.local/share/sshub" -name launcher.db | head -n1)
[ -n "$db" ] || { echo "no launcher.db"; exit 1; }
sqlite3 "$db" "UPDATE sync_ops SET canonical = zeroblob(1) || substr(canonical, 2) WHERE local = 1;"
test "$(sqlite3 "$db" "SELECT count(*) FROM sync_ops WHERE local = 1;")" -gt 0'
if psh peer-a "$tamper"; then
  echo "ok   tampered peer-a local ops"
else
  echo "FAIL tamper peer-a local ops"
  fail=$((fail + 1))
fi

out=$(snow peer-b)
expect "peer-b rejects or fails the tampered ops" "$out" '[.[] | select(.name=="peer-a") | ((.rejected // 0) >= 1) or (.status == "failed")] | any'
check "web40 not applied on peer-b" "! peer peer-b $SSHUB host list 2>/dev/null | grep -q web40"

finish "$NAME"
