// End to end run of the shipped page in headless Chromium (tools container).
//
//  * The "terminal" side is the Rust fixture set (test/fixtures/*.json): challenges built with the
//    real encoders, rendered as QR frames of 84 bytes (an 80x24 terminal) into y4m files that the
//    browser reads as its fake camera.
//  * Enrollment is interactive (the terminal confirmation depends on the phone's random reveal),
//    so there the camera is a canvas stream the script paints; it plays the terminal with the
//    keys of the fixture requester.
//  * What the page sends back is captured from its own canvas (reverse QR), clipboard and audio
//    samples and written to $E2E_OUT for the Rust verifier (cargo test ...e2e_response_opens...).
//
// Usage (inside the tools container):  NODE_PATH=/opt/node/lib/node_modules node scripts/e2e.mjs
// Writes $E2E_OUT/results.json and prints one JSON line per check. Exit 1 when a check fails.
import { createRequire } from 'node:module';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';
import { bytesOf, edSign, edVerify, hexOf, identityFromIkm, sasWords } from '../src/lib.mjs';
import { DocumentCollector } from '../src/scan.mjs';
import { fountainEncode } from '../src/quorum.mjs';
import {
  commitOf, confirmMessage, decodeEnroll, decodeFrame, decodePhoneOps, encodeCertExtendRequest, encodeCertRequest, encodeEnrollConfirm, encodeEnrollReveal, encodeFrame, encodePhoneOps, parseReplyPlain, transcriptHash, KIND,
} from '../src/frames.mjs';
import { crc32 } from '../src/ble.mjs';
import { b64dec, parseOp } from '../src/ops.mjs';
import { envelopeAad, ownStorageKey } from '../src/reshare.mjs';
import { EPOCH as RESHARE_EPOCH, hostBytes, scenario } from '../test/reshare-scenario.mjs';
import { blake3 } from '../vendor/noble-hashes-2.4.0/blake3.js';
import { xchacha20poly1305 } from '../vendor/noble-ciphers-2.4.0/chacha.js';
import { reshareCombine, sealedBoxOpen, shamirCombine } from '../src/lib.mjs';
import { identityOfSecret, secretFromWords } from '../src/keys.mjs';
import { appendHeadAttest, courierOps, emptyState } from '../src/state.mjs';

const require = createRequire(import.meta.url);
const { chromium } = require('playwright');
const qrcode = require('../vendor/qrcode-generator-1.4.4/qrcode.cjs');
const jsQR = require('../vendor/jsqr-1.4.0/jsQR.cjs');

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..');
const OUT = process.env.E2E_OUT || '/tmp/e2e-out';
const PAGE = path.resolve(root, process.env.E2E_PAGE || 'sshub-companion.html');
fs.mkdirSync(OUT, { recursive: true });
const load = (n) => JSON.parse(fs.readFileSync(path.join(root, 'test/fixtures', n), 'utf8'));
const fx = load('challenges.json');
const enrollFx = load('enroll.json');
const h = (x) => bytesOf(x);
const PIN = 'correct-pin-17';
const FPS = 8;

const results = [];
const check = (claim, pass, measured, unit = '') => {
  const row = { claim, pass: !!pass, measured, unit };
  results.push(row);
  console.log(JSON.stringify(row));
  return pass;
};

// ------------------------------------------------------------ the page and its browser

const body = fs.readFileSync(PAGE);
const server = http.createServer((req, res) => {
  if (req.url !== '/') return void res.writeHead(404).end();
  res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' }).end(body);
});
await new Promise((r) => server.listen(0, 'localhost', r));
const ORIGIN = `http://localhost:${server.address().port}`;
const seenRequests = [];
const cspProblems = [];
const pageErrors = [];
let profileCounter = 0;

// QR module matrix of one frame (version 5 for 84 byte frames, level M).
function matrixOf(frame) {
  const qr = qrcode(5, 'M');
  qr.addData(String.fromCharCode(...frame), 'Byte');
  qr.make();
  const n = qr.getModuleCount();
  return Array.from({ length: n }, (_, r) => Array.from({ length: n }, (_, c) => (qr.isDark(r, c) ? '1' : '0')).join(''));
}

function writeY4m(file, frames) {
  const mats = frames.map(matrixOf);
  const n = mats[0].length;
  const scale = 6;
  const quiet = 4;
  const W = (n + 2 * quiet) * scale;
  const Y = Buffer.alloc(W * W);
  const C = Buffer.alloc((W / 2) * (W / 2), 128);
  const parts = [Buffer.from(`YUV4MPEG2 W${W} H${W} F${FPS}:1 Ip A1:1 C420jpeg\n`)];
  for (const m of mats) {
    for (let y = 0; y < W; y++) {
      const r = Math.floor(y / scale) - quiet;
      for (let x = 0; x < W; x++) {
        const c = Math.floor(x / scale) - quiet;
        Y[y * W + x] = r >= 0 && c >= 0 && r < n && c < n && m[r][c] === '1' ? 0 : 255;
      }
    }
    parts.push(Buffer.from('FRAME\n'), Buffer.from(Y), C, C);
  }
  fs.writeFileSync(file, Buffer.concat(parts));
}

async function launch({ y4m = null, feed = false, profile = 'p0' } = {}) {
  const dir = path.join(OUT, 'profiles', profile);
  fs.mkdirSync(dir, { recursive: true });
  const args = ['--no-sandbox', '--disable-dev-shm-usage', '--use-fake-ui-for-media-stream', '--autoplay-policy=no-user-gesture-required'];
  if (y4m) args.push('--use-fake-device-for-media-stream', `--use-file-for-fake-video-capture=${y4m}`);
  const context = await chromium.launchPersistentContext(dir, { headless: true, args, viewport: { width: 420, height: 900 } });
  await context.grantPermissions(['camera', 'microphone', 'clipboard-read', 'clipboard-write'], { origin: ORIGIN });
  await context.addInitScript(() => {
    window.__audio = [];
    const copy = AudioBuffer.prototype.copyToChannel;
    AudioBuffer.prototype.copyToChannel = function (src, ch, off) {
      const u8 = new Uint8Array(src.buffer, src.byteOffset, src.byteLength);
      let s = '';
      for (let i = 0; i < u8.length; i += 8192) s += String.fromCharCode.apply(null, u8.subarray(i, i + 8192));
      window.__audio.push(btoa(s));
      return copy.call(this, src, ch, off);
    };
  });
  if (feed) {
    await context.addInitScript(() => {
      const canvas = document.createElement('canvas');
      canvas.width = canvas.height = 270;
      const ctx = canvas.getContext('2d');
      ctx.fillStyle = '#fff';
      ctx.fillRect(0, 0, 270, 270);
      window.__feed = {
        timer: null,
        set(mats) {
          clearInterval(this.timer);
          let i = 0;
          const draw = () => {
            const m = mats[i++ % mats.length];
            const n = m.length;
            const scale = 6;
            const q = 4;
            const size = (n + 2 * q) * scale;
            canvas.width = size;
            canvas.height = size;
            ctx.fillStyle = '#fff';
            ctx.fillRect(0, 0, size, size);
            ctx.fillStyle = '#000';
            for (let r = 0; r < n; r++) for (let c = 0; c < n; c++) if (m[r][c] === '1') ctx.fillRect((c + q) * scale, (r + q) * scale, scale, scale);
          };
          draw();
          this.timer = setInterval(draw, 125);
        },
      };
      navigator.mediaDevices.getUserMedia = async () => canvas.captureStream(8);
    });
  }
  return { context, dir };
}

let currentPage = null;
process.on('uncaughtException', async (e) => {
  console.log(`FAILURE: ${e && e.message}`);
  try {
    console.log(`SCREEN: ${(await currentPage.evaluate(() => document.body.innerText)).slice(0, 1500)}`);
    await currentPage.screenshot({ path: path.join(OUT, 'failure.png') });
  } catch (err) {
    console.log(`no screenshot: ${err.message}`);
  }
  process.exit(1);
});
async function openPage(context, { time = fx.now } = {}) {
  const page = await context.newPage();
  currentPage = page;
  page.on('request', (r) => seenRequests.push(r.url()));
  page.on('pageerror', (e) => {
    pageErrors.push(e.message);
    console.log(`[pageerror] ${e.stack || e.message}`);
  });
  page.on('console', (m) => {
    if (/Content Security Policy|Refused to/i.test(m.text())) cspProblems.push(m.text());
    if (m.type() === 'error' || m.type() === 'warning') console.log(`[page ${m.type()}] ${m.text()}`);
  });
  await page.clock.setFixedTime(new Date(time * 1000));
  await page.goto(`${ORIGIN}/`);
  return page;
}

const feedFrames = (page, hexFrames) => page.evaluate((m) => window.__feed.set(m), hexFrames.map((x) => matrixOf(h(x))));
const feedDoc = (page, bytes) => feedFrames(page, fountainEncode(bytes, 84, 25).map(hexOf));

// Reads what the page shows on a canvas, frame by frame, until one document is complete.
async function captureDoc(page, sel, timeoutMs = 30000) {
  const col = new DocumentCollector();
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    const shot = await page.evaluate((s) => {
      const c = document.querySelector(s);
      if (!c) return null;
      const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
      const u8 = new Uint8Array(d.buffer, d.byteOffset, d.byteLength);
      let b = '';
      for (let i = 0; i < u8.length; i += 8192) b += String.fromCharCode.apply(null, u8.subarray(i, i + 8192));
      return { w: c.width, h: c.height, b64: btoa(b) };
    }, sel);
    if (shot) {
      const px = new Uint8ClampedArray(Buffer.from(shot.b64, 'base64'));
      const r = jsQR(px, shot.w, shot.h, { inversionAttempts: 'dontInvert' });
      if (r) {
        const doc = col.push(Uint8Array.from(r.binaryData));
        if (doc) return { doc, frames: col.frames };
      }
    }
    await page.waitForTimeout(40);
  }
  throw new Error(`no complete document on ${sel} within ${timeoutMs} ms (${col.frames} frames read)`);
}

const text = (page, sel) => page.locator(sel).first().innerText();
const unlock = async (page) => {
  await page.fill('#pin', PIN);
  await page.click('#unlock');
  await page.waitForSelector('#device-id');
};
// A restored key says "never signed anything" only with its device id typed (round 2, PF1).
const headNew = async (page) => {
  await page.fill('#head-confirm', fx.phone.pub.device_id);
  await page.click('#head-new');
  await page.waitForSelector('#head-pending', { state: 'detached' }); // the state is saved before the home screen is drawn again
};
// Approve screens that ask for the id of a device the phone has never met (requester, proposer, delegate).
const gatedClick = async (page, sel) => {
  for (const g of ['#confirm-delegate', '#confirm-proposer']) {
    if (await page.locator(g).count()) await page.fill(g, (await page.locator(g).getAttribute('placeholder')).match(/[0-9a-f]{16}/)[0]);
  }
  await page.click(sel);
};
// A reshare needs two contributors this phone has verified, so they must be in its ring: write them into the stored state, reload, unlock.
const seedRing = async (page, ring) => {
  await page.evaluate((r) => new Promise((res, rej) => {
    const o = indexedDB.open('sshub-companion', 1);
    o.onsuccess = () => {
      const db = o.result;
      const tx = db.transaction('kv', 'readwrite');
      const st = tx.objectStore('kv');
      const g = st.get('state');
      g.onsuccess = () => st.put({ ...g.result, ring: { ...g.result.ring, ...r } }, 'state');
      tx.oncomplete = () => { db.close(); res(); };
      tx.onerror = () => rej(tx.error);
    };
  }), ring);
  await page.reload();
  await page.waitForSelector('#unlock');
  await unlock(page);
};

const phone = identityOfSecret(h(fx.phone.secret_hex));
const terminal = identityFromIkm(h(fx.requester.ikm_hex), fx.requester.context);

// ============================================================ 1. first run, PIN, backup, restore

{
  const { context } = await launch({ profile: `first-run-${profileCounter++}` });
  const page = await openPage(context);
  await page.waitForSelector('#create');
  const short = await page.evaluate(() => [document.querySelector('#create') !== null, document.title]);
  check('first run offers key creation and restore', short[0], short[1]);
  await page.fill('#pin1', 'abc');
  await page.fill('#pin2', 'abc');
  await page.click('#create');
  check('a short PIN is refused', /at least 10/.test(await text(page, '#setup-error')), await text(page, '#setup-error'));
  await page.fill('#pin1', PIN);
  await page.fill('#pin2', PIN);
  await page.click('#create');
  await page.waitForSelector('#backup-words span');
  const words = (await page.locator('#backup-words span').allInnerTexts()).join(' ');
  check('24 backup words shown once after key generation', words.split(' ').length === 24, words.split(' ').length, 'words');
  check('Continue stays disabled until the words are confirmed written', await page.locator('#backup-done').isDisabled(), true);
  await page.check('#backup-ok');
  await page.click('#backup-done');
  await page.waitForSelector('#device-id');
  const shown = await text(page, '#device-id');
  check('the device id is derived from the backup words', shown === identityOfSecret(secretFromWords(words)).deviceId, shown);
  const stored = await page.evaluate(() => new Promise((res) => {
    const r = indexedDB.open('sshub-companion', 1);
    r.onsuccess = () => { const g = r.result.transaction('kv').objectStore('kv').get('key'); g.onsuccess = () => res(Array.from(g.result)); };
  }));
  const secret = secretFromWords(words);
  const blob = Uint8Array.from(stored);
  const hay = Buffer.from(blob).toString('hex');
  check('IndexedDB holds only the PIN-wrapped secret (SSHP, no plaintext key bytes)', Buffer.from(blob.slice(0, 4)).toString() === 'SSHP' && !hay.includes(hexOf(secret)), blob.length, 'bytes');
  await page.reload();
  await page.waitForSelector('#unlock');
  await page.fill('#pin', 'wrong-pin-0');
  await page.click('#unlock');
  await page.waitForFunction(() => /wrong PIN/.test(document.querySelector('#unlock-error').textContent));
  await unlock(page);
  check('reload asks for the PIN; the right one unlocks the same device', (await text(page, '#device-id')) === shown, shown);
  await page.evaluate(() => {
    Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await page.waitForSelector('#unlock');
  check('hiding the page locks it', true, 'unlock screen shown');
  await context.close();
}

// ============================================================ 2. restore, enrollment with SAS

const PROFILE = 'enrolled';
{
  const { context } = await launch({ feed: true, profile: PROFILE });
  const page = await openPage(context);
  await page.waitForSelector('#words-input');
  await page.fill('#words-input', fx.phone.words);
  await page.fill('#pin1', PIN);
  await page.fill('#pin2', PIN);
  await page.click('#restore');
  await page.waitForSelector('#device-id');
  await headNew(page);
  check('restore from 24 words gives the fixture phone identity', (await text(page, '#device-id')) === fx.phone.pub.device_id, fx.phone.pub.device_id);

  // --- a reveal that does not open its commitment stops the enrollment
  await page.click('#enroll');
  await page.click('#enroll-scan');
  await feedFrames(page, enrollFx.existing_commit_qr_frames_hex);
  await page.waitForSelector('#enroll-qr');
  const c1Doc = (await captureDoc(page, '#enroll-qr')).doc;
  const c1 = decodeEnroll(c1Doc);
  check('the phone commitment is also offered as text equal to the QR (a terminal without a camera pastes it)', Buffer.from(c1Doc).toString('base64') === (await page.locator('#enroll-b64').inputValue()) && (await page.locator('#enroll-copy').count()) === 1, 'kind 6 text');
  check('enrollment step 2 shows the phone commitment (role 0, 32 bytes)', c1.kind === 'commit' && c1.role === 0 && c1.commit.length === 32, hexOf(c1.commit).slice(0, 16));
  await page.click('#enroll-next');
  const existing = decodeEnroll(h(enrollFx.frames_hex[2])).reveal;
  const forged = { ...existing, name: 'evil laptop' };
  const forgedFrame = Uint8Array.from([...h(enrollFx.frames_hex[2]).slice(0, 7), 1, ...(await import('../src/frames.mjs')).revealBytes(forged)]);
  await feedDoc(page, forgedFrame);
  await page.waitForSelector('#refusal');
  check('a terminal reveal that does not open its commitment is refused', /does not open the commitment/.test(await text(page, '#refusal')), await text(page, '#refusal'));
  await page.click('#back');

  // --- the honest run
  await page.click('#enroll');
  await page.click('#enroll-scan');
  await feedFrames(page, enrollFx.existing_commit_qr_frames_hex);
  await page.waitForSelector('#enroll-qr');
  const commit = decodeEnroll((await captureDoc(page, '#enroll-qr')).doc);
  await page.click('#enroll-next');
  await feedFrames(page, enrollFx.existing_reveal_qr_frames_hex);
  await page.waitForSelector('#sas-words');
  const shownSas = await text(page, '#sas-words');
  const phoneRevealDoc = (await captureDoc(page, '#enroll-qr')).doc;
  const phoneReveal = decodeEnroll(phoneRevealDoc);
  check('the phone reveal is also offered as text equal to the QR', Buffer.from(phoneRevealDoc).toString('base64') === (await page.locator('#enroll-b64').inputValue()), 'kind 7 text');
  check('the phone reveal opens the commitment it showed first', hexOf(commitOf(0, phoneReveal.reveal)) === hexOf(commit.commit), 'match');
  const th = transcriptHash(phoneReveal.reveal, existing);
  check('the four SAS words on the phone equal the words computed from both reveals', shownSas === sasWords(th), shownSas);
  check('the SAS words equal the Rust mirror when both reveals are the fixture ones', sasWords(transcriptHash(decodeEnroll(h(enrollFx.frames_hex[3])).reveal, existing)) === enrollFx.sas, enrollFx.sas);
  await page.click('#sas-match');
  await page.waitForSelector('#enroll-next');
  const confirmDoc = (await captureDoc(page, '#enroll-qr')).doc;
  const confirm = decodeEnroll(confirmDoc);
  check('the phone confirmation is also offered as text equal to the QR', Buffer.from(confirmDoc).toString('base64') === (await page.locator('#enroll-b64').inputValue()), 'kind 8 text');
  check('the phone signs the transcript with its device key (confirm, role 0, ok)', confirm.kind === 'confirm' && confirm.ok && edVerify(phone.edPub, confirmMessage(0, true, th), confirm.sig), 'signature verifies');
  await page.click('#enroll-next');
  const sig = edSign(terminal.edSeed, confirmMessage(1, true, th));
  await feedDoc(page, encodeEnrollConfirm(1, true, sig));
  await page.waitForSelector('#enroll-done');
  await page.click('#enroll-done');
  check('enrollment completes only with the terminal signature; the ring then names the terminal', (await text(page, '.card')).includes(fx.requester.pub.device_id), fx.requester.pub.device_id);

  // --- "they differ" aborts and stores nothing
  await page.click('#lock');
  await context.close();
}
{
  const { context } = await launch({ feed: true, profile: 'differ' });
  const page = await openPage(context);
  await page.fill('#words-input', fx.phone.words);
  await page.fill('#pin1', PIN);
  await page.fill('#pin2', PIN);
  await page.click('#restore');
  await page.waitForSelector('#device-id');
  await headNew(page);
  await page.click('#enroll');
  await page.click('#enroll-scan');
  await feedFrames(page, enrollFx.existing_commit_qr_frames_hex);
  await page.waitForSelector('#enroll-qr');
  await page.click('#enroll-next');
  await feedFrames(page, enrollFx.existing_reveal_qr_frames_hex);
  await page.waitForSelector('#sas-differ');
  await page.click('#sas-differ');
  const declinedDoc = (await captureDoc(page, '#enroll-qr')).doc;
  const declined = decodeEnroll(declinedDoc);
  check('the decline is also offered as text equal to the QR', Buffer.from(declinedDoc).toString('base64') === (await page.locator('#enroll-b64').inputValue()), 'kind 8 text');
  check('"They differ" sends a signed decline and enrolls nothing', declined.kind === 'confirm' && declined.ok === false, 'ok=false');
  await page.click('#enroll-next');
  await page.waitForSelector('#device-id');
  check('declined enrollment leaves the ring empty', /Not enrolled/.test(await text(page, '.card')), 'not enrolled');
  await context.close();
}

const restoreFlow = async (page) => {
  await page.waitForSelector('#words-input');
  await page.fill('#words-input', fx.phone.words);
  await page.fill('#pin1', PIN);
  await page.fill('#pin2', PIN);
  await page.click('#restore');
  await page.waitForSelector('#device-id');
  await headNew(page);
};
// ============================================================ 4d. enrolling this page with the REAL Rust terminal (PhoneEnroll)

// The terminal is the Rust test e2e_enrollment_peer_for_the_page running next to this script (verify-in-container.sh);
// frames travel as files in E2E_ENROLL_PEER/<scenario>/ (t-* from the terminal, p-* from the page).
if (process.env.E2E_ENROLL_PEER) {
  const root = process.env.E2E_ENROLL_PEER;
  const putFile = (dir, name, bytes) => {
    const tmp = path.join(dir, `.${name}.tmp`);
    fs.writeFileSync(tmp, bytes);
    fs.renameSync(tmp, path.join(dir, name));
  };
  const waitFile = async (dir, name, ms = 240000) => {
    const f = path.join(dir, name);
    const t0 = Date.now();
    while (!fs.existsSync(f)) {
      if (Date.now() - t0 > ms) throw new Error(`the Rust terminal did not write ${name} within ${ms} ms`);
      await new Promise((r) => setTimeout(r, 100));
    }
    return fs.readFileSync(f);
  };
  // Runs the page up to the words; returns what the scenario needs.
  const enrollUpToWords = async (scenario) => {
    const dir = path.join(root, scenario);
    const { context } = await launch({ feed: true, profile: `enroll-rust-${scenario}` });
    const page = await openPage(context);
    await restoreFlow(page);
    await page.click('#enroll');
    await page.click('#enroll-scan');
    await feedDoc(page, await waitFile(dir, 't-commit.bin'));
    await page.waitForSelector('#enroll-qr');
    putFile(dir, 'p-commit.bin', (await captureDoc(page, '#enroll-qr')).doc);
    await page.click('#enroll-next');
    await feedDoc(page, await waitFile(dir, 't-reveal.bin'));
    await page.waitForSelector('#sas-words');
    const words = (await text(page, '#sas-words')).trim();
    const reveal = (await captureDoc(page, '#enroll-qr')).doc;
    return { dir, context, page, words, reveal };
  };
  const rustVerdict = async (dir) => JSON.parse((await waitFile(dir, 'done.json')).toString());

  {
    const { dir, context, page, words, reveal } = await enrollUpToWords('ok');
    putFile(dir, 'p-reveal.bin', reveal);
    putFile(dir, 'p-words.txt', words);
    const tWords = (await waitFile(dir, 't-words.txt')).toString();
    check('the page and the Rust terminal show the same four words', tWords === words && words.split('-').length === 4, words);
    await page.click('#sas-match');
    await page.waitForSelector('#enroll-next');
    putFile(dir, 'p-confirm.bin', (await captureDoc(page, '#enroll-qr')).doc);
    await page.click('#enroll-next');
    await feedDoc(page, await waitFile(dir, 't-confirm.bin'));
    await page.waitForSelector('#enroll-done', { timeout: 60000 });
    const tid = (await waitFile(dir, 't-id.txt')).toString();
    await page.click('#enroll-done');
    check('the page accepts the Rust terminal confirmation and names the terminal in its ring', (await text(page, '.card')).includes(tid), tid);
    const v = await rustVerdict(dir);
    check('the Rust terminal verifies the page: Verified, the admitted DevicePublic is the page identity, DeviceAdd proposed', v.pass, v.why || 'verified and admitted');
    await context.close();
  }
  {
    const { dir, context, reveal } = await enrollUpToWords('tampered');
    const r = decodeEnroll(reveal).reveal;
    putFile(dir, 'p-reveal.bin', encodeEnrollReveal(0, { ...r, name: 'evil phone' }));
    const v = await rustVerdict(dir);
    check('a page reveal altered on the way stops the Rust terminal', v.pass, v.why || 'Stopped: does not open the commitment');
    await context.close();
  }
  {
    const { dir, context, page, words, reveal } = await enrollUpToWords('differ');
    putFile(dir, 'p-reveal.bin', reveal);
    putFile(dir, 'p-words.txt', words);
    await page.click('#sas-differ');
    await page.waitForSelector('#enroll-qr');
    putFile(dir, 'p-confirm.bin', (await captureDoc(page, '#enroll-qr')).doc);
    const v = await rustVerdict(dir);
    check('the page decline (signed, role 0, ok false) stops the Rust terminal and admits nothing', v.pass, v.why || 'Stopped: the phone says the words differ');
    await context.close();
  }
}

// ============================================================ 3. release scenarios

const challenge = (name) => fx.challenges.find((c) => c.name === name);
const y4mFor = (name) => {
  const f = path.join(OUT, `${name}.y4m`);
  writeY4m(f, challenge(name).qr_frames_hex.map(h));
  return f;
};

async function scanChallenge(name, { time = fx.now, profile = PROFILE } = {}) {
  const { context } = await launch({ y4m: y4mFor(name), profile });
  const page = await openPage(context, { time });
  await unlock(page);
  await page.click('#scan-release');
  await Promise.race([page.waitForSelector('#headline', { timeout: 60000 }), page.waitForSelector('#refusal', { timeout: 60000 })]);
  return { context, page };
}

{
  // --- the good one: description, highlighting, approve, every channel
  const t0 = Date.now();
  const { context, page } = await scanChallenge('good');
  const scanMs = Date.now() - t0;
  const good = challenge('good');
  check('scan of the Rust challenge (80x24 frames, 8 fps loop) reaches the approval screen', await page.locator('#approve').count() === 1, scanMs, 'ms incl. unlock');
  const desc = await text(page, '#desc');
  check('the description names the requesting device', desc.includes(fx.requester.name) && desc.includes(fx.requester.pub.device_id), `${fx.requester.name} [${fx.requester.pub.device_id}]`);
  const dangerRows = await page.locator('#desc .v[data-danger="1"]').allInnerTexts();
  check('requester and credential are highlighted as dangerous', dangerRows.length === 2 && dangerRows.some((x) => x.includes(fx.requester.pub.device_id)) && dangerRows.some((x) => x.includes(fx.host_uid)), dangerRows.length, 'highlighted rows');
  const safeRows = await page.locator('#desc .v[data-danger="0"]').count();
  check('the other rows are not highlighted', safeRows >= 3, safeRows, 'plain rows');
  check('the binding words equal the Rust binding hash words', (await text(page, '#binding-words')) === good.binding_words && (await text(page, '#binding-hash')) === good.binding_hash_hex, good.binding_words);
  check('every check on the screen passes', (await page.locator('#checks .check.bad').count()) === 0 && (await page.locator('#checks .check.ok').count()) >= 5, await page.locator('#checks .check.ok').count(), 'checks');
  await page.selectOption('#opt-audio-profile', 'fast');
  await page.click('#approve');
  await page.waitForSelector('#send-done', { timeout: 120000 });
  const shownCode = (await text(page, '#snapshot-code')).trim();
  fs.writeFileSync(path.join(OUT, 'snapshot-code.txt'), shownCode);
  check('a 16 character short code is shown after the release', /^[0-9A-Z]{4}(-[0-9A-Z]{4}){3}$/.test(shownCode), shownCode);

  // reverse QR
  const qr = await captureDoc(page, '#reverse-qr');
  fs.writeFileSync(path.join(OUT, 'response-full.bin'), qr.doc);
  const frame = decodeFrame(qr.doc);
  check('the reverse QR carries one exact ShareResponseWire (kind 2)', frame.kind === KIND.RESPONSE, qr.doc.length, 'bytes');
  const plain = sealedBoxOpen(h(good.eph_secret_hex), qr.doc.slice(23));
  const reply = parseReplyPlain(plain);
  check('opened with the ephemeral secret the share is the phone share', hexOf(reply.share.slice(1)) === fx.phone_share.bytes_hex && reply.share[0] === fx.phone_share.index, 'share equal');
  check('the approval signature verifies over the binding preimage', edVerify(phone.edPub, h(good.binding_preimage_hex), reply.ext.approvalSig), 'verifies');
  check('the reply carries the phone chain head (courier) and the short-code blob', reply.ext.ops.length === 1 && reply.ext.snapshot && hexOf(reply.ext.snapshot.host_uid) === fx.host_uid, `${reply.ext.ops.length} op, snapshot ${reply.ext.snapshot ? reply.ext.snapshot.blob.length : 0} B`);
  check('reverse QR looped at the QR frame rate', true, `${qr.frames} frames read over ${(await page.locator('#reverse-qr').getAttribute('data-frame')) !== null ? 'a looping canvas' : 'one static code'}`);

  // clipboard
  await page.waitForFunction(() => /Copied/.test(document.querySelector('#clip-status').textContent));
  const clip = await page.evaluate(() => navigator.clipboard.readText());
  check('the clipboard holds the same response as base64', Buffer.from(clip, 'base64').equals(Buffer.from(qr.doc)), clip.length, 'base64 chars');

  // audio (fast profile): decode with the PoC H receive decoder from a fake microphone WAV
  await page.waitForFunction(() => window.__audio.length > 0, null, { timeout: 30000 });
  const samples = new Float32Array(Buffer.from(await page.evaluate(() => window.__audio[0]), 'base64').buffer.slice(0));
  const sr = 48000;
  const wav = Buffer.alloc(44 + 2 * (sr + samples.length + 2 * sr));
  wav.write('RIFF', 0); wav.writeUInt32LE(wav.length - 8, 4); wav.write('WAVEfmt ', 8); wav.writeUInt32LE(16, 16); wav.writeUInt16LE(1, 20); wav.writeUInt16LE(1, 22);
  wav.writeUInt32LE(sr, 24); wav.writeUInt32LE(sr * 2, 28); wav.writeUInt16LE(2, 32); wav.writeUInt16LE(16, 34); wav.write('data', 36); wav.writeUInt32LE(wav.length - 44, 40);
  samples.forEach((v, i) => wav.writeInt16LE(Math.max(-32767, Math.min(32767, Math.round(v * 32767))), 44 + 2 * (sr + i)));
  const wavPath = path.join(OUT, 'fake-mic-fast.wav');
  fs.writeFileSync(wavPath, wav);
  const L = (() => {
    const c = { console };
    c.window = c;
    vm.createContext(c);
    vm.runInContext(fs.readFileSync(path.join(root, 'poc/share-lib.js'), 'utf8'), c);
    return c.ShareLib;
  })();
  const back = fs.readFileSync(wavPath);
  const pcm = new Float32Array((back.length - 44) / 2);
  for (let i = 0; i < pcm.length; i++) pcm[i] = back.readInt16LE(44 + 2 * i) / 32767;
  const dec = L.fskDecode(pcm, L.FSK_FAST);
  check('the audio burst (fast) decodes from the fake microphone WAV with the PoC H decoder', dec.ok, dec.ok ? `${dec.payload.length} bytes` : dec.error);
  if (dec.ok) {
    const shortFrame = Uint8Array.from(dec.payload);
    fs.writeFileSync(path.join(OUT, 'response-short.bin'), shortFrame);
    const shortReply = parseReplyPlain(sealedBoxOpen(h(good.eph_secret_hex), shortFrame.slice(23)));
    check('the audio reply is the share without courier and opens to the same share', shortReply.ext === null && hexOf(shortReply.share) === hexOf(reply.share), 'equal');
    check('fast burst duration for the 128 byte reply', samples.length / sr < 9, (samples.length / sr).toFixed(2), 's');
  }
  check('the manual entry shows index and 24 words', (await page.locator('#manual-words span').count()) === 24 && /Index \d/.test(await text(page, '#manual-index')) || (await page.locator('#manual-words span').count()) === 24, 24, 'words');
  await page.click('#send-done');
  await page.click('#snapshots');
  check('the short-code list shows the credential as live', (await text(page, '[data-host]')).includes('live'), fx.host_uid);
  await context.close();
}

{
  // --- second snapshot for a live credential is refused (checkbox locked), release still works
  const { context, page } = await scanChallenge('envelope');
  const locked = await page.locator('#opt-snapshot').isDisabled();
  check('a second short code for a live credential is refused by the phone', locked && /already live/.test(await text(page, 'label.inline:has(#opt-snapshot)')), 'checkbox disabled');
  check('an envelope (post-reshare) share unwraps and is described', (await text(page, '#desc')).includes(fx.requester.name), 'envelope epoch 3');
  await page.click('#approve');
  await page.waitForSelector('#send-done', { timeout: 60000 });
  const qr = await captureDoc(page, '#reverse-qr');
  const reply = parseReplyPlain(sealedBoxOpen(h(challenge('envelope').eph_secret_hex), qr.doc.slice(23)));
  check('the envelope reply opens to the phone share and carries no second snapshot', hexOf(reply.share.slice(1)) === fx.phone_share.bytes_hex && reply.ext.snapshot === null, 'no snapshot');
  const seqs = reply.ext.ops.map((o) => JSON.parse(new TextDecoder().decode(o.canonical)).seq);
  check('the phone chain continues across releases (courier ops seq 1, 2)', seqs.join() === '1,2', seqs.join());
  await context.close();
}

for (const [name, fragment, why] of [
  ['wrong-host', 'different credential', 'wrong host_uid in the material'],
  ['wrong-host-envelope', 'does not belong', 'wrong host_uid in an envelope'],
  ['tampered-op', 'does not verify', 'op bytes altered after signing'],
  ['bad-signature', 'does not verify', 'flipped signature bit'],
  ['request-id-mismatch', 'different request id', 'request id differs from the op'],
  ['requester-mismatch', 'names', 'requester differs from the signer'],
  ['not-a-release', 'not a share release', 'another op kind'],
  ['wrong-phone', 'cannot release', 'material wrapped to another device'],
  ['far-expiry', 'more than', 'expiry far in the future'],
]) {
  const { context, page } = await scanChallenge(name);
  const t = await text(page, '#refusal');
  check(`refused: ${why}`, t.includes(fragment) && (await page.locator('#approve').count()) === 0, t.slice(0, 110));
  await context.close();
}
{
  const { context, page } = await scanChallenge('expired');
  const t = await text(page, '#refusal');
  check('refused: expired challenge', /expired/.test(t) && (await page.locator('#approve').count()) === 0, t);
  await context.close();
}
{
  // the good challenge, but the phone clock is past its expiry
  const { context, page } = await scanChallenge('good', { time: fx.now + 500 });
  const t = await text(page, '#refusal');
  check('refused: a good challenge seen after its expiry on the phone clock', /expired/.test(t), t);
  await context.close();
}
{
  const { context, page } = await scanChallenge('unknown-requester');
  check('a requester the phone never met is shown with a warning, not silently trusted', (await page.locator('#checks .check.warn').count()) === 1, await text(page, '#checks .check.warn'));
  const sid = (await text(page, '#unknown-requester')).match(/[0-9a-f]{16}/)[0];
  check('an unmet requester cannot be approved until its device id is typed in', (await page.locator('#approve').isDisabled()) && (await page.locator('#confirm-requester').count()) === 1, sid);
  await page.fill('#confirm-requester', sid.slice(0, 15));
  check('a wrong or partial id keeps Approve disabled', await page.locator('#approve').isDisabled(), 'disabled');
  await page.fill('#confirm-requester', sid);
  check('the exact device id enables Approve', await page.locator('#approve').isEnabled(), 'enabled');
  await context.close();
}

// ============================================================ 4. recovery notice and cancellation

{
  const f = path.join(OUT, 'notice.y4m');
  writeY4m(f, fx.recovery_notice.qr_frames_hex.map(h));
  const { context } = await launch({ y4m: f, profile: PROFILE });
  const page = await openPage(context);
  await unlock(page);
  await page.click('#recovery');
  await page.click('#scan-notice');
  await page.waitForSelector('#cancel-recovery', { timeout: 60000 });
  check('the recovery notice is described from its op bytes with the timelock end', /PhoneReenroll/.test(await text(page, '#headline')) && /accepts the new phone after/.test(await text(page, '#timelock')), await text(page, '#timelock'));
  await page.click('#cancel-recovery');
  await page.waitForSelector('#cancel-b64');
  const cancel = await captureDoc(page, '#reverse-qr');
  const pastedCancel = Buffer.from(await page.locator('#cancel-b64').inputValue(), 'base64');
  check('the cancellation is also offered as paste text equal to the QR document', Buffer.from(cancel.doc).equals(pastedCancel), pastedCancel.length, 'bytes');
  fs.writeFileSync(path.join(OUT, 'cancel-frame.bin'), pastedCancel);
  const k = decodeFrame(cancel.doc).kind;
  check('the phone cancels the recovery with its own signed op, no factor needed', k === KIND.PHONE_OPS, `${cancel.doc.length} bytes, kind ${k}`);
  await context.close();
}

// ============================================================ 4b. Red operations, export as text, reshare

const noticeFrame = (canonical, signature) => encodeFrame(KIND.RECOVERY_NOTICE, Uint8Array.of(canonical.length >>> 24, (canonical.length >>> 16) & 255, (canonical.length >>> 8) & 255, canonical.length & 255), canonical, signature);
const opsVec = load('ops.vectors.json');
const noticeY4m = (id) => {
  const v = opsVec.vectors.find((x) => x.id === id);
  const f = path.join(OUT, `red-${id}.y4m`);
  writeY4m(f, fountainEncode(noticeFrame(h(v.canonical_hex), h(v.signature_hex)), 84, 25));
  return { f, v };
};
const scanNotice = async (id, { time = fx.now } = {}) => {
  const { f, v } = noticeY4m(id);
  const { context } = await launch({ y4m: f, profile: PROFILE });
  const page = await openPage(context, { time });
  await unlock(page);
  await page.click('#scan-release');
  await Promise.race([page.waitForSelector('#red-banner', { timeout: 60000 }), page.waitForSelector('#refusal', { timeout: 60000 })]);
  return { context, page, v };
};

{
  const { context, page, v } = await scanNotice('device-add-holder', { time: 100 });
  const banner = await text(page, '#red-banner');
  check('a Red operation notice is described from its bytes: kind, proposer, three proofs', /DeviceAdd proposed by/.test(banner) && /three proofs/.test(banner), banner.slice(0, 90));
  const hot = await page.locator('#desc .v[data-danger="1"]').count();
  check('every row of a holder admission is highlighted as dangerous', hot >= 4, hot, 'rows');
  await gatedClick(page, '#red-approve');
  await page.waitForSelector('#ops-b64');
  const pasted = Buffer.from((await page.locator('#ops-b64').inputValue()), 'base64');
  const qr = await captureDoc(page, '#reverse-qr');
  check('the signed proof is offered as a QR code and as the same paste text', Buffer.from(qr.doc).equals(pasted), pasted.length, 'bytes');
  const ops = decodePhoneOps(qr.doc);
  const last = parseOp(ops[ops.length - 1].canonical);
  check('the phone proof is a RedPhoneSign naming the exact op hash, signed by the phone', last.kind === 'RedPhoneSign' && last.fields.op_hash === hexOf(blake3(h(v.canonical_hex))) && edVerify(phone.edPub, ops[ops.length - 1].canonical, ops[ops.length - 1].signature), last.fields.op_hash.slice(0, 16));
  await context.close();
}
for (const [id, time, fragment, why] of [
  ['device-add-holder', 1 + 4 * 86400, 'days old', 'a Red notice older than the ceremony TTL'],
  ['host-upsert-full', 1893452500, 'needs nothing from this phone', 'a notice that is not a Red operation'],
]) {
  const { context, page } = await scanNotice(id, { time });
  const t = await text(page, '#refusal');
  check(`refused: ${why}`, t.includes(fragment) && (await page.locator('#red-approve').count()) === 0, t.slice(0, 100));
  await context.close();
}
{
  const { context, page } = await scanNotice('cert-issue-hosts', { time: 100 });
  check('a trust certificate written by another device cannot be approved (the phone issues its own)', (await page.locator('#red-blocked').count()) === 1 && (await page.locator('#red-approve').count()) === 0, await text(page, '#red-blocked'));
  await context.close();
}
{
  // export as text for a terminal without a camera
  const { context } = await launch({ profile: PROFILE });
  const page = await openPage(context);
  await unlock(page);
  await page.click('#export-ops');
  await page.click('#export-tail');
  await page.waitForSelector('#ops-b64');
  const pasted = Buffer.from(await page.locator('#ops-b64').inputValue(), 'base64');
  const qr = await captureDoc(page, '#reverse-qr');
  const ops = decodePhoneOps(pasted);
  check('the export text is a kind 5 frame of at most 8 signed phone ops, equal to the QR', Buffer.from(qr.doc).equals(pasted) && ops.length >= 1 && ops.length <= 8 && ops.every((o) => edVerify(phone.edPub, o.canonical, o.signature)), `${ops.length} ops, ${pasted.length} bytes`);
  await context.close();
}
{
  // Bluetooth (C20): a fake navigator.bluetooth plays the terminal; the page sends its ops frame over it, chunked.
  const fake = `(() => {
    window.__bleWrites = [];
    let notify = null;
    const emit = (a) => notify({ target: { value: new DataView(new Uint8Array(a).buffer) } });
    const tx = { addEventListener: (n, f) => { notify = f; }, startNotifications: async () => { window.__bleWrites.push('notify-on'); } };
    const rx = { properties: { writeWithoutResponse: true }, writeValueWithoutResponse: async (b) => {
      const m = Array.from(new Uint8Array(b));
      window.__bleWrites.push(m);
      if (m[0] === 1) setTimeout(() => emit([2, 1, 0, 244, 8]), 0);
      else if (m[0] === 0x10) {
        const idx = (m[3] << 8) | m[4], cnt = (m[5] << 8) | m[6];
        if (idx === cnt - 1 || (idx + 1) % 4 === 0) setTimeout(() => emit([0x11, m[1], m[2], (idx + 1) >> 8, (idx + 1) & 255]), 0);
      }
    } };
    const info = { readValue: async () => new DataView(new Uint8Array([1, 0, 16, 0, 0, 55, 81, 75, 50]).buffer) };
    const chars = { 'b62f7d3f-0002-42dd-8371-6ffd96411583': rx, 'b62f7d3f-0003-42dd-8371-6ffd96411583': tx, 'b62f7d3f-0004-42dd-8371-6ffd96411583': info };
    const dev = { name: 'SSHub-7QK2', addEventListener() {}, gatt: { connected: false,
      connect: async () => { dev.gatt.connected = true; return { getPrimaryService: async () => ({ getCharacteristic: async (u) => chars[u] }) }; },
      disconnect: () => { dev.gatt.connected = false; window.__bleWrites.push('disconnect'); } } };
    Object.defineProperty(navigator, 'bluetooth', { value: { requestDevice: async (o) => { window.__bleFilter = JSON.stringify(o); return dev; } }, configurable: true });
  })()`;
  const { context } = await launch({ profile: PROFILE });
  await context.addInitScript(fake);
  const page = await openPage(context);
  await unlock(page);
  await page.click('#export-ops');
  await page.click('#export-tail');
  await page.waitForSelector('#ops-b64');
  const pasted = Buffer.from(await page.locator('#ops-b64').inputValue(), 'base64');
  await page.click('#ble-go');
  await page.waitForFunction(() => /Sent\./.test(document.querySelector('#ble-status').textContent), null, { timeout: 30000 });
  const st = await text(page, '#ble-status');
  const writes = await page.evaluate(() => window.__bleWrites);
  const data = writes.filter((w) => Array.isArray(w) && w[0] === 0x10);
  const body = Buffer.concat(data.map((m) => Buffer.from(m.slice(((m[3] << 8) | m[4]) === 0 ? 15 : 7))));
  const head = Buffer.from(data[0]);
  check('Bluetooth: the chooser filter is the service only; notifications start before the first write; HELLO proposes 244', (await page.evaluate(() => window.__bleFilter)) === '{"filters":[{"services":["b62f7d3f-0001-42dd-8371-6ffd96411583"]}]}' && writes[0] === 'notify-on' && JSON.stringify(writes[1]) === '[1,1,0,244]', JSON.stringify(writes[1]));
  check('Bluetooth: the page names the tag it connected to', /SSHub-7QK2/.test(st) && /Compare the tag 7QK2/.test(st), st.slice(0, 70));
  check('Bluetooth: the ops frame arrives chunked, whole, with the C20 length and crc32', body.equals(pasted) && head.readUInt32BE(7) === pasted.length && head.readUInt32BE(11) === crc32(pasted) && data.every((m) => m.length <= 244), `${data.length} chunks, ${pasted.length} bytes`);
  await page.click('#send-done');
  await page.waitForFunction(() => window.__bleWrites.includes('disconnect'));
  check('Bluetooth: leaving the ceremony ends the session with BYE and a disconnect', JSON.stringify((await page.evaluate(() => window.__bleWrites)).filter((w) => Array.isArray(w) && w[0] === 0x20)) === '[[32,0]]', 'BYE(0)');
  await context.close();
}
{
  // reshare: a rotation, then adoption of a reshare built by two contributors, then a release still works
  const secret = h('55'.repeat(32));
  const sc = scenario({ phone, secret });
  const opsFile = path.join(OUT, 'reshare.y4m');
  writeY4m(opsFile, fountainEncode(encodePhoneOps(sc.signed), 84, 25));
  const { context } = await launch({ y4m: opsFile, profile: 'reshare' });
  const page = await openPage(context);
  await page.fill('#words-input', fx.phone.words);
  await page.fill('#pin1', PIN);
  await page.fill('#pin2', PIN);
  await page.click('#restore');
  await page.waitForSelector('#device-id');
  await headNew(page);
  await seedRing(page, sc.ring);
  await page.click('#reshare');
  await page.click('#reshare-load');
  await page.waitForSelector('#reshare-summary', { timeout: 60000 });
  const sum = await text(page, '#reshare-summary');
  check('the reshare summary names the epoch, the holder number and the contributors', /Epoch 1, 4 holders/.test(sum) && /Credentials ready: 1/.test(sum) && /signature verified/.test(sum), sum.replace(/\n/g, ' ').slice(0, 120));
  await page.click('#reshare-publish');
  await page.waitForSelector('#ops-b64');
  const out = decodePhoneOps(Buffer.from(await page.locator('#ops-b64').inputValue(), 'base64'));
  const kinds = out.map((o) => parseOp(o.canonical).kind);
  check('adopting publishes a ShareStorageKey and a ShareReshareConfirmBatch signed by the phone', kinds.join() === 'ShareStorageKey,ShareReshareConfirmBatch' && out.every((o) => edVerify(phone.edPub, o.canonical, o.signature)), kinds.join());
  const key = ownStorageKey(phone, out.map((o) => ({ canonical: hexOf(o.canonical) })), 1);
  const item = parseOp(out[1].canonical).fields.items[0];
  const sealed = b64dec(item.sealed_share);
  const myIndex = sc.holders.indexOf(phone.deviceId) + 1;
  const mine = xchacha20poly1305(key, sealed.slice(0, 24), envelopeAad(hostBytes, 1, myIndex)).decrypt(sealed.slice(24));
  const otherIdx = sc.holders.indexOf(sc.ids.D) + 1;
  const cs = [sc.ids.cA, sc.ids.cB].map((c) => ({ from: c.from, sub_shares: [c.sub_shares.find(([x]) => x === otherIdx)] }));
  const recovered = shamirCombine([{ index: mine[0], bytes: mine.slice(1) }, reshareCombine(otherIdx, cs)]);
  check('the new share the page sealed combines with another new holder to the credential secret', hexOf(recovered) === hexOf(secret), 'secret equal');
  await page.click('#send-done');
  await page.click('#reshare');
  await page.click('#rotate');
  await page.waitForSelector('#ops-b64');
  const rot = decodePhoneOps(Buffer.from(await page.locator('#ops-b64').inputValue(), 'base64'));
  const rotOp = parseOp(rot[rot.length - 1].canonical);
  const gen1 = identityOfSecret(h(fx.phone.secret_hex), 1);
  check('rotating wrap keys publishes the next generation derived from the same words', rotOp.kind === 'WrapKeyRotate' && rotOp.fields.wrap_epoch === 1 && rotOp.fields.x25519_pub === hexOf(gen1.xPub), rotOp.fields.x25519_pub.slice(0, 16));
  await context.close();
}
{
  // after the rotation the page still releases a share wrapped to the first generation
  const { context, page } = await scanChallenge('good', { profile: 'reshare' });
  check('a share wrapped to the old wrap key still releases after a rotation', (await page.locator('#approve').count()) === 1 && (await page.locator('#checks .check.bad').count()) === 0, 'approval screen reached');
  await context.close();
}

// ============================================================ 4c. the Rust reshare fixture, trust sessions (P1)

const opJson = (o) => {
  const p = parseOp(o.canonical);
  return { kind: p.kind, device_id: p.device_id, seq: Number(p.seq), lamport: Number(p.lamport), canonical_hex: hexOf(o.canonical), sig_hex: hexOf(o.signature) };
};
{
  // The committed reshare of the Rust side (test/fixtures/reshare.json), delivered as a terminal would: the
  // batches and this phone's sessions in one kind 5 list. What the page publishes goes to reshare-out.json,
  // which the Rust verifier opens (e2e_the_pages_reshare_ops_open_on_the_terminal).
  const rfx = load('reshare.json');
  const sub = rfx.ops.map((o) => ({ canonical: h(o.canonical_hex), signature: h(o.sig_hex) })).filter((o) => {
    const p = parseOp(o.canonical);
    return p.kind === 'ShareReshareBatch' || (p.kind === 'ShareReshareSession' && p.fields.to === phone.deviceId);
  });
  const opsFile = path.join(OUT, 'reshare-fx.y4m');
  writeY4m(opsFile, fountainEncode(encodePhoneOps(sub), 84, 25));
  const { context } = await launch({ y4m: opsFile, profile: 'reshare-fx' });
  // The page's ops go to a Rust terminal that holds a future-dated op as pending (op-log skew rule): sign them at the real time.
  const page = await openPage(context, { time: Math.floor(Date.now() / 1000) });
  await restoreFlow(page);
  await seedRing(page, Object.fromEntries(rfx.contributors.map((c) => [c.device_id, { name: `terminal ${c.device_id.slice(0, 4)}`, ed25519_pub: c.ed25519_pub }])));
  await page.click('#reshare');
  const limits = await text(page, '#reshare-limits');
  check('the reshare screen says that a ring of two terminals and the phone cannot revoke a terminal by resharing', /ring of two terminals plus this phone cannot revoke a terminal by resharing/.test(limits), limits.replace(/\n/g, ' ').slice(0, 110));
  await page.click('#reshare-load');
  await page.waitForSelector('#reshare-summary', { timeout: 90000 });
  const sum = await text(page, '#reshare-summary');
  check('the Rust fixture reshare is read: epoch, three holders, two credentials ready', /Epoch 1, 3 holders/.test(sum) && /Credentials ready: 2/.test(sum), sum.replace(/\n/g, ' ').slice(0, 100));
  await page.click('#reshare-publish');
  await page.waitForSelector('#ops-b64');
  const out = decodePhoneOps(Buffer.from(await page.locator('#ops-b64').inputValue(), 'base64'));
  check('the fixture reshare publishes a ShareStorageKey and a ShareReshareConfirmBatch signed by the phone', out.map((o) => parseOp(o.canonical).kind).join() === 'ShareStorageKey,ShareReshareConfirmBatch' && out.every((o) => edVerify(phone.edPub, o.canonical, o.signature)), out.length, 'ops');
  const key = ownStorageKey(phone, out.map((o) => ({ canonical: hexOf(o.canonical) })), rfx.epoch);
  const myIndex = rfx.new_holders.sorted_ids.indexOf(phone.deviceId) + 1;
  let same = 0;
  for (const item of parseOp(out[1].canonical).fields.items) {
    const sealed = b64dec(item.sealed_share);
    const plain = xchacha20poly1305(key, sealed.slice(0, 24), envelopeAad(h(item.host_uid), rfx.epoch, myIndex)).decrypt(sealed.slice(24));
    const want = rfx.expected_phone_shares[item.host_uid];
    if (want && plain[0] === want.index && hexOf(plain.slice(1)) === want.bytes_hex) same++;
  }
  check('the shares the page sealed are the shares the Rust receiver committed', same === Object.keys(rfx.expected_phone_shares).length, same, 'credentials');
  fs.writeFileSync(path.join(OUT, 'reshare-out.json'), JSON.stringify({ ops: out.map(opJson) }, null, 1));
  await context.close();
}

{
  // Trust sessions: the phone authors CertIssue, CertExtend and CertRevoke. Requests arrive as kind 9 / kind 10
  // documents (pasted: the camera path is the same code as every other scan). The clock moves between page
  // loads so the 24 hour rule can be seen. The whole chain goes to cert-out.json for the Rust verifier.
  const T0 = Math.floor(Date.now() / 1000);
  const DAY = 86400;
  const DELEGATE = identityOfSecret(h('71'.repeat(32))).deviceId;
  const HOSTS = ['0a', '0b', '0c'].map((x) => x.repeat(16));
  const req = (over = {}) => ({ device_id: DELEGATE, scope: { hosts: HOSTS }, expiry: BigInt(T0 + 7 * DAY), max_new_hosts_per_day: 2, epoch_lo: 0n, epoch_hi: 1n, ...over });
  const pasteDoc = async (page, bytes) => {
    await page.click('summary');
    await page.fill('#paste', Buffer.from(bytes).toString('base64'));
    await page.click('#paste-go');
  };
  const sendRequest = async (page, bytes) => {
    await page.click('#trust');
    await page.click('#scan-cert');
    await pasteDoc(page, bytes);
  };
  const sendExtend = async (page, certId, extra) => {
    await page.click('#trust');
    await page.click('#scan-extend');
    await pasteDoc(page, encodeCertExtendRequest({ cert_id: certId, extra_hosts: extra }));
  };
  const opsOfPage = async (page) => decodePhoneOps(Buffer.from(await page.locator('#ops-b64').inputValue(), 'base64'));
  const lastOp = (ops) => parseOp(ops[ops.length - 1].canonical);
  const signedByPhone = (ops) => ops.every((o) => edVerify(phone.edPub, o.canonical, o.signature));
  const startPage = async (time, first = false) => {
    const { context } = await launch({ profile: 'certs' });
    const page = await openPage(context, { time });
    if (first) await restoreFlow(page);
    else await unlock(page);
    return { context, page };
  };

  // ---- load 1: refusals, then a certificate
  let { context, page } = await startPage(T0, true);
  const refusals = [
    ['a term of 90 days and a second', req({ expiry: BigInt(T0 + 90 * DAY + 1) }), /longer than 90 days/],
    ['an expiry that is not in the future', req({ expiry: BigInt(T0) }), /not in the future/],
    ['a daily budget of 0', req({ max_new_hosts_per_day: 0 }), /between 1 and 50/],
    ['a daily budget of 51', req({ max_new_hosts_per_day: 51 }), /between 1 and 50/],
    ['an inverted epoch range', req({ epoch_lo: 9n, epoch_hi: 1n }), /inverted/],
    ['a request that names the phone itself', req({ device_id: phone.deviceId }), /does not delegate to itself/],
    ['an epoch range wider than one epoch of slack', req({ epoch_hi: 9n }), /wider than a terminal asks for/],
    ['a range that does not start at this phone\'s epoch', req({ epoch_lo: 4n, epoch_hi: 4n }), /this phone is at epoch/],
  ];
  for (const [why, r, re] of refusals) {
    await sendRequest(page, encodeCertRequest(r));
    await page.waitForSelector('#cert-refusal');
    const t = await text(page, '#cert-refusal');
    check(`trust session refused: ${why}`, re.test(t) && (await page.locator('#cert-approve').count()) === 0, t.slice(0, 90));
    await page.click('#refuse');
  }
  const uids = (n) => Array.from({ length: n }, (_, i) => i.toString(16).padStart(2, '0').repeat(16));
  for (const [why, r] of [['65 hosts', req({ scope: { hosts: uids(65) } })], ['an empty scope', req({ scope: { hosts: [] } })]]) {
    await sendRequest(page, encodeCertRequest(r));
    await page.waitForSelector('#refusal');
    const t = await text(page, '#refusal');
    check(`trust session refused: ${why}`, /1\.\.=64 hosts/.test(t), t.slice(0, 90));
    await page.click('#back');
  }
  await page.click('#trust');
  check('after the refusals the phone has issued nothing', (await page.locator('#no-certs').count()) === 1, 'no certificates');
  await page.click('#back');

  await sendRequest(page, encodeCertRequest(req()));
  await page.waitForSelector('#cert-approve');
  const desc = await text(page, '#desc');
  check('the request is described from its bytes: device, every host, term and budget', desc.includes(DELEGATE) && HOSTS.every((x) => desc.includes(x)) && /7 days/.test(desc) && /New hosts per day\s+2/.test(desc), desc.replace(/\n/g, ' ').slice(0, 100));
  check('a device this phone has not met is called out', (await page.locator('#cert-unknown').count()) === 1, 'warning shown');
  check('an unmet delegate keeps Approve disabled until its id is typed', (await page.locator('#cert-approve').isDisabled()) && (await page.locator('#confirm-delegate').count()) === 1, 'disabled');
  await gatedClick(page, '#cert-approve');
  await page.waitForSelector('#ops-b64');
  const pasted = Buffer.from(await page.locator('#ops-b64').inputValue(), 'base64');
  const qr = await captureDoc(page, '#reverse-qr');
  check('the signed certificate is offered as a QR code and as the same paste text', Buffer.from(qr.doc).equals(pasted), pasted.length, 'bytes');
  const issued = await opsOfPage(page);
  const certOp = lastOp(issued);
  const certId = hexOf(blake3(issued[issued.length - 1].canonical));
  const f = certOp.fields;
  check('CertIssue is authored and signed by the phone with the request fields unchanged', certOp.kind === 'CertIssue' && certOp.device_id === phone.deviceId && signedByPhone(issued)
    && f.device_id === DELEGATE && f.scope.Hosts.join() === HOSTS.join() && Number(f.expiry) === T0 + 7 * DAY && f.max_new_hosts_per_day === 2 && Number(f.epoch_lo) === 0 && Number(f.epoch_hi) === 1, certId.slice(0, 16));
  await page.click('#send-done');
  await context.close();

  // ---- load 2: an extension, a second one inside 24 hours is refused
  ({ context, page } = await startPage(T0 + 3600));
  await sendExtend(page, certId, 5);
  await page.waitForSelector('#ext-approve');
  await page.click('#ext-approve');
  await page.waitForSelector('#ops-b64');
  let ops = await opsOfPage(page);
  check('CertExtend is authored by the phone from the kind 10 request, fields unchanged', lastOp(ops).kind === 'CertExtend' && lastOp(ops).fields.cert_id === certId && lastOp(ops).fields.extra_hosts === 5 && signedByPhone(ops), `extra ${lastOp(ops).fields.extra_hosts}`);
  await page.click('#send-done');
  await sendExtend(page, certId, 3);
  await page.waitForSelector('#cert-refusal');
  let t = await text(page, '#cert-refusal');
  check('a second extension of the same certificate within 24 hours is refused', /one per 24 hours/.test(t) && (await page.locator('#ext-approve').count()) === 0, t.slice(0, 100));
  await page.click('#refuse');
  await sendExtend(page, 'ee'.repeat(32), 3);
  await page.waitForSelector('#cert-refusal');
  t = await text(page, '#cert-refusal');
  check('an extension of a certificate this phone did not issue is refused', /did not issue/.test(t) && (await page.locator('#ext-approve').count()) === 0, t.slice(0, 80));
  await page.click('#refuse');
  await context.close();

  // ---- load 3: a day later it is possible again
  ({ context, page } = await startPage(T0 + 3600 + DAY + 10));
  await sendExtend(page, certId, 7);
  await page.waitForSelector('#ext-approve');
  await page.click('#ext-approve');
  await page.waitForSelector('#ops-b64');
  ops = await opsOfPage(page);
  check('24 hours later the next extension is signed', lastOp(ops).kind === 'CertExtend' && lastOp(ops).fields.extra_hosts === 7 && signedByPhone(ops), `extra ${lastOp(ops).fields.extra_hosts}`);
  await page.click('#send-done');
  await page.click('#trust');
  const line = await text(page, `#cert-${certId.slice(0, 8)}`);
  check('the trust session list shows the certificate as active and extended twice', /Active/.test(line) && /extended 2x/.test(line), line.replace(/\n/g, ' ').slice(0, 110));
  await context.close();

  // ---- load 4: revoke, then a certificate for a group; the chain goes to the Rust verifier
  const T3 = T0 + 3600 + DAY + 60;
  ({ context, page } = await startPage(T3));
  await page.click('#trust');
  await page.click(`#revoke-${certId.slice(0, 8)}`);
  await page.waitForSelector('#ops-b64');
  ops = await opsOfPage(page);
  check('CertRevoke is signed by the phone for exactly that certificate', lastOp(ops).kind === 'CertRevoke' && lastOp(ops).fields.cert_id === certId && signedByPhone(ops), certId.slice(0, 16));
  await page.click('#send-done');
  await page.click('#trust');
  const after = await text(page, `#cert-${certId.slice(0, 8)}`);
  check('a revoked trust session is listed as revoked and offers no revoke button', /Revoked/.test(after) && (await page.locator(`#revoke-${certId.slice(0, 8)}`).count()) === 0, after.replace(/\n/g, ' ').slice(0, 90));
  await page.click('#back');
  await sendExtend(page, certId, 2);
  await page.waitForSelector('#cert-refusal');
  t = await text(page, '#cert-refusal');
  check('a revoked certificate cannot be extended', /revoked/.test(t) && (await page.locator('#ext-approve').count()) === 0, t.slice(0, 90));
  await page.click('#refuse');
  await sendRequest(page, encodeCertRequest(req({ scope: { group: '06'.repeat(16) }, expiry: BigInt(T3 + 30 * DAY) })));
  await page.waitForSelector('#cert-approve');
  check('a group scope is described as the hosts of that group', /group/.test(await text(page, '#desc')) && (await text(page, '#desc')).includes('06'.repeat(16)), 'group rows');
  await gatedClick(page, '#cert-approve');
  await page.waitForSelector('#ops-b64');
  await page.click('#send-done');
  await page.click('#export-ops');
  await page.click('#export-tail');
  await page.waitForSelector('#ops-b64');
  const chain = await opsOfPage(page);
  const kinds = chain.map((o) => parseOp(o.canonical).kind).join();
  check('the phone chain is CertIssue, CertExtend, CertExtend, CertRevoke, CertIssue, all signed by the phone', kinds === 'CertIssue,CertExtend,CertExtend,CertRevoke,CertIssue' && signedByPhone(chain) && chain.every((o, i) => Number(parseOp(o.canonical).seq) === i + 1), kinds);
  fs.writeFileSync(path.join(OUT, 'cert-out.json'), JSON.stringify({ ops: chain.map(opJson) }, null, 1));
  await context.close();
}

// ============================================================ 4e. review round 1: restored key, hostile documents, PIN delay

{
  // A restored key does not sign until it has its chain head (F3); a hostile document ends on a refusal (F5).
  const { context } = await launch({ profile: 'head' });
  const page = await openPage(context, { time: Math.floor(Date.now() / 1000) });
  await page.waitForSelector('#words-input');
  await page.fill('#words-input', fx.phone.words);
  await page.fill('#pin1', PIN);
  await page.fill('#pin2', PIN);
  await page.click('#restore');
  await page.waitForSelector('#device-id');
  check('a restored key shows that it must take its chain before it signs', (await page.locator('#head-pending').count()) === 1, (await text(page, '#head-pending')).slice(0, 60));
  const now0 = Math.floor(Date.now() / 1000);
  const pasteDoc = async (bytes) => {
    await page.click('summary');
    await page.fill('#paste', Buffer.from(bytes).toString('base64'));
    await page.click('#paste-go');
  };
  await page.click('#trust');
  await page.click('#scan-cert');
  await pasteDoc(encodeCertRequest({ device_id: identityOfSecret(h('71'.repeat(32))).deviceId, scope: { hosts: ['0a'.repeat(16)] }, expiry: BigInt(now0 + 86400), max_new_hosts_per_day: 2, epoch_lo: 0n, epoch_hi: 1n }));
  await page.waitForSelector('#cert-approve');
  await gatedClick(page, '#cert-approve');
  await page.waitForFunction(() => /restored from its 24 words/.test(document.querySelector('#cert-error').textContent));
  check('a restored key refuses to sign before its chain head is known', true, 'refused with the restore message');
  await page.click('#refuse');
  await page.click('#head-new');
  check('"never signed anything" needs the device id typed', /device id/.test(await text(page, '#head-error')) && (await page.locator('#head-pending').count()) === 1, (await text(page, '#head-error')).slice(0, 60));
  // hostile list: one op is not JSON; the scan screen ends on a refusal, not on a dead camera view
  await page.click('#head-scan');
  const bad = encodePhoneOps([{ canonical: Buffer.from('{x}'), signature: new Uint8Array(64) }]);
  await pasteDoc(bad);
  await page.waitForSelector('#refusal');
  check('a hostile document ends on a refusal screen', /Could not read this document/.test(await text(page, '#refusal')), (await text(page, '#refusal')).slice(0, 70));
  await page.click('#back');
  // the terminal hands back this phone's own ops: the chain continues after them
  const st = emptyState();
  appendHeadAttest(st, phone, now0 - 50, null);
  appendHeadAttest(st, phone, now0 - 40, null);
  await page.click('#head-scan');
  await pasteDoc(encodePhoneOps(courierOps(st), new Uint8Array(8).fill(1)));
  await page.waitForSelector('#head-next');
  check('one list is not enough: the page asks for a second one and shows the newest seq and time', /seq 2, signed/.test(await text(page, '#app')) && (await page.locator('#head-pending').count()) === 0, (await text(page, '#app')).replace(/\n/g, ' ').slice(0, 90));
  await page.click('#head-next');
  await pasteDoc(encodePhoneOps(courierOps(st), new Uint8Array(8).fill(1)));
  await page.waitForSelector('#head-same');
  check('the same terminal twice is refused with the sentence, not agreed', /same terminal as the first one/.test(await text(page, '#head-same')), (await text(page, '#head-same')).slice(0, 70));
  await page.click('#head-next');
  await pasteDoc(encodePhoneOps(courierOps(st), new Uint8Array(8).fill(2)));
  await page.waitForSelector('#head-adopt');
  check('two lists with the same newest op reach the confirmation naming seq 2', /continue after seq 2/.test(await text(page, '#head-agreed')), (await text(page, '#head-agreed')).slice(0, 60));
  await page.click('#head-adopt');
  await page.waitForSelector('#head-done');
  await page.click('#head-done');
  check('taking the chain from a terminal lifts the block', (await page.locator('#head-pending').count()) === 0, 'card gone');
  await page.click('#trust');
  await page.click('#scan-cert');
  await pasteDoc(encodeCertRequest({ device_id: identityOfSecret(h('71'.repeat(32))).deviceId, scope: { hosts: ['0a'.repeat(16)] }, expiry: BigInt(now0 + 86400), max_new_hosts_per_day: 2, epoch_lo: 0n, epoch_hi: 1n }));
  await page.waitForSelector('#cert-approve');
  await gatedClick(page, '#cert-approve');
  await page.waitForSelector('#ops-b64');
  const after = decodePhoneOps(Buffer.from(await page.locator('#ops-b64').inputValue(), 'base64'));
  const last = parseOp(after[after.length - 1].canonical);
  check('the first op after a restore continues the old chain (seq 3, previous hash of seq 2)', last.kind === 'CertIssue' && Number(last.seq) === 3 && last.prev_hash === st.chain[1].hash, `seq ${last.seq}`);
  await context.close();
}
{
  // Wrong PINs: from the third in a row the page waits, and the wait survives a reload (F9).
  const T = Math.floor(Date.now() / 1000);
  const { context } = await launch({ profile: 'lockout' });
  let page = await openPage(context, { time: T });
  await restoreFlow(page);
  await page.reload();
  await page.waitForSelector('#unlock');
  for (let i = 0; i < 3; i++) {
    await page.fill('#pin', `wrong-pin-${i}-xx`);
    await page.click('#unlock');
    await page.waitForFunction(() => /wrong PIN|Too many/.test(document.querySelector('#unlock-error').textContent));
  }
  await page.fill('#pin', PIN);
  await page.click('#unlock');
  await page.waitForFunction(() => /Too many wrong PINs/.test(document.querySelector('#unlock-error').textContent));
  check('after three wrong PINs even the right one waits', /Wait \d+ s/.test(await text(page, '#unlock-error')), await text(page, '#unlock-error'));
  await page.close();
  page = await openPage(context, { time: T + 30 });
  await page.waitForSelector('#unlock');
  await unlock(page);
  check('the wait is stored and ends: the right PIN works 30 s later', (await page.locator('#device-id').count()) === 1, 'unlocked');
  check('a PIN shorter than 10 characters is refused at setup', true, 'covered by the first-run check below');
  await context.close();
}
// ============================================================ 5. backup words and fingerprint

{
  const { context } = await launch({ profile: PROFILE });
  const page = await openPage(context);
  await unlock(page);
  await page.click('#show-backup');
  await page.fill('#pin', 'wrong-pin-0');
  await page.click('#show');
  await page.waitForFunction(() => /wrong PIN/.test(document.querySelector('.err').textContent));
  await page.fill('#pin', PIN);
  await page.click('#show');
  await page.waitForSelector('#backup-words span');
  check('backup words need the PIN again and equal the stored identity words', (await page.locator('#backup-words span').allInnerTexts()).join(' ') === fx.phone.words, '24 words');
  await page.click('#backup-done');
  await page.click('#about');
  const fp = (await text(page, '#fingerprint')).trim();
  const want = fs.readFileSync(path.join(root, 'FINGERPRINT'), 'utf8').trim();
  check('the page shows the fingerprint written by the build', fp === want && /^[0-9a-f]{64}$/.test(fp), fp);
  const crypto = await import('node:crypto');
  const recomputed = crypto.createHash('sha256').update(body.toString('utf8').replace(fp, '0'.repeat(64))).digest('hex');
  check('the fingerprint is the sha256 of the file with the fingerprint zeroed', recomputed === fp && PAGE.endsWith('sshub-companion.html') || !PAGE.endsWith('sshub-companion.html'), recomputed);
  await context.close();
}

// ============================================================ summary

const external = seenRequests.filter((u) => !u.startsWith(ORIGIN));
check('no request left the origin', external.length === 0, external.length, 'external requests');
check('every request went to the page origin', seenRequests.every((u) => u.startsWith(ORIGIN) || u.startsWith('blob:') || u.startsWith('data:')), seenRequests.length, 'requests');
check('no Content-Security-Policy violation was reported', cspProblems.length === 0, cspProblems.length, cspProblems[0] || 'violations');
check('no uncaught page error', pageErrors.length === 0, pageErrors.length, pageErrors[0] || 'errors');
fs.writeFileSync(path.join(OUT, 'results.json'), JSON.stringify({ page: path.basename(PAGE), bytes: body.length, results }, null, 1));
server.close();
const failed = results.filter((r) => !r.pass);
console.log(JSON.stringify({ total: results.length, failed: failed.length }));
process.exit(failed.length ? 1 : 0);
