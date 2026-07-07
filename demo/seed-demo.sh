#!/usr/bin/env bash
# Reset and seed demo/home/.local/share/sshub/launcher.db for VHS tapes.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export SSHUB_DATA_DIR="${SSHUB_DATA_DIR:-$ROOT/demo/home/.local/share/sshub}"
export SSHUB_SSH_CONFIG="${SSHUB_SSH_CONFIG:-$ROOT/demo/home/ssh_config}"
mkdir -p "$SSHUB_DATA_DIR"
cargo run --quiet --bin seed-demo --manifest-path "$ROOT/Cargo.toml"
