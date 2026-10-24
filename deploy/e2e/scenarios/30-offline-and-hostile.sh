#!/bin/sh
# 30: one peer offline, and a hostile ssh config on the syncing peer. The
# healthy peer must still sync; the hostile config must not prompt or change state.
. "$E2E_DIR/lib.sh"
NAME=30-offline-and-hostile

reset_peers
check "peer-b joins peer-a's ring" "enroll_peer peer-b peer-a"
check "peer-c joins peer-a's ring" "enroll_peer peer-c peer-a"

# Offline: pause freezes peer-c. Its TCP accept still works in the kernel, so
# ssh may stall at the banner; the peer-side timeout bounds that wait.
check "pause peer-c" "$C pause peer-c"
start=$(date +%s)
out=$(peer peer-a timeout 60 $SSHUB sync-now --format json 2>/dev/null || true)
elapsed=$(( $(date +%s) - start ))
$C unpause peer-c >/dev/null 2>&1 || true

expect "peer-c reported failed while offline" "$out" '[.[] | select(.name=="peer-c") | .status] == ["failed"]'
expect "peer-b still ok while peer-c offline" "$out" '[.[] | select(.name=="peer-b") | .status] == ["ok"]'
check "offline sync bounded to 30 s (took ${elapsed}s)" "[ $elapsed -le 30 ]"

# Hostile config: an ssh config that turns off batch mode and offers password auth.
psh peer-a 'cp "$HOME/.ssh/config" "$HOME/.ssh/config.good"'
psh peer-a '{ printf "Host *\n  BatchMode no\n  PasswordAuthentication yes\n  KbdInteractiveAuthentication yes\n"; cat "$HOME/.ssh/config.good"; } > "$HOME/.ssh/config"'
before=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
out=$(peer peer-a timeout 60 $SSHUB sync-now --format json 2>&1 </dev/null || true)
psh peer-a 'mv "$HOME/.ssh/config.good" "$HOME/.ssh/config"'
after=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)

check "hostile config did not prompt for a password" "! printf '%s' \"\$out\" | grep -qi password"
expect "peer-b still ok under hostile config" "$out" '[.[] | select(.name=="peer-b") | .status] == ["ok"]'
expect "hostile sync left pending count unchanged" "[$before, $after]" '.[0].pending == .[1].pending'

finish "$NAME"
