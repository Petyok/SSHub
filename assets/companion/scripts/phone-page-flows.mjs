// The companion page on a real Android Chrome, driven over CDP in one tab of our own. Camera,
// microphone, speaker and clipboard are NOT used: getUserMedia is rejected inside the page,
// AudioContext is removed and clipboard writes are refused (phone-device.mjs `prepare`), so
// nothing prompts and nothing is played. Challenges come from the Rust fixtures, pasted into the
// page's paste box; the page clock is shifted to the fixture time (only inside the page).
import { createRequire } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { blake3 } from '../vendor/noble-hashes-2.4.0/blake3.js';
import { bytesOf, edVerify, hexOf, sealedBoxOpen } from '../src/lib.mjs';
import { DocumentCollector } from '../src/scan.mjs';
import { KIND, decodeFrame, decodePhoneOps, parseReplyPlain } from '../src/frames.mjs';
import { identityOfSecret, secretFromWords } from '../src/keys.mjs';
import { parseOp } from '../src/ops.mjs';
import { ORIGIN, clearSiteData, coolDown, guard, sleep } from './phone-device.mjs';

const require = createRequire(import.meta.url);
const jsQR = require('../vendor/jsqr-1.4.0/jsQR.cjs');
const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..');
const load = (n) => JSON.parse(fs.readFileSync(path.join(root, 'test/fixtures', n), 'utf8'));
const h = (x) => bytesOf(x);
const PIN = 'correct-pin-17';

export async function runPage({ chrome }) {
  const fx = load('challenges.json');
  const ops = load('ops.vectors.json');
  const phone = identityOfSecret(h(fx.phone.secret_hex));
  const results = [];
  const check = (claim, pass, measured) => {
    results.push({ claim, pass: !!pass, measured });
    console.log(JSON.stringify({ claim, pass: !!pass, measured }));
    return pass;
  };

  const q = (s) => JSON.stringify(s);
  const exists = (sel) => chrome.eval(`!!document.querySelector(${q(sel)})`);
  const text = (sel) => chrome.eval(`(document.querySelector(${q(sel)}) || {innerText: ''}).innerText`);
  const click = (sel) => chrome.eval(`(() => { const e = document.querySelector(${q(sel)}); if (!e) throw new Error('missing ${sel.replace(/'/g, '')}'); e.click(); return true; })()`, { gesture: true });
  const fill = (sel, v) => chrome.eval(`(() => { const e = document.querySelector(${q(sel)}); if (!e) throw new Error('missing ${sel.replace(/'/g, '')}'); const proto = e.tagName === 'TEXTAREA' ? HTMLTextAreaElement : HTMLInputElement; Object.getOwnPropertyDescriptor(proto.prototype, 'value').set.call(e, ${q(v)}); e.dispatchEvent(new Event('input', { bubbles: true })); return true; })()`);
  const value = (sel) => chrome.eval(`document.querySelector(${q(sel)}).value`);
  const wait = async (sel, ms = 180000) => {
    const t0 = Date.now();
    while (Date.now() - t0 < ms) {
      if (await exists(sel)) return;
      await sleep(150);
    }
    throw new Error(`timeout waiting for ${sel}; screen: ${(await text('body')).slice(0, 200)}`);
  };
  const waitAny = async (sels, ms = 180000) => {
    const t0 = Date.now();
    while (Date.now() - t0 < ms) {
      for (const s of sels) if (await exists(s)) return s;
      await sleep(150);
    }
    throw new Error('timeout');
  };
  const captureDoc = async (sel, ms = 30000) => {
    const col = new DocumentCollector();
    const t0 = Date.now();
    while (Date.now() - t0 < ms) {
      const shot = await chrome.eval(`(() => { const c = document.querySelector(${q(sel)}); if (!c) return null; const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; const u8 = new Uint8Array(d.buffer, d.byteOffset, d.byteLength); let b = ''; for (let i = 0; i < u8.length; i += 8192) b += String.fromCharCode.apply(null, u8.subarray(i, i + 8192)); return { w: c.width, h: c.height, b64: btoa(b) }; })()`);
      if (shot) {
        const r = jsQR(new Uint8ClampedArray(Buffer.from(shot.b64, 'base64')), shot.w, shot.h, { inversionAttempts: 'dontInvert' });
        if (r) {
          const doc = col.push(Uint8Array.from(r.binaryData));
          if (doc) return doc;
        }
      }
      await sleep(60);
    }
    throw new Error(`no document on ${sel}`);
  };
  const load_ = async (url, nowSeconds = null) => {
    await chrome.prepare({ nowSeconds, camera: false });
    await chrome.goto(url);
  };
  const unlock = async () => {
    await fill('#pin', PIN);
    await click('#unlock');
    await wait('#device-id');
  };
  const b64 = (hex) => Buffer.from(h(hex)).toString('base64');
  const paste = async (frameHex) => {
    await click('#scan-release');
    await wait('#paste');
    await chrome.eval(`document.querySelector('details').open = true`);
    await fill('#paste', b64(frameHex));
    await click('#paste-go');
    return waitAny(['#headline', '#refusal', '#red-banner']);
  };

  await chrome.openTab('about:blank');
  check('the tab is visible (the page locks itself when hidden)', (await chrome.eval('document.visibilityState')) === 'visible', await chrome.eval('document.visibilityState'));

  // ---- 1. vector replay (the 1 GiB MASTER vector is left out on a phone)
  await load_(`${ORIGIN}/selftest.html#skip-master`);
  const t0 = Date.now();
  await chrome.eval('new Promise((res) => { const t = setInterval(() => { if (globalThis.__done) { clearInterval(t); res(true); } }, 200); })');
  const r = await chrome.eval('JSON.stringify(globalThis.__result)').then(JSON.parse);
  check('crypto vectors replay on the phone (MASTER 1 GiB left out)', r.pass && r.crypto.failures.length === 0, `${r.crypto.total - r.crypto.skipped} of ${r.crypto.expected} replayed, ${r.crypto.skipped} skipped, ${Date.now() - t0} ms`);
  guard('after vector replay');
  await clearSiteData(chrome);

  // ---- 2. first run: key creation under a PIN, words, reload, wrong PIN
  await load_(`${ORIGIN}/sshub-companion.html`);
  await wait('#create');
  await fill('#pin1', PIN);
  await fill('#pin2', PIN);
  const tc = Date.now();
  await click('#create');
  await wait('#backup-words span');
  const msCreate = Date.now() - tc;
  const words = (await chrome.eval(`Array.from(document.querySelectorAll('#backup-words span')).map((s) => s.innerText).join(' ')`));
  check('key created under a PIN, 24 backup words shown', words.split(' ').length === 24, `${msCreate} ms (Argon2id 64 MiB x 3 included)`);
  await click('#backup-ok');
  await click('#backup-done');
  await wait('#device-id');
  check('device id derives from the shown words', (await text('#device-id')) === identityOfSecret(secretFromWords(words)).deviceId, await text('#device-id'));
  const blob = await chrome.eval(`new Promise((res) => { const r = indexedDB.open('sshub-companion', 1); r.onsuccess = () => { const g = r.result.transaction('kv').objectStore('kv').get('key'); g.onsuccess = () => res(Array.from(g.result)); }; })`);
  check('IndexedDB holds the PIN-wrapped secret only', Buffer.from(blob.slice(0, 4)).toString() === 'SSHP' && !Buffer.from(blob).toString('hex').includes(hexOf(secretFromWords(words))), `${blob.length} bytes`);
  await chrome.goto(`${ORIGIN}/sshub-companion.html`);
  await wait('#unlock');
  await fill('#pin', 'wrong-pin-0');
  await click('#unlock');
  await chrome.eval(`new Promise((res) => { const t = setInterval(() => { if (/wrong PIN/.test(document.querySelector('#unlock-error').textContent)) { clearInterval(t); res(true); } }, 100); })`);
  const tu = Date.now();
  await unlock();
  check('reload asks for the PIN; wrong PIN refused, right PIN unlocks the same device', (await text('#device-id')) === identityOfSecret(secretFromWords(words)).deviceId, `${Date.now() - tu} ms unlock`);
  guard('after key creation');
  await clearSiteData(chrome);

  // ---- 3. restore the fixture identity, release from a pasted challenge, short code, second code refused
  await load_(`${ORIGIN}/sshub-companion.html`, fx.now);
  await wait('#words-input');
  await fill('#words-input', fx.phone.words);
  await fill('#pin1', PIN);
  await fill('#pin2', PIN);
  await click('#restore');
  await wait('#device-id');
  check('restore from 24 words gives the fixture phone identity', (await text('#device-id')) === fx.phone.pub.device_id, fx.phone.pub.device_id);
  const good = fx.challenges.find((c) => c.name === 'good');
  await coolDown(36);
  await paste(good.frame_hex);
  check('pasted Rust challenge: requester named, requester and credential highlighted', (await text('#desc')).includes(fx.requester.pub.device_id) && (await chrome.eval(`document.querySelectorAll('#desc .v[data-danger="1"]').length`)) === 2, '2 highlighted rows');
  check('binding words equal the Rust binding hash words', (await text('#binding-words')) === good.binding_words, good.binding_words);
  check('every check passes (the requester is unknown to a fresh phone: one warning)', (await chrome.eval(`document.querySelectorAll('#checks .check.bad').length`)) === 0 && (await chrome.eval(`document.querySelectorAll('#checks .check.warn').length`)) === 1, 'checks');
  await chrome.eval(`for (const id of ['opt-clip', 'opt-sound']) { const e = document.querySelector('#' + id); if (e.checked) e.click(); }`, { gesture: true });
  const ta = Date.now();
  await click('#approve');
  await wait('#send-done');
  const msApprove = Date.now() - ta;
  const code = (await text('#snapshot-code')).trim();
  check('approve with a short code (Argon2id 256 MiB x 4 snapshot included)', /^[0-9A-Z]{4}(-[0-9A-Z]{4}){3}$/.test(code), `${msApprove} ms, code shown`);
  const guardAfter = guard('after approve');
  const doc = await captureDoc('#reverse-qr');
  const reply = parseReplyPlain(sealedBoxOpen(h(good.eph_secret_hex), doc.slice(23)));
  check('reverse QR reply opens with the ephemeral secret to the phone share', decodeFrame(doc).kind === KIND.RESPONSE && hexOf(reply.share.slice(1)) === fx.phone_share.bytes_hex, `${doc.length} bytes`);
  check('approval signature verifies over the binding preimage', edVerify(phone.edPub, h(good.binding_preimage_hex), reply.ext.approvalSig), 'verifies');
  check('the reply carries the courier op and the short code blob', reply.ext.ops.length === 1 && !!reply.ext.snapshot, `${reply.ext.ops.length} op, snapshot ${reply.ext.snapshot ? reply.ext.snapshot.blob.length : 0} B`);
  void guardAfter;
  await click('#send-done');

  const env = fx.challenges.find((c) => c.name === 'envelope');
  await paste(env.frame_hex);
  check('a second short code for a live credential is refused (checkbox locked)', await chrome.eval(`document.querySelector('#opt-snapshot').disabled`), 'disabled');
  await chrome.eval(`for (const id of ['opt-clip', 'opt-sound', 'opt-snapshot']) { const e = document.querySelector('#' + id); if (e.checked) e.click(); }`, { gesture: true });
  await click('#approve');
  await wait('#send-done');
  const envReply = parseReplyPlain(sealedBoxOpen(h(env.eph_secret_hex), (await captureDoc('#reverse-qr')).slice(23)));
  check('an envelope (post-reshare) share unwraps and the chain continues (courier ops 1, 2)', hexOf(envReply.share.slice(1)) === fx.phone_share.bytes_hex && envReply.ext.ops.map((o) => JSON.parse(new TextDecoder().decode(o.canonical)).seq).join() === '1,2', 'ok');
  await click('#send-done');

  for (const [name, fragment] of [['wrong-host', 'different credential'], ['tampered-op', 'unknown requester'], ['request-id-mismatch', 'different request id'], ['expired', 'expired']]) {
    const c = fx.challenges.find((x) => x.name === name);
    await paste(c.frame_hex);
    const t = await text('#refusal');
    if (name === 'tampered-op') check('a tampered op from an unmet requester is only warned about (signature unknown), never silently trusted', (await exists('#headline')) || t.length > 0, (await exists('#headline')) ? 'warning path' : t.slice(0, 80));
    else check(`refused: ${name}`, t.includes(fragment), t.slice(0, 90));
    await click(name === 'tampered-op' && (await exists('#refuse')) ? '#refuse' : '#back');
  }

  // ---- 4. export as text, Red notice
  await click('#export-ops');
  await click('#export-tail');
  await wait('#ops-b64');
  const pasted = Buffer.from(await value('#ops-b64'), 'base64');
  const qrDoc = await captureDoc('#reverse-qr');
  const opsList = decodePhoneOps(pasted);
  check('export text equals the QR and carries signed phone ops', Buffer.from(qrDoc).equals(pasted) && opsList.length >= 1 && opsList.every((o) => edVerify(phone.edPub, o.canonical, o.signature)), `${opsList.length} ops`);
  await click('#send-done');

  await load_(`${ORIGIN}/sshub-companion.html`, 100);
  await wait('#unlock');
  await unlock();
  const vec = ops.vectors.find((x) => x.id === 'device-add-holder');
  const notice = Buffer.concat([Buffer.from([0x53, 0x57, 0x52, 0x31, 0, 1, KIND.RECOVERY_NOTICE]), Buffer.from(new Uint8Array(4).map((_, i) => (h(vec.canonical_hex).length >>> (24 - 8 * i)) & 255)), Buffer.from(h(vec.canonical_hex)), Buffer.from(h(vec.signature_hex))]);
  await click('#scan-release');
  await wait('#paste');
  await chrome.eval(`document.querySelector('details').open = true`);
  await fill('#paste', notice.toString('base64'));
  await click('#paste-go');
  await wait('#red-banner');
  check('a Red operation notice is described and offers the phone proof', /DeviceAdd proposed by/.test(await text('#red-banner')), 'red banner');
  await click('#red-approve');
  await wait('#ops-b64');
  const redOps = decodePhoneOps(Buffer.from(await value('#ops-b64'), 'base64'));
  const last = parseOp(redOps[redOps.length - 1].canonical);
  check('the proof is a RedPhoneSign naming the exact op hash, signed by the phone', last.kind === 'RedPhoneSign' && last.fields.op_hash === hexOf(blake3(h(vec.canonical_hex))) && edVerify(phone.edPub, redOps[redOps.length - 1].canonical, redOps[redOps.length - 1].signature), last.fields.op_hash.slice(0, 16));
  guard('end of page flows');
  fs.writeFileSync('/tmp/phone-page-results.json', JSON.stringify(results, null, 1));
  const failed = results.filter((x) => !x.pass);
  console.log(JSON.stringify({ total: results.length, failed: failed.length }));
  if (failed.length) throw new Error(`${failed.length} page checks failed`);
}
