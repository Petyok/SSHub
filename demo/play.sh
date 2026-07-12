#!/usr/bin/env bash
# Launch SSHub against the demo fixtures so you can click around by hand.
#
# Uses an isolated data dir (so it never clobbers the DB used while recording
# GIFs) and prepends demo/bin to PATH, so "connecting" to any host shows the
# simulated session instead of a real ssh.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

export SSHUB_VERSION_LABEL=""
export SSHUB_CONFIG_DIR="$ROOT/demo/home/.config/sshub"
export SSHUB_DATA_DIR="${SSHUB_PLAY_DATA_DIR:-$ROOT/demo/home/.local/share/sshub-play}"
export SSHUB_SSH_CONFIG="$ROOT/demo/home/ssh_config"
export PATH="$ROOT/demo/bin:$ROOT/target/release:$PATH"

mkdir -p "$SSHUB_DATA_DIR"
cargo build --release --quiet
bash "$ROOT/demo/seed-demo.sh"
exec sshub
