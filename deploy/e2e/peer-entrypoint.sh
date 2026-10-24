#!/bin/sh
# Per-peer startup: fresh host and user keys, publish them on the shared /mesh
# volume, wait for every peer, then write authorized_keys and pinned known_hosts.
set -eu

: "${PEER_NAME:?PEER_NAME must name this peer (its compose service name)}"
: "${PEERS:?PEERS must list every peer name, space separated}"
MESH=/mesh
SSH_PORT=2222
H="$HOME"
ALL="$PEERS ${HOPS:-}"

# Fresh host key per container: the pinned known_hosts is the only trust anchor.
rm -f "$H/ssh/host_ed25519" "$H/ssh/host_ed25519.pub"
ssh-keygen -q -t ed25519 -N '' -C "$PEER_NAME-host" -f "$H/ssh/host_ed25519"
[ -f "$H/.ssh/id_ed25519" ] || ssh-keygen -q -t ed25519 -N '' -C "$PEER_NAME-user" -f "$H/.ssh/id_ed25519"

cp "$H/.ssh/id_ed25519.pub" "$MESH/$PEER_NAME.pub"
cut -d' ' -f1,2 "$H/ssh/host_ed25519.pub" > "$MESH/$PEER_NAME.hostkey"

# Wait (bounded) until every peer has published both keys.
i=0
while :; do
  missing=0
  for p in $ALL; do
    [ -s "$MESH/$p.pub" ] && [ -s "$MESH/$p.hostkey" ] || missing=1
  done
  [ "$missing" -eq 0 ] && break
  i=$((i + 1))
  [ "$i" -le 120 ] || { echo "peer-entrypoint: timed out waiting for mesh keys" >&2; exit 1; }
  sleep 1
done

: > "$H/.ssh/authorized_keys"
: > "$H/.ssh/known_hosts"
for p in $ALL; do
  cat "$MESH/$p.pub" >> "$H/.ssh/authorized_keys"
  echo "[$p]:$SSH_PORT $(cat "$MESH/$p.hostkey")" >> "$H/.ssh/known_hosts"
done
chmod 0600 "$H/.ssh/authorized_keys" "$H/.ssh/known_hosts"

cat > "$H/.ssh/config" <<EOF
Host $ALL
  Port $SSH_PORT
  User ci
  IdentityFile $H/.ssh/id_ed25519
  UserKnownHostsFile $H/.ssh/known_hosts
  BatchMode yes
  StrictHostKeyChecking yes
EOF
chmod 0600 "$H/.ssh/config"

cat > "$H/ssh/sshd_config" <<EOF
Port $SSH_PORT
ListenAddress 0.0.0.0
HostKey $H/ssh/host_ed25519
PidFile $H/ssh/sshd.pid
AuthorizedKeysFile $H/.ssh/authorized_keys
AllowUsers ci
PermitRootLogin no
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM no
StrictModes no
LogLevel VERBOSE
SetEnv DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent/bus
EOF

echo "peer-entrypoint: $PEER_NAME ready, sshd on $SSH_PORT"
# sshd daemonizes (pid in $H/ssh/sshd.pid) so scenarios can stop and restart it.
# This shell holds the container open; as PID 1 sshd would ignore SIGTERM.
/usr/sbin/sshd -e -f "$H/ssh/sshd_config"
exec sleep infinity
