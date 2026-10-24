#!/bin/sh
# Links the vendored JS libraries into node_modules so their bare imports
# ('@noble/hashes' and friends) resolve without npm and without a network.
# node_modules is not committed (and not synced to the build container), so
# every test run starts with this script.
set -eu
cd "$(dirname "$0")"
mkdir -p node_modules/@noble node_modules/@scure
for p in ciphers:noble-ciphers-2.4.0 curves:noble-curves-2.4.0 hashes:noble-hashes-2.4.0 post-quantum:noble-post-quantum-0.7.1; do
  ln -sfn "../../vendor/${p#*:}" "node_modules/@noble/${p%%:*}"
done
ln -sfn ../../vendor/scure-bip39-2.4.0 node_modules/@scure/bip39
ln -sfn ../vendor/hash-wasm-4.12.0 node_modules/hash-wasm
