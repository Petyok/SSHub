// The e2e phone: the committed sshub-companion.html in headless Chromium, driven only through what a person
// has in front of them: buttons, the paste box of a scan screen, the text the page shows and copies (ops text,
// clipboard) and the QR code it draws (read back from the screen like a camera would). No hook of any kind is
// added to the page: no init script, no clock shift, no fake camera, no evaluation of page internals.
//
//   node phone-driver.mjs serve            long-lived daemon (compose service `phone`), one Chromium, one page
//   node phone-driver.mjs <command> [json] one command against the daemon; prints one JSON line, exit 1 on error
//
// Commands (all arguments in one JSON object):
//   info                                   page fingerprint check, current heading, device id when on home
//   setup {words?, pin?}                   restore the fixed test identity (first run) or unlock it, land on home
//   home                                   back to the home screen from wherever a flow ended
//   red-open {frame}                       scan (paste) a kind 3 notice; returns banner, description rows, op hash
//   red-approve                            tap Approve; returns the ops text (kind 5, base64) the page shows
//   share-open {frame}                     scan (paste) a kind 1 challenge; returns headline, words, rows, warnings
//   share-approve {confirm?, snapshot?}    approve (types the requester id when the page asks); returns the reply text
//   enroll-start {name?}                   scan screen of the enrollment, waiting for the terminal commitment
//   enroll-commit {frame}                  paste the terminal commitment; returns the phone's commitment (kind 6)
//   enroll-reveal {frame}                  paste the terminal reveal; returns SAS words and the phone's reveal (kind 7)
//   enroll-match {ok?}                     tap "words match" (or "differ"); returns the phone's confirmation (kind 8)
//   enroll-finish {frame}                  paste the terminal confirmation; returns the final screen text
//   ops-text                               the ops list the current screen shows as text (base64 of a kind 5 frame)
//   verify-ops {ops, kind?, opHash?}       check a kind 5 list: every op by this phone, signature valid, chain from
//                                          seq 1 continuous, last op of `kind` (RedPhoneSign naming `opHash`)
//   screen                                 text of the whole page (diagnostics)
import http from 'node:http';
import net from 'node:net';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const COMPANION = process.env.PHONE_LIB || '/app/assets/companion';
const PAGE_DIR = process.env.PHONE_PAGE_DIR || path.join(here, 'phone-page');
const PROFILE = process.env.PHONE_PROFILE || '/phone/profile';
const PORT = Number(process.env.PHONE_PORT || 7777);
const PIN = process.env.PHONE_PIN || 'e2e-phone-pin-0001';
const SECRET_HEX = '42'.repeat(32); // the fixture phone of the Rust side (its device id is derived below)

const require = createRequire(import.meta.url);
const lib = (f) => import(pathToFileURL(path.join(COMPANION, 'src', f)).href);
// A tap. Chrome on Android moves the page under the point Playwright computed when a long screen first scrolls (the toolbar
// hides), and another element then "intercepts" the click: a real click first, the element's own click event when the hit test is wrong.
// `page` is assigned below, before any tap runs.
async function tap(sel, opts = {}) {
  try {
    await page.click(sel, { timeout: 5000, ...opts });
  } catch (e) {
    if (!/intercepts pointer events|Timeout/.test(String(e.message))) throw e;
    await page.locator(sel).dispatchEvent('click');
  }
}

const { chromium, _android: android } = require('playwright');
const jsQR = require(path.join(COMPANION, 'vendor/jsqr-1.4.0/jsQR.cjs'));

// ------------------------------------------------------------------ the page file under test
const pageHtml = fs.readFileSync(path.join(PAGE_DIR, 'sshub-companion.html'));
const fingerprint = fs.readFileSync(path.join(PAGE_DIR, 'FINGERPRINT'), 'utf8').trim();
const recomputed = crypto.createHash('sha256').update(pageHtml.toString('utf8').replace(fingerprint, '0'.repeat(64))).digest('hex');
const fingerprintOk = pageHtml.toString('utf8').split(fingerprint).length === 2 && recomputed === fingerprint;

let page = null;
let target = null;
const S = {}; // loaded library modules and the identity

// ------------------------------------------------------------------ targets: where the page runs
// A target is the one place that knows about the browser: open(pageHtml) serves the page and returns
// { page, close }, where page is a Playwright Page (everything below only uses the Page API, so every target runs the
// same steps). PHONE_TARGET picks one; 'desktop' is headless Chromium in this container. A second target (Chrome on
// an Android emulator) is `android` below; nothing else in this file knows which one runs.
const targets = {
  async desktop(html) {
    const server = http.createServer((req, res) => {
      if (req.url.split('?')[0] !== '/sshub-companion.html' && req.url !== '/') return void res.writeHead(404).end();
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' }).end(html);
    });
    await new Promise((r) => server.listen(0, 'localhost', r));
    const origin = `http://localhost:${server.address().port}`;
    fs.mkdirSync(PROFILE, { recursive: true });
    const ctx = await chromium.launchPersistentContext(PROFILE, {
      headless: true,
      args: ['--no-sandbox', '--disable-dev-shm-usage'],
      viewport: { width: 420, height: 900 },
    });
    await ctx.grantPermissions(['clipboard-read', 'clipboard-write'], { origin });
    const p = await ctx.newPage();
    p.on('pageerror', (e) => console.error('[pageerror]', e.message));
    await p.goto(`${origin}/sshub-companion.html`);
    return { page: p, close: async () => { await ctx.close(); server.close(); } };
  },

  // Google Chrome on the Android emulator of the compose service `android` (adb server android:5037). Chrome is a user
  // build, so Playwright cannot launch it with flags: the first run is tapped through with uiautomator, Chrome's
  // always-present devtools socket is relayed to 127.0.0.1:9222 and Playwright attaches over CDP (same approach as the
  // Android app's chrome probe). The page is not served over the network: its committed bytes are fulfilled for one
  // https origin by a route, which makes it a secure context (camera, clipboard, crypto) without any adb forwarding.
  async android(html) {
    const host = process.env.ADB_HOST || 'android';
    const port = Number(process.env.ADB_PORT || 5037);
    const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
    let devices = [];
    for (let i = 0; i < 60 && devices.length !== 1; i++) {
      devices = await android.devices({ host, port });
      if (devices.length !== 1) await sleep(2000); // the adb server lists the emulator a moment after its boot
    }
    if (devices.length !== 1) throw new Error(`expected 1 device on ${host}:${port}, got ${devices.length}`);
    const device = devices[0];
    const sh = async (c) => (await device.shell(c)).toString();
    const shTry = (c) => sh(c).catch(() => ''); // uiautomator and friends fail while the UI is still starting
    const FRE = [/^use without an account$/i, /^no,? thanks$/i, /^don.t turn on$/i, /^accept (&|and) continue$/i, /^skip$/i, /^got it$/i, /^continue$/i, /^next$/i, /^not now$/i, /^don.t allow$/i];
    const tapOnce = async () => {
      const xml = await shTry('uiautomator dump /dev/tty');
      const nodes = [...xml.matchAll(/<node [^>]*?text="([^"]*)"[^>]*?clickable="true"[^>]*?bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"/g)];
      const hit = FRE.map((re) => nodes.find((n) => re.test(n[1]))).find(Boolean);
      if (!hit) return false;
      await sh(`input tap ${(+hit[2] + +hit[4]) / 2} ${(+hit[3] + +hit[5]) / 2}`);
      return true;
    };
    for (let i = 0; i < 30 && !(await shTry('pm path com.android.chrome')).includes('package:'); i++) await sleep(2000);
    await sh('am start -n com.android.chrome/com.google.android.apps.chrome.Main');
    let up = false;
    for (let i = 0; i < 120 && !up; i++) { // a cold emulator under load needs minutes for Chrome's first run
      await sleep(2500);
      const tapped = await tapOnce();
      const socket = Number((await shTry('cat /proc/net/unix | grep -c chrome_devtools_remote')).trim()) > 0;
      up = !tapped && socket && i >= 2;
    }
    if (!up) {
      const top = (await shTry('dumpsys activity activities | grep -m1 topResumedActivity')).trim();
      const texts = [...(await shTry('uiautomator dump /dev/tty')).matchAll(/text="([^"]+)"/g)].map((m) => m[1]).slice(0, 15).join(' | ');
      throw new Error(`Chrome did not reach its tab switcher; top activity: ${top}; screen texts: ${texts}`);
    }
    const relay = net.createServer(async (conn) => {
      const dev = await device.open('localabstract:chrome_devtools_remote');
      conn.on('data', (b) => dev.write(b));
      dev.on('data', (b) => conn.write(b));
      conn.on('close', () => dev.close().catch(() => {}));
      dev.on('close', () => conn.destroy());
      conn.on('error', () => {});
    });
    await new Promise((r) => relay.listen(9222, '127.0.0.1', r));
    const browser = await chromium.connectOverCDP('http://127.0.0.1:9222');
    const ctx = browser.contexts()[0] ?? (await browser.newContext());
    const origin = 'https://phone.e2e.test';
    await ctx.route(`${origin}/**`, (route) => route.fulfill({ status: 200, contentType: 'text/html; charset=utf-8', body: html.toString('utf8') }));
    const p = await ctx.newPage();
    p.on('pageerror', (e) => console.error('[pageerror]', e.message));
    await p.goto(`${origin}/sshub-companion.html`);
    return { page: p, close: async () => { await p.close().catch(() => {}); await browser.close().catch(() => {}); relay.close(); await device.close().catch(() => {}); } };
  },
};

async function boot() {
  if (!fingerprintOk) throw new Error(`the pinned page does not match its FINGERPRINT (${recomputed} vs ${fingerprint})`);
  const keys = await lib('keys.mjs');
  const frames = await lib('frames.mjs');
  const ops = await lib('ops.mjs');
  const scan = await lib('scan.mjs');
  const l = await lib('lib.mjs');
  Object.assign(S, { keys, frames, ops, scan, l });
  const secret = l.bytesOf(SECRET_HEX);
  S.identity = keys.identityOfSecret(secret);
  S.words = keys.secretToWords(secret);
  const kind = process.env.PHONE_TARGET || 'desktop';
  if (!targets[kind]) throw new Error(`unknown PHONE_TARGET ${kind} (have: ${Object.keys(targets).join(', ')})`);
  target = await targets[kind](pageHtml);
  page = target.page;
}

// ------------------------------------------------------------------ small helpers on the page
const has = async (sel) => (await page.locator(sel).count()) > 0;
const text = async (sel) => (await page.locator(sel).first().innerText()).trim();
const heading = async () => ((await has('h1')) ? text('h1') : '');
const waitAny = (sels, ms = 60000) => page.waitForSelector(sels.join(', '), { timeout: ms });
const b64 = (u8) => Buffer.from(u8).toString('base64');
const unb64 = (s) => Uint8Array.from(Buffer.from(s.replace(/\s+/g, ''), 'base64'));

async function home() {
  await page.waitForSelector('#words-input, #unlock, #device-id, h1', { timeout: 60000 });
  for (let i = 0; i < 4; i++) {
    if (await has('#device-id')) return;
    if (await has('#unlock')) {
      await page.waitForTimeout(500); // the unlock screen settles after a reload
      await page.fill('#pin', PIN);
      await tap('#unlock', { force: true });
      await waitAny(['#device-id', '#unlock-error']);
      continue;
    }
    let tapped = false;
    for (const id of ['#send-done', '#enroll-done', '#head-done', '#refuse', '#back']) {
      if (await has(id)) {
        await tap(id);
        tapped = true;
        break;
      }
    }
    if (!tapped) await page.reload();
    await page.waitForTimeout(150);
  }
  if (!(await has('#device-id'))) throw new Error(`could not reach the home screen, page shows: ${(await heading()) || '(nothing)'}`);
}

async function setup({ words = S.words, pin = PIN } = {}) {
  // The page draws its first screen asynchronously (it reads its storage first). A flow that ended on a screen
  // without a way back (a refusal) is left by reloading the page, as a person would reopen it.
  await page.reload(); // whatever the last flow left on screen, a person reopening the page starts from its lock screen
  await page.waitForSelector('#words-input, #unlock, #device-id', { timeout: 60000 });
  if (await has('#words-input')) {
    await page.fill('#words-input', words);
    await page.fill('#pin1', pin);
    await page.fill('#pin2', pin);
    await tap('#restore');
    // The page shows "Deriving the key..." in the same place while Argon2 runs; only a different text is a refusal.
    await page.waitForFunction(() => {
      if (document.querySelector('#device-id')) return true;
      const e = document.querySelector('#setup-error');
      return !!e && e.textContent !== '' && !/^Deriving/.test(e.textContent);
    }, null, { timeout: 60000 });
    if (!(await has('#device-id'))) throw new Error(`restore refused: ${await text('#setup-error')}`);
    // A restored key does not author until it knows where its chain ended; this key has never signed.
    if (await has('#head-new')) {
      await page.fill('#head-confirm', S.identity.deviceId); // the page asks for the device id before it trusts a fresh chain
      await tap('#head-new');
    }
  }
  await home();
  const id = await text('#device-id');
  if (id !== S.identity.deviceId) throw new Error(`the page shows device ${id}, expected ${S.identity.deviceId}`);
  return { device_id: id, restored_from: 'fixed test words' };
}

// Pastes into the scan screen the way a person without a camera does: summary, text box, button.
async function paste(frameB64) {
  if (!(await has('#paste'))) throw new Error(`not on a scan screen (page shows: ${await heading()})`);
  if (!(await page.locator('#paste').isVisible())) await tap('summary');
  await page.fill('#paste', frameB64);
  await tap('#paste-go');
}

// Reads a looping QR canvas back like a camera: frames decoded with jsQR, assembled by the page's own collector.
async function readQr(sel, ms = 30000) {
  const col = new S.scan.DocumentCollector();
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    const shot = await page.evaluate((s) => {
      const c = document.querySelector(s);
      if (!c) return null;
      const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
      let b = '';
      const u8 = new Uint8Array(d.buffer, d.byteOffset, d.byteLength);
      for (let i = 0; i < u8.length; i += 8192) b += String.fromCharCode.apply(null, u8.subarray(i, i + 8192));
      return { w: c.width, h: c.height, b64: btoa(b) };
    }, sel);
    if (shot) {
      const r = jsQR(new Uint8ClampedArray(Buffer.from(shot.b64, 'base64')), shot.w, shot.h, { inversionAttempts: 'dontInvert' });
      if (r) {
        const doc = col.push(Uint8Array.from(r.binaryData));
        if (doc) return doc;
      }
    }
    await page.waitForTimeout(40);
  }
  throw new Error(`no complete document on ${sel} within ${ms} ms`);
}

// A kind 5 frame: "SWR1" || version u16 || kind 5 || n u8 || n * (len u32 BE || op || sig(64)). Decoded here, not with
// the page's own module, so the check does not depend on which version of the libraries the image carries.
function decodeOps(f) {
  if (f.length < 8 || Buffer.from(f.slice(0, 4)).toString() !== 'SWR1' || f[6] !== 5) throw new Error('not a kind 5 frame');
  const n = f[7];
  let at = 8;
  const out = [];
  for (let i = 0; i < n; i++) {
    const len = ((f[at] << 24) | (f[at + 1] << 16) | (f[at + 2] << 8) | f[at + 3]) >>> 0;
    at += 4;
    if (f.length < at + len + 64) throw new Error('op list truncated');
    out.push({ canonical: f.slice(at, at + len), signature: f.slice(at + len, at + len + 64) });
    at += len + 64;
  }
  if (at !== f.length) throw new Error('ops frame has trailing bytes');
  return out;
}

// An enrollment frame as the page shows it as text (#enroll-b64), cross-checked against the QR it draws.
async function enrollFrame() {
  await page.waitForSelector("#enroll-qr");
  const t = (await page.locator("#enroll-b64").inputValue()).trim();
  const qr = b64(await readQr("#enroll-qr"));
  if (t !== qr) throw new Error("the enrollment text differs from the QR the page draws");
  return t;
}

const rowsOf = () => page.evaluate(() => Array.from(document.querySelectorAll('#desc .k')).map((k) => [k.textContent, k.nextElementSibling && k.nextElementSibling.textContent, k.nextElementSibling && k.nextElementSibling.dataset.danger === '1']));

// ------------------------------------------------------------------ commands
const commands = {
  async info() {
    return { fingerprint, fingerprint_ok: fingerprintOk, heading: await heading(), device_id: (await has('#device-id')) ? await text('#device-id') : null, expected_device_id: S.identity.deviceId };
  },
  setup,
  // Forget everything this origin stored (the key, its chain, the ring) as a person would by clearing the site's data,
  // so each scenario's fresh ring meets a phone whose chain starts at seq 1 (the page lists only its newest 8 ops).
  async reset() {
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Storage.clearDataForOrigin', { origin: new URL(page.url()).origin, storageTypes: 'all' });
    await cdp.detach().catch(() => {});
    await page.reload();
    return { reset: true };
  },
  async home() {
    await home();
    return { device_id: await text('#device-id') };
  },
  // The 24 backup words of the fixed test identity (what the native app restores from; the same key as the page's).
  async words() {
    return { words: S.words };
  },
  async screen() {
    return { heading: await heading(), text: await page.evaluate(() => document.body.innerText) };
  },

  // ---- Red notice (kind 3)
  async 'red-open'({ frame }) {
    await home();
    await tap('#scan-release');
    await paste(frame);
    await waitAny(['#red-banner', '#refusal']);
    if (await has('#refusal')) return { ok: false, refusal: await text('#refusal') };
    return {
      ok: true,
      banner: await text('#red-banner'),
      headline: await text('#headline'),
      rows: await rowsOf(),
      op_hash: (await text('#red-hash')).replace(/^Operation hash\s+/, ''),
      blocked: (await has('#red-blocked')) ? await text('#red-blocked') : null,
      can_approve: await has('#red-approve'),
    };
  },
  async 'red-approve'() {
    await tap('#red-approve');
    await waitAny(['#ops-b64', '#red-error']);
    if (await has('#red-error') && (await text('#red-error'))) return { ok: false, error: await text('#red-error') };
    const ops = await page.locator('#ops-b64').inputValue();
    const qr = b64(await readQr('#reverse-qr'));
    return { ok: true, ops, qr_equals_text: qr === ops };
  },

  // ---- share release (kind 1)
  async 'share-open'({ frame }) {
    await home();
    await tap('#scan-release');
    await paste(frame);
    await waitAny(['#headline', '#refusal']);
    if (await has('#refusal')) return { ok: false, refusal: await text('#refusal'), checks: await page.locator('#checks .check').allInnerTexts() };
    return {
      ok: true,
      headline: await text('#headline'),
      binding_words: await text('#binding-words'),
      rows: await rowsOf(),
      checks: await page.locator('#checks .check').allInnerTexts(),
      unknown_requester: (await has('#unknown-requester')) ? await text('#unknown-requester') : null,
      approve_enabled: await page.locator('#approve').isEnabled(),
    };
  },
  async 'share-approve'({ confirm = null, snapshot = false } = {}) {
    if (await has('#confirm-requester')) {
      if (!confirm) throw new Error('the page asks for the requester device id and none was given');
      await page.fill('#confirm-requester', confirm);
    }
    // Options as a person would set them: QR and clipboard stay on, sound off, short code only when asked for.
    if ((await has('#opt-sound')) && (await page.locator('#opt-sound').isChecked())) await tap('#opt-sound');
    if ((await has('#opt-snapshot')) && (await page.locator('#opt-snapshot').isChecked()) !== snapshot) await tap('#opt-snapshot');
    await tap('#approve');
    // #release-error shows "Approving..." while the page works; only another text is a refusal.
    await page.waitForFunction(() => {
      if (document.querySelector('#send-done')) return true;
      const e = document.querySelector('#release-error');
      return !!e && e.textContent !== '' && !/^Approving/.test(e.textContent);
    }, null, { timeout: 120000 });
    if ((await has('#release-error')) && (await text('#release-error')) && !(await has('#send-done'))) return { ok: false, error: await text('#release-error') };
    const clip = await page.evaluate(() => navigator.clipboard.readText()).catch(() => null);
    const qr = b64(await readQr('#reverse-qr'));
    return { ok: true, reply: clip ?? qr, clipboard_readable: clip !== null, qr_equals_clipboard: clip === null ? null : clip === qr, code: (await has('#snapshot-code')) ? await text('#snapshot-code') : null };
  },

  // ---- enrollment (phone is the initiator, role 0): kinds 6, 7, 8
  async 'enroll-start'({ name = 'my phone' } = {}) {
    await home();
    await tap('#enroll');
    await page.fill('#device-name', name);
    await tap('#enroll-scan');
    return { heading: await heading() };
  },
  async 'enroll-commit'({ frame }) {
    await paste(frame);
    await waitAny(['#enroll-qr', '#refusal']);
    if (await has('#refusal')) return { ok: false, refusal: await text('#refusal') };
    return { ok: true, frame: await enrollFrame() };
  },
  async 'enroll-reveal'({ frame }) {
    await tap('#enroll-next');
    await paste(frame);
    await waitAny(['#sas-words', '#refusal']);
    if (await has('#refusal')) return { ok: false, refusal: await text('#refusal') };
    return { ok: true, words: await text('#sas-words'), frame: await enrollFrame() };
  },
  async 'enroll-match'({ ok = true } = {}) {
    await tap(ok ? '#sas-match' : '#sas-differ');
    await page.waitForSelector('#enroll-qr');
    return { frame: await enrollFrame() };
  },
  async 'enroll-finish'({ frame }) {
    await tap('#enroll-next');
    await paste(frame);
    await waitAny(['#enroll-done', '#refusal']);
    if (await has('#refusal')) return { ok: false, refusal: await text('#refusal') };
    return { ok: true, text: await text('.card, h1') };
  },

  // ---- text the page shows
  async 'ops-text'() {
    return { ops: await page.locator('#ops-b64').inputValue() };
  },

  // ---- check what the page gave back (the terminal side would run the same checks in Rust)
  async 'verify-ops'({ ops, kind = null, opHash = null }) {
    // Plain JSON.parse is enough here (the numbers are small) and keeps the check independent of the page libraries.
    const { blake3 } = await import(pathToFileURL(path.join(COMPANION, 'vendor/noble-hashes-2.4.0/blake3.js')).href);
    const list = decodeOps(unb64(ops));
    if (!list.length) throw new Error('the list is empty');
    const hex = (u) => Buffer.from(u).toString('hex');
    let prev = null;
    const out = [];
    let last = null;
    for (const o of list) {
      const op = JSON.parse(Buffer.from(o.canonical).toString('utf8'));
      if (op.device_id !== S.identity.deviceId) throw new Error(`op by ${op.device_id}, not by this phone`);
      if (!S.l.edVerify(S.identity.edPub, o.canonical, o.signature)) throw new Error(`the signature of seq ${op.seq} does not verify under the phone key`);
      const hash = hex(blake3(o.canonical));
      if (prev) {
        if (op.seq !== prev.seq + 1 || op.prev_hash !== prev.hash) throw new Error('the ops are not one chain');
      } else if (op.seq === 1 && op.prev_hash !== '0'.repeat(64)) throw new Error('seq 1 does not start from zero');
      prev = { seq: op.seq, hash };
      last = op;
      out.push({ seq: op.seq, kind: Object.keys(op.body)[0] });
    }
    const lastKind = Object.keys(last.body)[0];
    if (kind && lastKind !== kind) throw new Error(`the last op is ${lastKind}, expected ${kind}`);
    const named = last.body[lastKind].op_hash || null;
    if (opHash && named !== opHash) throw new Error(`the proof names ${named}, expected ${opHash}`);
    return { ok: true, ops: out, device_id: S.identity.deviceId, op_hash: named };
  },

  // ---- the terminal frame builders the CLI does not offer yet (used by phone-selftest.sh only, which has no CLI peer; scenarios use the CLI verb sync-notice)
  async 'frame-notice'({ canonicalHex, signatureHex }) {
    const canonical = Uint8Array.from(Buffer.from(canonicalHex, 'hex'));
    const signature = Uint8Array.from(Buffer.from(signatureHex, 'hex'));
    const len = canonical.length;
    const frame = S.frames.encodeFrame(S.frames.KIND.RECOVERY_NOTICE, Uint8Array.of(len >>> 24, (len >>> 16) & 255, (len >>> 8) & 255, len & 255), canonical, signature);
    return { frame: b64(frame) };
  },
};

// ------------------------------------------------------------------ daemon and client
async function serve() {
  await boot();
  let busy = Promise.resolve();
  const srv = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      // one command at a time: the page is one phone with one screen
      busy = busy.then(async () => {
        let code = 200;
        let out;
        try {
          const { cmd, args } = JSON.parse(body || '{}');
          if (!commands[cmd]) throw new Error(`unknown command ${cmd}`);
          out = await commands[cmd](args || {});
        } catch (e) {
          code = 500;
          out = { error: e.message };
        }
        res.writeHead(code, { 'content-type': 'application/json' }).end(JSON.stringify(out));
      });
    });
  });
  srv.listen(PORT, '127.0.0.1', () => console.log(`phone driver ready on ${PORT}; page fingerprint ${fingerprint}`));
  const stop = async () => {
    try { await target.close(); } catch { /* closing */ }
    process.exit(0);
  };
  process.on('SIGTERM', stop);
  process.on('SIGINT', stop);
}

async function client(cmd, argText) {
  const args = argText ? JSON.parse(argText) : {};
  const r = await fetch(`http://127.0.0.1:${PORT}/`, { method: 'POST', body: JSON.stringify({ cmd, args }) });
  const out = await r.json();
  console.log(JSON.stringify(out));
  process.exit(r.ok ? 0 : 1);
}

const [cmd, arg] = process.argv.slice(2);
if (cmd === 'serve') await serve();
else if (cmd) await client(cmd, arg);
else {
  console.error('usage: phone-driver.mjs serve | <command> [json]');
  process.exit(2);
}
