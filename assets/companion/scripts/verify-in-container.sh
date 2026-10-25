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

# The committed page and its fingerprint (the container has no .git): the build must reproduce them byte for byte.
cp sshub-companion.html /tmp/committed.html
cp FINGERPRINT /tmp/committed.fingerprint
step npm-ci npm ci --no-audit --no-fund
step link node scripts/link-vendor.mjs
step build node build.mjs
cmp sshub-companion.html /tmp/committed.html && cmp FINGERPRINT /tmp/committed.fingerprint && echo "COMMITTED EQUALS BUILD yes" || { echo "COMMITTED EQUALS BUILD no (rebuild and commit sshub-companion.html and FINGERPRINT)"; fail=1; }
cp sshub-companion.html /tmp/build1.html
step build-again node build.mjs
cmp sshub-companion.html /tmp/build1.html && echo "REPRODUCIBLE yes" || { echo "REPRODUCIBLE no"; fail=1; }
step build-selftest node build.mjs --selftest
step node-tests node --test test/phone.test.mjs test/phone2.test.mjs test/phone3.test.mjs test/review-app-round1.test.mjs test/quorum.test.mjs test/replay.test.js
step headless-replay node scripts/headless-check.mjs
# The Rust terminal of the enrollment check (phone_fixtures e2e_enrollment_peer_for_the_page) runs next to the page flow
# and exchanges the enrollment frames with it as files; compile first so it is ready when the page starts.
step rust-build sh -c 'cd /work && cargo test --lib --no-run'
rm -rf /tmp/e2e
mkdir -p /tmp/e2e/enroll-peer
( cd /work && SSHUB_PHONE_ENROLL_PEER=/tmp/e2e/enroll-peer cargo test --lib sync::phone_fixtures::e2e_enrollment_peer_for_the_page -- --exact --nocapture >/tmp/enroll-peer.txt 2>&1; echo $? >/tmp/enroll-peer.rc ) &
peer=$!
step e2e env E2E_OUT=/tmp/e2e E2E_ENROLL_PEER=/tmp/e2e/enroll-peer node scripts/e2e.mjs
wait $peer
echo "STEP enroll-peer rc=$(cat /tmp/enroll-peer.rc)"
cat /tmp/enroll-peer.txt
[ "$(cat /tmp/enroll-peer.rc)" = 0 ] || fail=1
step rust-verify sh -c 'cd /work && SSHUB_PHONE_E2E_DIR=/tmp/e2e cargo test --lib sync::phone_fixtures -- --nocapture'
exit $fail
