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

mkdir -p "$SSHUB_DATA_DIR"
# Same build as demo/record.py (see its build()): the shared target dir can hold
# another worktree's binary, so touch the crate roots, and ask cargo where the
# target dir is instead of assuming ./target.
touch "$ROOT/src/lib.rs" "$ROOT/src/main.rs"
cargo build --release --quiet --no-default-features --manifest-path "$ROOT/Cargo.toml"
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps --manifest-path "$ROOT/Cargo.toml" \
    | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
export PATH="$ROOT/demo/bin:$TARGET_DIR/release:$PATH"
bash "$ROOT/demo/seed-demo.sh"
exec sshub
