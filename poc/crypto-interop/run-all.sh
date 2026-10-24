#!/bin/sh
# PoC A full run: Rust unit tests, vector emission, JS replay (A1), JS blob
# production and Rust verification (A2), Python oracles. Needs cargo, node and
# the oracle venv (the tools container provides all three).
# Usage: sh run-all.sh   (from anywhere; CARGO_TARGET_DIR defaults to /tmp/rig)
set -u
cd "$(dirname "$0")"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/rig}"
rc=0
step() {
  name=$1; shift
  echo "=== $name"
  if "$@"; then echo "@@@ $name PASS"; else echo "@@@ $name FAIL"; rc=1; fi
}
sh ./setup-node.sh
step rust-unit cargo test -q
step emit-vectors cargo run -q --bin emit-vectors
step js-replay node --test test/interop.test.js
step emit-js-blobs node emit-js-blobs.node.mjs
step verify-js-blobs cargo run -q --bin verify-js-blobs
step build-single-file sh build-single-file.sh
step browser-test node browser-test.mjs 5
step verify-browser-blobs cargo run -q --bin verify-js-blobs -- blobs-browser.json
if [ -d oracles ]; then
  step oracle-browser-blobs python3 oracles/oracle_vectors.py vectors.json blobs-browser.json

  for f in oracles/*.py; do step "oracle $(basename "$f")" python3 "$f"; done
fi
exit $rc
