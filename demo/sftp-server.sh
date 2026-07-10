#!/usr/bin/env bash
# Throwaway rootless SSH/SFTP server for recording the SFTP demo.
#
# SSHub's SFTP browser talks native libssh2 to a real server (unlike the
# embedded shell sessions, which the fake demo/bin/ssh can stand in for). So to
# record the SFTP tape we need an actual sshd. This spins one up entirely as the
# current user — no root — bound to 127.0.0.1:2222, with:
#   * an ephemeral host key + client key generated under demo/home/.ssh
#   * pubkey-only auth (the demo client key is the sole authorized key)
#   * ForceCommand internal-sftp -d <sandbox>, so every login lands in a curated
#     directory of believable files instead of the recorder's real $HOME.
#
# Usage: demo/sftp-server.sh start | stop | status
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

PORT="${SSHUB_SFTP_DEMO_PORT:-2222}"
SSHDIR="$ROOT/demo/home/.ssh"
SANDBOX="$ROOT/demo/home/sftp-remote"
LOCALBOX="$ROOT/demo/home/sftp-local"
HOSTKEY="$SSHDIR/demo_host_ed25519"
CLIENTKEY="$SSHDIR/demo_client_ed25519"
AUTHKEYS="$SSHDIR/authorized_keys"
SSHD_CONF="$SSHDIR/sshd_config"
PIDFILE="$SSHDIR/sshd.pid"

# sshd lives in different places across distros; probe the usual spots.
SSHD_BIN="$(command -v sshd || echo /usr/sbin/sshd)"
[ -x "$SSHD_BIN" ] || SSHD_BIN=/usr/bin/sshd

seed_keys() {
    mkdir -p "$SSHDIR" "$SANDBOX"
    chmod 700 "$SSHDIR"
    [ -f "$HOSTKEY" ]   || ssh-keygen -q -t ed25519 -f "$HOSTKEY"   -N "" -C "sshub-demo-host"
    [ -f "$CLIENTKEY" ] || ssh-keygen -q -t ed25519 -f "$CLIENTKEY" -N "" -C "sshub-demo-client"
    cp "$CLIENTKEY.pub" "$AUTHKEYS"
    chmod 600 "$AUTHKEYS" "$CLIENTKEY"

    # A curated remote tree so the browser shows something believable.
    rm -rf "$SANDBOX"
    mkdir -p "$SANDBOX/releases" "$SANDBOX/logs"
    printf '# production web server\n' > "$SANDBOX/README.md"
    printf 'server { listen 443 ssl; server_name sshub.dev; }\n' > "$SANDBOX/nginx.conf"
    head -c 24576 /dev/zero | tr '\0' '=' > "$SANDBOX/app.log"
    head -c 65536 /dev/urandom > "$SANDBOX/backup.tar.gz"
    printf 'v1.4.0\n' > "$SANDBOX/releases/CURRENT"
    printf 'access log\n' > "$SANDBOX/logs/access.log"

    # A curated LOCAL side too, so the right pane shows a tidy deploy folder
    # instead of the recorder's working tree. The tape cd's here before launch.
    rm -rf "$LOCALBOX"
    mkdir -p "$LOCALBOX"
    printf '#!/usr/bin/env bash\necho deploying...\n' > "$LOCALBOX/deploy.sh"
    printf 'env: production\nreplicas: 3\n' > "$LOCALBOX/config.yaml"
    printf '# ops runbook\n' > "$LOCALBOX/runbook.md"
    head -c 4096 /dev/urandom > "$LOCALBOX/tls-cert.pem"
}

write_conf() {
    cat > "$SSHD_CONF" <<EOF
Port $PORT
ListenAddress 127.0.0.1
HostKey $HOSTKEY
PidFile $PIDFILE
AuthorizedKeysFile $AUTHKEYS
PasswordAuthentication no
PubkeyAuthentication yes
UsePAM no
StrictModes no
PrintMotd no
AllowUsers $USER
Subsystem sftp internal-sftp
ForceCommand internal-sftp -d $SANDBOX
EOF
}

start() {
    stop 2>/dev/null || true
    seed_keys
    write_conf
    # This is a throwaway server on a loopback port; drop any prior host-key
    # entry so SSHub's trust-on-first-use re-records the current ephemeral key
    # instead of tripping its "host key changed" (MITM) guard on a stale line.
    if [ -f "$HOME/.ssh/known_hosts" ]; then
        ssh-keygen -R "[127.0.0.1]:$PORT" -f "$HOME/.ssh/known_hosts" >/dev/null 2>&1 || true
    fi
    # -D keeps sshd in the foreground; background it ourselves so the tape's
    # shell keeps going. -e sends logs to stderr (silenced).
    "$SSHD_BIN" -f "$SSHD_CONF" -D -e >/dev/null 2>&1 &
    echo $! > "$PIDFILE.wrapper"
    sleep 0.5
    echo "sftp demo server on 127.0.0.1:$PORT (user $USER, key $CLIENTKEY)"
}

stop() {
    for pf in "$PIDFILE" "$PIDFILE.wrapper"; do
        [ -f "$pf" ] && kill "$(cat "$pf")" 2>/dev/null || true
        rm -f "$pf"
    done
    # Belt and suspenders: kill any sshd we spawned for this config.
    pkill -f "sshd -f $SSHD_CONF" 2>/dev/null || true
}

status() {
    if pgrep -f "sshd -f $SSHD_CONF" >/dev/null; then
        echo "running on 127.0.0.1:$PORT"
    else
        echo "stopped"
    fi
}

case "${1:-start}" in
    start)  start ;;
    stop)   stop ;;
    status) status ;;
    *) echo "usage: $0 start|stop|status" >&2; exit 1 ;;
esac
