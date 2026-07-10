#!/usr/bin/env bash
# Reset and seed demo/home/.local/share/sshub/launcher.db for VHS tapes.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export SSHUB_DATA_DIR="${SSHUB_DATA_DIR:-$ROOT/demo/home/.local/share/sshub}"
export SSHUB_SSH_CONFIG="${SSHUB_SSH_CONFIG:-$ROOT/demo/home/ssh_config}"
mkdir -p "$SSHUB_DATA_DIR"

# If the SFTP demo server's client key exists, hand the seeder the details so it
# adds a real 127.0.0.1:2222 host for the SFTP-browser demo (see
# demo/sftp-server.sh). Harmless when absent — the seeder just skips that host.
CLIENTKEY="$ROOT/demo/home/.ssh/demo_client_ed25519"
if [ -f "$CLIENTKEY" ]; then
    export SSHUB_DEMO_SFTP_KEY="$CLIENTKEY"
    export SSHUB_DEMO_SFTP_USER="${SSHUB_DEMO_SFTP_USER:-$USER}"
fi

cargo run --quiet --example seed-demo --manifest-path "$ROOT/Cargo.toml"
