#!/bin/sh
# Installs a pinned esbuild outside the repo and builds the single-file page.
set -eu
cd "$(dirname "$0")"
export ESBUILD_DIR="${ESBUILD_DIR:-/tmp/esbuild-tool}"
if [ ! -d "$ESBUILD_DIR/node_modules/esbuild" ]; then
  mkdir -p "$ESBUILD_DIR"
  (cd "$ESBUILD_DIR" && npm install --no-audit --no-fund --silent esbuild@0.25.12)
fi
node build-single-file.mjs
