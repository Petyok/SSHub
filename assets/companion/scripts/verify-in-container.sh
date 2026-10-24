#!/bin/sh
# Everything the phone page needs verified, in the order it is meant to run. Runs inside the
# tools container (gpu-verify/tools.sh), where /work is the checkout and node, Playwright and
# cargo are installed:
#   gpu-verify/tools.sh <worktree> <name> -- 'sh /work/assets/companion/scripts/verify-in-container.sh'
# Prints one RC line per step; exits 1 when any step failed.
cd /work/assets/companion || exit 1
export NODE_PATH=/opt/node/lib/node_modules
fail=0
step() { name=$1; shift; "$@" >"/tmp/$name.txt" 2>&1; rc=$?; echo "STEP $name rc=$rc"; cat "/tmp/$name.txt"; [ $rc -eq 0 ] || fail=1; }

step npm-ci npm ci --no-audit --no-fund
step link node scripts/link-vendor.mjs
step build node build.mjs
cp sshub-companion.html /tmp/build1.html
step build-again node build.mjs
cmp sshub-companion.html /tmp/build1.html && echo "REPRODUCIBLE yes" || { echo "REPRODUCIBLE no"; fail=1; }
step build-selftest node build.mjs --selftest
step node-tests node --test test/phone.test.mjs test/phone2.test.mjs test/quorum.test.mjs test/replay.test.js
step headless-replay node scripts/headless-check.mjs
rm -rf /tmp/e2e
step e2e env E2E_OUT=/tmp/e2e node scripts/e2e.mjs
step rust-verify sh -c 'cd /work && SSHUB_PHONE_E2E_DIR=/tmp/e2e cargo test --lib sync::phone_fixtures -- --nocapture'
exit $fail
