#!/bin/sh
# 45: a synced identity key K, held in the Vault, unlocks on peer-b and reaches
# the destination dest through the jump host jump. K's public key is trusted by
# jump and dest only. The key file exists only in XDG_RUNTIME_DIR for one exec;
# afterwards no key bytes may remain on peer-b, and K is gone from jump and dest.
. "$E2E_DIR/lib.sh"
NAME=45-hop-synced-key

# Test fixtures, not real credentials (same values as the Vault scenarios).
KIT='sshub:ring:1:00112233445566778899aabbccddeeff'
PHRASE='abacus abdomen abdominal abide abiding ability'
RT=/tmp/xdg-45
KEYTAG=e2e-45-K

trust() { printf '%s\n' "$PUB" | peer "$1" sh -c 'cat >> "$HOME/.ssh/authorized_keys"'; }
cnt() { peer peer-b sh -c 'sqlite3 "$(find "$HOME/.local/share/sshub" -name launcher.db | head -n1)" "$1"' sh "$1"; }
run_dest() { printf '%s\n' "$1" | $C exec -T peer-b env XDG_RUNTIME_DIR=$RT $SSHUB exec dest -- hostname; }
disk_hits() { psh peer-b "grep -rlF '$BODY' \"\$HOME/.local/share/sshub\" \"\$HOME/.config/sshub\" /tmp 2>/dev/null"; }
pem_hits() { psh peer-b "grep -rl 'PRIVATE KEY' \"\$HOME/.local/share/sshub\" \"\$HOME/.config/sshub\" $RT 2>/dev/null"; }
rt_empty() { psh peer-b "test -z \"\$(ls -A $RT 2>/dev/null)\""; }

reset_peers
check "peer-a founds the ring with hub" "enroll_peer peer-a hub"
check "peer-b joins via hub" "enroll_peer peer-b hub"
check "peer-a approves peer-b's admission" "approve_all peer-a"
check "peer-b records peer-a directly" "enroll_peer peer-b peer-a"
check "peer-a sets up the Vault" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-a $SSHUB sync-import-kit"
check "peer-b imports the same kit" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-b $SSHUB sync-import-kit"

# K is made on peer-a and added as an identity. Its file is deleted afterwards,
# so K lives only in the Vault.
check "peer-a makes key K" "psh peer-a 'rm -f /tmp/k45 /tmp/k45.pub && ssh-keygen -q -t ed25519 -N \"\" -C $KEYTAG -f /tmp/k45'"
PUB=$(psh peer-a 'cat /tmp/k45.pub')
BODY=$(psh peer-a 'sed -n 2p /tmp/k45')
check "K public key and a body line were captured" '[ -n "$PUB" ] && [ -n "$BODY" ]'
check "peer-a adds identity K from the key file" "peer peer-a $SSHUB identity add --name K --username ci --private-key /tmp/k45"
check "peer-a deletes the local key file" "psh peer-a 'rm -f /tmp/k45 /tmp/k45.pub'"

out=$(snow peer-b)
expect "peer-b sync-now reaches peer-a ok" "$out" '[.[] | select(.name=="peer-a") | .status] == ["ok"]'
check "peer-b approves the pending sync changes" "approve_all peer-b"
check "the encrypted credential reached peer-b" '[ "$(cnt "SELECT count(*) FROM sync_cred_state")" -ge 1 ]'
expect "identity K is known on peer-b" "$(peer peer-b $SSHUB identity show K --format json 2>/dev/null)" '.name == "K"'

check "jump accepts K" "trust jump"
check "dest accepts K" "trust dest"
check "peer-b records jump as a host with K" "peer peer-b $SSHUB host add --name jump --address jump --username ci --port 2222 --identity K"
check "peer-b records dest through jump with K" "peer peer-b $SSHUB host add --name dest --address dest --username ci --port 2222 --identity K --proxy-jump jump"

psh peer-b "rm -rf $RT && mkdir -p $RT && chmod 700 $RT"
check "exec dest without the phrase fails: K stays locked" '! run_dest "" >/dev/null 2>&1'

JIP=$(psh peer-b 'getent hosts jump | cut -d" " -f1')
check "peer-b resolves jump's address" '[ -n "$JIP" ]'
OUT=$(run_dest "$PHRASE" 2>&1)
[ "$OUT" = dest ] || { echo "--- exec output:"; echo "$OUT" | sed 's/^/     | /'; }
check "exec dest with the phrase reaches dest" '[ "$OUT" = dest ]'
check "dest saw the connection from jump's address (hop)" "$C logs --no-color dest 2>&1 | grep -q 'Accepted publickey for ci from $JIP '"

check "no key bytes on peer-b's disk after exec" '[ -z "$(disk_hits)" ]'
check "no PRIVATE KEY block in sshub's dirs or runtime dir" '[ -z "$(pem_hits)" ]'
check "the runtime key directory is empty after exec" rt_empty

psh jump "sed -i '/$KEYTAG/d' \"\$HOME/.ssh/authorized_keys\""
psh dest "sed -i '/$KEYTAG/d' \"\$HOME/.ssh/authorized_keys\""
check "K is removed from jump and dest" '! psh jump "grep -q $KEYTAG \"\$HOME/.ssh/authorized_keys\"" && ! psh dest "grep -q $KEYTAG \"\$HOME/.ssh/authorized_keys\""'
psh peer-b "rm -rf $RT"

finish "$NAME"
