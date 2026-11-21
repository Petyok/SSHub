// Round 3 browser reproductions (real Chromium against the built page). Run in the tools container:
//   cd /work/assets/companion && NODE_PATH=/opt/node/lib/node_modules node --test test/review-app-round3-browser.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { canonicalJSON, edSign, utf8 } from '../src/lib.mjs';
import { identityOfSecret, secretToWords } from '../src/keys.mjs';
import { b64enc } from '../src/ops.mjs';
import { adoptOwnChain, appendHeadAttest, courierOps, emptyState } from '../src/state.mjs';
import { encodeFrame, encodePhoneOps, KIND } from '../src/frames.mjs';
import { cutFrame } from '../src/ble.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const { chromium } = require('playwright');
const body = fs.readFileSync(path.resolve(here, '..', 'sshub-companion.html'));
const PIN = 'correct-pin-17';

async function withPage(fn, { fakeBle = false } = {}) {
  const server = http.createServer((req, res) => (req.url === '/' ? res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' }).end(body) : res.writeHead(404).end()));
  await new Promise((r) => server.listen(0, 'localhost', r));
  const browser = await chromium.launch();
  try {
    const context = await browser.newContext();
    if (fakeBle) {
      await context.addInitScript(`(() => {
        window.__bleWrites = [];
        let notify = null;
        window.__emit = (a) => notify({ target: { value: new DataView(new Uint8Array(a).buffer) } });
        const tx = { addEventListener: (n, f) => { notify = f; }, startNotifications: async () => {} };
        const rx = { properties: { writeWithoutResponse: true }, writeValueWithoutResponse: async (b) => {
          const m = Array.from(new Uint8Array(b));
          window.__bleWrites.push(m);
          if (m[0] === 1) setTimeout(() => window.__emit([2, 1, 0, 244, 8]), 0);
        } };
        const info = { readValue: async () => new DataView(new Uint8Array([1, 0, 16, 0, 0, 55, 81, 75, 50]).buffer) };
        const chars = { 'b62f7d3f-0002-42dd-8371-6ffd96411583': rx, 'b62f7d3f-0003-42dd-8371-6ffd96411583': tx, 'b62f7d3f-0004-42dd-8371-6ffd96411583': info };
        const dev = { name: 'SSHub-7QK2', addEventListener() {}, gatt: { connected: false,
          connect: async () => { dev.gatt.connected = true; return { getPrimaryService: async () => ({ getCharacteristic: async (u) => chars[u] }) }; },
          disconnect: () => { dev.gatt.connected = false; } } };
        Object.defineProperty(navigator, 'bluetooth', { value: { requestDevice: async () => dev }, configurable: true });
      })()`);
    }
    const page = await context.newPage();
    await page.goto(`http://localhost:${server.address().port}/`);
    await page.waitForSelector('#create');
    await fn(page);
  } finally {
    await browser.close();
    server.close();
  }
}

// ---------------------------------------------------------------- RP10: a Red notice claiming to be authored by this very phone
test('RP10 FAILS a Red notice that claims this phone as its author needs the same typed id and shows that the signature is unchecked', { timeout: 120000 }, async () => {
  await withPage(async (page) => {
    await page.fill('#pin1', PIN);
    await page.fill('#pin2', PIN);
    await page.click('#create');
    await page.waitForSelector('#backup-words span');
    await page.check('#backup-ok');
    await page.click('#backup-done');
    await page.waitForSelector('#device-id');
    const own = (await page.textContent('#device-id')).trim();
    const now = Math.floor(Date.now() / 1000);
    const canonical = utf8(canonicalJSON({ device_id: own, seq: 1, lamport: 5, prev_hash: '00'.repeat(32), timestamp: now, epoch: 0, alg: [1, 1], body: { DeviceRoleChange: { device_id: 'dddddddddddddddd', role: 'Holder' } } }));
    const forgedSig = new Uint8Array(64).fill(7); // nobody signed this
    const len = canonical.length;
    const frame = encodeFrame(KIND.RECOVERY_NOTICE, Uint8Array.of(len >>> 24, (len >>> 16) & 255, (len >>> 8) & 255, len & 255), canonical, forgedSig);
    await page.click('#scan-release');
    await page.evaluate(() => (document.querySelector('details').open = true));
    await page.fill('#paste', b64enc(frame));
    await page.click('#paste-go');
    // Fixed by verifying the claimed own signature: a forged notice ends on a refusal (stricter than the typed id the finding
    // asked for); a notice that did show the screen would still need the typed id.
    await page.waitForSelector('#red-banner, #refusal');
    if (await page.locator('#refusal').count()) {
      assert.match(await page.textContent('#refusal'), /does not verify/);
      assert.equal(await page.locator('#red-approve').count(), 0);
      return;
    }
    const approveEnabled = await page.locator('#red-approve').isEnabled();
    const gate = await page.locator('#confirm-proposer').count();
    const sigLine = await page.locator('.mut', { hasText: 'signature' }).first().textContent();
    assert.ok(!approveEnabled || gate === 1, `Approve is enabled at once, no typed id; the page says: "${sigLine}" for an op whose signature it never verified (own id ${own})`);
  });
});

// ---------------------------------------------------------------- RP11: a frame pushed over Bluetooth between two receiving screens is dropped
test('RP11 FAILS a list the terminal pushes while no screen wants it waits for the next receiving screen (bleLast)', { timeout: 180000 }, async () => {
  const secret = new Uint8Array(32).fill(21);
  const id = identityOfSecret(secret.slice());
  const live = emptyState();
  appendHeadAttest(live, id, 1_900_000_000, null);
  appendHeadAttest(live, id, 1_900_000_010, null);
  const list = encodePhoneOps(courierOps(live, 8), new Uint8Array(8).fill(1));
  const list2 = encodePhoneOps(courierOps(live, 8), new Uint8Array(8).fill(2)); // the second list comes from another terminal
  const msgs = cutFrame(list, 1, 244);
  const msgs2 = cutFrame(list2, 2, 244);
  await withPage(async (page) => {
    await page.fill('#pin1', PIN);
    await page.fill('#pin2', PIN);
    await page.fill('#words-input', secretToWords(secret.slice()));
    await page.click('#restore');
    await page.waitForSelector('#head-scan');
    await page.click('#head-scan');
    await page.click('#ble-go');
    await page.waitForFunction(() => /Waiting for the terminal/.test(document.querySelector('#ble-status').textContent), null, { timeout: 15000 });
    for (const m of msgs) await page.evaluate((a) => window.__emit(a), Array.from(m));
    await page.waitForSelector('#head-next', { timeout: 15000 }); // list one read: "one more needed"
    for (const m of msgs2) await page.evaluate((a) => window.__emit(a), Array.from(m)); // the terminal pushes the second list now
    await page.click('#head-next');
    await page.click('#ble-go');
    let agreed = true;
    try {
      await page.waitForSelector('#head-agreed', { timeout: 30000 });
    } catch {
      agreed = false;
    }
    assert.ok(agreed, `the second list was pushed over an open session and lost; screen: ${(await page.evaluate(() => document.body.innerText)).replace(/\n+/g, ' | ').slice(0, 400)}`);
  }, { fakeBle: true });
});
