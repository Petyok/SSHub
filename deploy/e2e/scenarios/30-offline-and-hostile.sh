#!/bin/sh
# 30: peer-a founds the ring with hub, joins peer-b and peer-c again directly so
# it records their hosts, then peer-c's sshd is stopped. The offline peer must be
# reported failed within 5 s while peer-b still syncs. A hostile ssh config on
# the syncing peer must not prompt or change state.
. "$E2E_DIR/lib.sh"
NAME=30-offline-and-hostile

reset_peers
check "peer-a founds the ring with hub" "enroll_peer peer-a hub"
check "peer-b joins via hub" "enroll_peer peer-b hub"
check "peer-c joins via hub" "enroll_peer peer-c hub"
check "peer-a approves admissions of peer-b and peer-c" "approve_all peer-a"
check "peer-a records peer-b directly" "enroll_peer peer-a peer-b"
check "peer-a records peer-c directly" "enroll_peer peer-a peer-c"
dump_state "after direct enrollment"

# Offline: stop peer-c's sshd (pid file written by sshd), bounded at 5 s.
check "stop peer-c sshd" "psh peer-c 'kill \"\$(cat \"\$HOME/ssh/sshd.pid\")\"'"
start=$(date +%s)
out=$(peer peer-a timeout 5 $SSHUB sync-now --format json 2>/dev/null || true)
elapsed=$(( $(date +%s) - start ))
check "restart peer-c sshd" "psh peer-c '/usr/sbin/sshd -e -f \"\$HOME/ssh/sshd_config\"'"

expect "peer-c reported failed while offline" "$out" '[.[] | select(.name=="peer-c") | .status] == ["failed"]'
expect "peer-b still ok while peer-c offline" "$out" '[.[] | select(.name=="peer-b") | .status] == ["ok"]'
check "offline peer detected within 5 s (took ${elapsed}s)" "[ $elapsed -le 5 ]"
dump_state "after offline sync"

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
dump_state "after hostile config"

finish "$NAME"
