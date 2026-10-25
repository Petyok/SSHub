#!/bin/sh
# phone-selftest.sh: the driver against a Red notice made in this script, no compose, no peers. Runs in the tools
# image (node, Playwright), e.g.
#   gpu-verify/tools.sh <checkout> phone-selftest -- 'sh /work/deploy/e2e/phone-selftest.sh'
# It proves the phone half only: restore, describe a Red notice from its bytes, sign RedPhoneSign, text equals QR.
set -eu
W=${W:-/work}
export NODE_PATH=${NODE_PATH:-/opt/node/lib/node_modules}
export PHONE_LIB=$W/assets/companion PHONE_PAGE_DIR=$W/deploy/e2e/phone-page PHONE_PROFILE=$(mktemp -d)/profile PHONE_PORT=7790
(cd "$W/assets/companion" && node scripts/link-vendor.mjs >/dev/null)
D="node $W/deploy/e2e/phone-driver.mjs"
$D serve >/tmp/phone-daemon.log 2>&1 &
trap 'kill $! 2>/dev/null || true' EXIT
i=0; until $D info >/dev/null 2>&1; do i=$((i + 1)); [ "$i" -le 60 ] || { cat /tmp/phone-daemon.log; exit 1; }; sleep 1; done
echo "info:  $($D info)"
echo "setup: $($D setup)"
# A Red op (DeviceRevoke) signed by a throw-away key the phone has never met, dated now.
node --input-type=module -e "
import { pathToFileURL } from 'node:url';
const l = await import(pathToFileURL(process.env.PHONE_LIB + '/src/lib.mjs').href);
const k = await import(pathToFileURL(process.env.PHONE_LIB + '/src/keys.mjs').href);
const id = k.identityOfSecret(new Uint8Array(32).fill(9));
const now = Math.floor(Date.now() / 1000);
const body = { device_id: id.deviceId, seq: 1, lamport: 1, prev_hash: '00'.repeat(32), timestamp: now, epoch: 0, alg: [1, 1], body: { DeviceRevoke: { device_id: 'aaaaaaaaaaaaaaaa' } } };
const canonical = l.utf8(l.canonicalJSON(body));
const sig = l.edSign(id.edSeed, canonical);
const hex = (u) => Buffer.from(u).toString('hex');
console.log(JSON.stringify({ canonicalHex: hex(canonical), signatureHex: hex(sig) }));
" > /tmp/op.json
F=$($D frame-notice "$(cat /tmp/op.json)" | sed 's/.*"frame":"\([^"]*\)".*/\1/')
OPEN=$($D red-open "{\"frame\":\"$F\"}")
echo "open:  $OPEN" | cut -c1-400
HASH=$(printf '%s' "$OPEN" | sed 's/.*"op_hash":"\([0-9a-f]*\)".*/\1/')
APPR=$($D red-approve)
echo "approve: $APPR" | cut -c1-200
OPS=$(printf '%s' "$APPR" | sed 's/.*"ops":"\([^"]*\)".*/\1/')
VER=$($D verify-ops "{\"ops\":\"$OPS\",\"kind\":\"RedPhoneSign\",\"opHash\":\"$HASH\"}" || true)
echo "verify: $VER"
printf '%s' "$APPR" | grep -q '"qr_equals_text":true'
printf '%s' "$VER" | grep -q '"ok":true'
echo "PHONE SELFTEST PASS"
