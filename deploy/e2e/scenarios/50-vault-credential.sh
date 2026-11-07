#!/bin/sh
# 50: Vault mode. peer-a sets up the Vault with a scripted kit and master phrase
# (sync-import-kit is the headless founding path), stores a host with a
# password and migrates it into the ring. peer-b types the same kit, syncs,
# and opens the credential with the phrase. A bare copy of peer-b's data
# directory (no phrase) opens nothing.
. "$E2E_DIR/lib.sh"
. "$E2E_DIR/lib-creds.sh"
NAME=50-vault-credential

ring2
check "peer-a sets up the Vault" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-a $SSHUB sync-import-kit"
check "peer-a stores host vault1 with a password" "printf '%s\n' '$SECRET' | peer peer-a $SSHUB host add --name vault1 --address vault1.internal --password-stdin"
# With a Vault set up, `host add --password-stdin` already writes the replicated
# credential (a CredUpdate op) and leaves no local copy, so sync-migrate has nothing to move.
check "host add wrote a CredUpdate op" '[ "$(sq peer-a "SELECT count(*) FROM sync_ops WHERE kind = '"'"'CredUpdate'"'"'")" -ge 1 ]'
show "peer-a sync-migrate" peer peer-a $SSHUB sync-migrate --format json
expect "nothing is left to migrate" "$LAST_OUT" '.migrated == 0'
st=$(peer peer-a $SSHUB sync-status --format json 2>/dev/null || true)
expect "peer-a reports Vault mode" "$st" '.credential_mode == "vault"'

check "peer-b imports the kit" "printf '%s\n%s\n' '$KIT' '$PHRASE' | peer peer-b $SSHUB sync-import-kit"
out=$(snow peer-b)
expect "peer-b sync-now reaches peer-a ok" "$out" '[.[] | select(.name=="peer-a") | .status] == ["ok"]'
approve_all peer-b >/dev/null 2>&1 || true
check "the encrypted credential reached peer-b" '[ "$(sq peer-b "SELECT count(*) FROM sync_cred_state")" -ge 1 ]'
check "peer-b stores only ciphertext" '[ "$(sq peer-b "SELECT count(*) FROM sync_cred_state WHERE instr(ciphertext, '"'"'$SECRET'"'"') > 0")" -eq 0 ]'
# Desired: the host itself replicates. Product gap P1: nothing turns a CLI host change into a HostUpsert op.
check "vault1 reached peer-b" "peer peer-b $SSHUB host list 2>/dev/null | grep -q vault1"

show "peer-b host resolve vault1 (phrase on stdin)" sh -c "printf '%s\n' '$PHRASE' | $C exec -T peer-b $SSHUB host resolve vault1 --format json"
expect "peer-b opens the credential with the phrase" "$LAST_OUT" '[.. | .has_stored_secret? // empty] | any'

# Plaintext equality needs the export ceremony: scenario 95 compares the CSV.

psh peer-b 'rm -rf /tmp/copy && mkdir -p /tmp/copy/.local/share && cp -a "$HOME/.local/share/sshub" /tmp/copy/.local/share/'
show "a copy of peer-b's data dir alone, no phrase" psh peer-b 'HOME=/tmp/copy sshub --profile default host resolve vault1 --format json </dev/null'
CP_RC=$LAST_RC; CP_OUT=$LAST_OUT
check "the copy opens nothing" '[ "$CP_RC" -ne 0 ] || ! printf "%s" "$CP_OUT" | pjq -e "[.. | .has_stored_secret? // empty] | any" >/dev/null'
check "the copy holds no plaintext password" "! psh peer-b 'grep -rF \"$SECRET\" /tmp/copy' >/dev/null 2>&1"
check "peer-b's own store holds no plaintext password either" "! psh peer-b 'grep -rF \"$SECRET\" \"\$HOME/.local/share/sshub\"' >/dev/null 2>&1"
check "peer-a keeps no plaintext copy after host add either" "! psh peer-a 'grep -rF \"$SECRET\" \"\$HOME/.local/share/sshub\"' >/dev/null 2>&1"
psh peer-b 'rm -rf /tmp/copy'
dump_state "after vault"

finish "$NAME"
