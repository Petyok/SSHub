// Drives the real Android Chrome of a USB-connected phone for the PoC D3 bench, the companion
// page checks and the over-the-air benches (PoC C4, H3, H4). Node only (built-in fetch and
// WebSocket), nothing is installed on either device.
//
// SAFETY (enforced in code):
//  * every adb call goes through `adb()`, which accepts only the allow-list below, always with
//    the phone's serial (`PHONE_SERIAL`, never committed: other devices may share the adb server);
//  * Chrome is driven in ONE new tab created here (Target.createTarget); other tabs are never
//    listed, attached, screenshotted or printed;
//  * camera and microphone are granted to http://localhost:8443 only and set back to "prompt";
//  * the origin's site data is cleared at the end, the tab closed, forward and reverse removed;
//  * battery temperature and level are read before and between heavy runs; the script stops above
//    40 C or under 30 percent.
//
// Usage: PHONE_SERIAL=<serial> node phone-device.mjs <recon|d3|page|cleanup> [--runs 5]
import { execFileSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..');
export const ORIGIN = 'http://localhost:8443';
const SERIAL = process.env.PHONE_SERIAL;
if (!SERIAL) throw new Error('set PHONE_SERIAL (adb devices -l)');
export const MAX_TEMP_C = 40;
export const MIN_LEVEL = 30;

// ------------------------------------------------------------ adb allow-list
const ALLOWED = [
  /^shell getprop [A-Za-z0-9_.]+$/,
  /^shell dumpsys battery$/,
  /^shell dumpsys thermalservice$/,
  /^shell dumpsys sensor_privacy$/,
  /^shell dumpsys package com\.android\.chrome$/,
  /^shell dumpsys package com\.brave\.browser$/,
  /^shell cat \/proc\/net\/unix$/,
  /^shell am start -a android\.intent\.action\.VIEW -d http:\/\/localhost:8443\/[A-Za-z0-9_.\/-]*$/,
  /^reverse tcp:8443 tcp:8443$/,
  /^forward tcp:9222 localabstract:[A-Za-z0-9_.]*devtools_remote$/,
  /^reverse --remove tcp:8443$/,
  /^forward --remove tcp:9222$/,
  /^reverse --list$/,
  /^forward --list$/,
];
export function adb(args) {
  const line = args.join(' ');
  if (!ALLOWED.some((r) => r.test(line))) throw new Error(`adb command not allowed: ${line}`);
  return execFileSync('adb', ['-s', SERIAL, ...args], { encoding: 'utf8', maxBuffer: 1 << 24 });
}
// The browser in the foreground (Chrome or Brave): its devtools socket is found read-only in
// /proc/net/unix (only the matching lines are used), never guessed.
export const BROWSER_PKG = process.env.BROWSER_PKG || 'com.android.chrome';
export function devtoolsSocket() {
  const lines = adb(['shell', 'cat', '/proc/net/unix']).split('\n').filter((l) => l.includes('devtools_remote'));
  const names = [...new Set(lines.map((l) => /@([A-Za-z0-9_.]*devtools_remote)/.exec(l)?.[1]).filter(Boolean))];
  const want = names.find((n) => n.startsWith(BROWSER_PKG)) || names.find((n) => n === 'chrome_devtools_remote') || null;
  console.log(JSON.stringify({ devtools_sockets: names, using: want }));
  if (!want) throw new Error(`no devtools socket for ${BROWSER_PKG}; the owner must enable remote debugging in the browser`);
  return want;
}
// One diagnostic screen capture to a temp file (the owner allows it at diagnostic steps only; the
// caller views it and deletes it at once, it is never copied or committed).
export function screencap(file) {
  const png = execFileSync('adb', ['-s', SERIAL, 'exec-out', 'screencap', '-p'], { maxBuffer: 1 << 26 });
  fs.writeFileSync(file, png, { mode: 0o600 });
  return file;
}
export const adbDevices = () => execFileSync('adb', ['devices', '-l'], { encoding: 'utf8' }); // read-only, no serial filter needed

// ------------------------------------------------------------ battery guard
export function battery() {
  const out = adb(['shell', 'dumpsys', 'battery']);
  const level = Number(/^\s{2}level: (\d+)/m.exec(out)?.[1]);
  const temp = Number(/^\s{2}temperature: (\d+)/m.exec(out)?.[1]) / 10;
  return { level, tempC: temp };
}
export function guard(label) {
  const b = battery();
  console.log(JSON.stringify({ battery: label, level: b.level, temp_c: b.tempC }));
  if (b.tempC > MAX_TEMP_C) throw new Error(`STOP: battery ${b.tempC} C is above ${MAX_TEMP_C} C (${label})`);
  if (b.level < MIN_LEVEL) throw new Error(`STOP: battery ${b.level}% is under ${MIN_LEVEL}% (${label})`);
  return b;
}
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
export async function coolDown(target = 36, maxWaitMs = 180000) {
  const t0 = Date.now();
  for (;;) {
    const b = guard('cool-down');
    if (b.tempC <= target || Date.now() - t0 > maxWaitMs) return b;
    await sleep(15000);
  }
}

// ------------------------------------------------------------ local web server for the pages
export function assembleServeDir() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'phone-serve-'));
  const put = (from, to) => {
    fs.mkdirSync(path.dirname(path.join(dir, to)), { recursive: true });
    fs.copyFileSync(from, path.join(dir, to));
  };
  put(path.join(root, 'sshub-companion.html'), 'sshub-companion.html');
  const selftest = path.join(root, 'build', 'selftest.html');
  if (fs.existsSync(selftest)) put(selftest, 'selftest.html');
  fs.writeFileSync(path.join(dir, 'blank.html'), '<!doctype html><title>bench</title>');
  fs.writeFileSync(path.join(dir, 'gum.html'), '<!doctype html><title>gum</title><button id="b">camera</button><pre id="o">idle</pre><script>document.getElementById("b").onclick=async()=>{document.getElementById("o").textContent="asking";try{const s=await navigator.mediaDevices.getUserMedia({video:true});document.getElementById("o").textContent="ok "+s.getVideoTracks()[0].label;s.getTracks().forEach(t=>t.stop())}catch(e){document.getElementById("o").textContent="error "+e.name+" "+e.message}}</script>');
  const poc = path.join(root, 'poc');
  for (const f of ['qr-scanner.html', 'share-transmit.html', 'share-receive.html', 'reverse-qr-receive.html', 'share-lib.js']) put(path.join(poc, f), f);
  for (const f of ['jsQR.js', 'qrcode.js']) put(path.join(poc, 'vendor', f), `vendor/${f}`);
  const bench = process.env.D3_BENCH_DIR;
  if (bench) for (const f of ['measure.html', 'hash-wasm-argon2.umd.min.js']) put(path.join(bench, f), f);
  return dir;
}
export async function startServer(dir) {
  const p = spawn('python3', ['-m', 'http.server', '--bind', '127.0.0.1', '8443', '-d', dir], { stdio: 'ignore' });
  for (let i = 0; i < 50; i++) {
    try {
      await fetch('http://127.0.0.1:8443/');
      return p;
    } catch {
      await sleep(100);
    }
  }
  p.kill();
  throw new Error('http.server did not start');
}

// ------------------------------------------------------------ CDP: one tab of our own
export class Chrome {
  constructor() {
    this.id = 0;
    this.pending = new Map();
    this.sessionId = null;
    this.targetId = null;
    this.events = [];
  }
  async connect() {
    const v = await (await fetch('http://127.0.0.1:9222/json/version')).json();
    this.version = v;
    this.ws = new WebSocket(v.webSocketDebuggerUrl);
    await new Promise((res, rej) => {
      this.ws.onopen = res;
      this.ws.onerror = rej;
    });
    this.ws.onmessage = (e) => {
      const m = JSON.parse(e.data);
      if (m.id && this.pending.has(m.id)) {
        this.pending.get(m.id)(m);
        this.pending.delete(m.id);
      } else if (m.method && m.sessionId === this.sessionId) this.events.push(m);
    };
    return v.Browser;
  }
  send(method, params = {}, session = true) {
    return new Promise((resolve, reject) => {
      const id = ++this.id;
      this.pending.set(id, (m) => (m.error ? reject(new Error(`${method}: ${m.error.message}`)) : resolve(m.result)));
      const msg = { id, method, params };
      if (session && this.sessionId) msg.sessionId = this.sessionId;
      this.ws.send(JSON.stringify(msg));
    });
  }
  // A tab of our own. Target.createTarget works when Chrome has a window; when it answers "Could not
  // find TabModelList" (no window yet) the allowed `am start` intent opens http://localhost:8443/blank.html
  // and the tab is found by that URL (the /json list is filtered to our own origin, nothing else is read).
  async openTab(url = 'about:blank') {
    let targetId;
    try {
      ({ targetId } = await this.send('Target.createTarget', { url }, false));
    } catch (e) {
      if (!/TabModelList/.test(e.message)) throw e;
      adb(['shell', 'am', 'start', '-a', 'android.intent.action.VIEW', '-d', `${ORIGIN}/blank.html`]);
      for (let i = 0; i < 100 && !targetId; i++) {
        const list = await (await fetch('http://127.0.0.1:9222/json/list')).json();
        targetId = list.find((t) => t.type === 'page' && t.url.startsWith(`${ORIGIN}/blank.html`))?.id;
        if (!targetId) await sleep(200);
      }
      if (!targetId) throw new Error('our tab did not appear after the VIEW intent');
      this.viaIntent = true;
    }
    this.targetId = targetId;
    const { sessionId } = await this.send('Target.attachToTarget', { targetId, flatten: true }, false);
    this.sessionId = sessionId;
    await this.send('Page.enable');
    await this.send('Runtime.enable');
    return targetId;
  }
  // Every evaluation has a deadline: a page that waits on a permission prompt must not hang the run.
  // The owner opens OUR page in a tab himself (a real tap is needed for the camera prompt); the tab is
  // found by its URL (filtered to our own origin, no other tab is read) and driven like our own.
  async adoptTab(prefix, waitS = 300) {
    for (let i = 0; i < waitS * 2; i++) {
      const list = await (await fetch('http://127.0.0.1:9222/json/list')).json();
      const t = list.find((x) => x.type === 'page' && x.url.startsWith(prefix));
      if (t) {
        this.targetId = t.id;
        this.sessionId = (await this.send('Target.attachToTarget', { targetId: t.id, flatten: true }, false)).sessionId;
        await this.send('Page.enable');
        await this.send('Runtime.enable');
        this.viaIntent = true;
        return t.id;
      }
      await sleep(500);
    }
    throw new Error(`no tab of ours appeared (${prefix})`);
  }
  async eval(expression, { await: aw = true, gesture = false, timeoutMs = 30000 } = {}) {
    const call = this.send('Runtime.evaluate', { expression, awaitPromise: aw, returnByValue: true, userGesture: gesture });
    let timer;
    const limit = new Promise((_, rej) => {
      timer = setTimeout(() => rej(new Error(`page evaluation timed out after ${timeoutMs} ms`)), timeoutMs);
    });
    const r = await Promise.race([call, limit]).finally(() => clearTimeout(timer));
    if (r.exceptionDetails) throw new Error(`page exception: ${r.exceptionDetails.exception?.description || r.exceptionDetails.text}`);
    return r.result.value;
  }
  // Date shifted so that `now` seconds is "now" in the page, camera and audio neutralised unless allowed.
  async prepare({ nowSeconds = null, camera = false, silent = true } = {}) {
    if (this.scriptId) await this.send('Page.removeScriptToEvaluateOnNewDocument', { identifier: this.scriptId });
    const off = nowSeconds === null ? 0 : nowSeconds * 1000 - Date.now();
    const src = `(() => {
      const D = Date; const off = ${off};
      if (off) { globalThis.Date = class extends D { constructor(...a) { a.length ? super(...a) : super(D.now() + off); } static now() { return D.now() + off; } }; }
      ${camera ? '' : "navigator.mediaDevices.getUserMedia = () => Promise.reject(new DOMException('blocked by the test driver', 'NotAllowedError'));"}
      ${silent ? "globalThis.AudioContext = undefined; globalThis.webkitAudioContext = undefined; try { navigator.clipboard.writeText = () => Promise.reject(new Error('blocked by the test driver')); } catch (e) {}" : ''}
    })();`;
    this.scriptId = (await this.send('Page.addScriptToEvaluateOnNewDocument', { source: src })).identifier;
  }
  async goto(url) {
    await this.send('Page.navigate', { url });
    for (let i = 0; i < 100; i++) {
      if ((await this.eval('document.readyState')) === 'complete') return;
      await sleep(100);
    }
    throw new Error(`page did not load: ${url}`);
  }
  async grant(perms) {
    await this.send('Browser.grantPermissions', { origin: ORIGIN, permissions: perms }, false);
    this.granted = [...new Set([...(this.granted || []), ...perms])];
  }
  async close() {
    if (this.targetId) await this.send('Target.closeTarget', { targetId: this.targetId }, false).catch(() => {});
    this.targetId = null;
  }
}
const PERM_NAMES = { videoCapture: 'camera', audioCapture: 'microphone' };
export async function revokeAll(chrome) {
  for (const p of chrome.granted || []) {
    await chrome.send('Browser.setPermission', { permission: { name: PERM_NAMES[p] || p }, setting: 'prompt', origin: ORIGIN }, false).catch((e) => console.log(`revoke ${p}: ${e.message}`));
  }
  chrome.granted = [];
}
// Sent through our own tab's session (the browser-level endpoint answers with an internal error).
export const clearSiteData = (chrome) => chrome.send('Storage.clearDataForOrigin', { origin: ORIGIN, storageTypes: 'all' }, !!chrome.sessionId);

// ------------------------------------------------------------ cleanup and proof (always runs)
export async function cleanup({ chrome, server, dir }) {
  const proof = {};
  try {
    if (chrome && chrome.ws && chrome.ws.readyState === 1) {
      await revokeAll(chrome);
      if (chrome.sessionId) {
        await clearSiteData(chrome);
        // Proof: a directory listing of our own server shares the origin and creates no storage.
        try {
          await chrome.goto(`${ORIGIN}/`);
          proof.site_storage_after_clear = await chrome.eval(`(async () => JSON.stringify({ indexedDB: (await indexedDB.databases()).length, localStorage: localStorage.length, caches: (await caches.keys()).length }))()`);
        } catch (e) {
          proof.site_storage_after_clear = `not checked: ${e.message}`;
        }
      }
      await chrome.close();
      chrome.sessionId = null;
      chrome.ws.close();
    }
  } catch (e) {
    proof.chrome_error = e.message;
  }
  try { adb(['reverse', '--remove', 'tcp:8443']); } catch (e) { proof.reverse_remove = e.message.split('\n')[0]; }
  try { adb(['forward', '--remove', 'tcp:9222']); } catch (e) { proof.forward_remove = e.message.split('\n')[0]; }
  if (server) server.kill();
  if (dir) fs.rmSync(dir, { recursive: true, force: true });
  proof.reverse_list = adb(['reverse', '--list']).trim() || '(empty)';
  proof.forward_list = adb(['forward', '--list']).split('\n').filter((l) => l.includes(SERIAL)).join('\n') || '(none for this phone)';
  proof.server_stopped = await fetch('http://127.0.0.1:8443/').then(() => false, () => true);
  proof.serve_dir_removed = dir ? !fs.existsSync(dir) : true;
  console.log(`CLEANUP ${JSON.stringify(proof)}`);
  return proof;
}

// Everything setup() creates is recorded in `state` at once, so cleanup() reaches it even when
// a later step throws.
export const state = {};
export async function setup() {
  // A killed or interrupted run still cleans up (tab, permissions, site data, forward, reverse, server).
  for (const sig of ['SIGINT', 'SIGTERM', 'SIGHUP']) process.once(sig, async () => { await cleanup(state).catch(() => {}); process.exit(130); });
  state.dir = assembleServeDir();
  state.server = await startServer(state.dir);
  adb(['reverse', 'tcp:8443', 'tcp:8443']);
  adb(['forward', 'tcp:9222', `localabstract:${devtoolsSocket()}`]);
  state.chrome = new Chrome();
  state.browser = await state.chrome.connect();
  const pkg = state.chrome.version['Android-Package'];
  console.log(JSON.stringify({ devtools_owner: pkg || '(not reported)' }));
  if (pkg && pkg !== BROWSER_PKG) throw new Error(`the devtools socket belongs to ${pkg}, not ${BROWSER_PKG}`);
  return state;
}

// ------------------------------------------------------------ subcommands
const cmd = process.argv[2];
const runs = process.argv.includes('--runs') ? Number(process.argv[process.argv.indexOf('--runs') + 1]) : 5;
const median = (a) => [...a].sort((x, y) => x - y)[Math.floor(a.length / 2)];

export function recon() {
  const prop = (k) => adb(['shell', 'getprop', k]).trim();
  const out = {
    model: prop('ro.product.model'),
    manufacturer: prop('ro.product.manufacturer'),
    android: prop('ro.build.version.release'),
    sdk: prop('ro.build.version.sdk'),
    browser: BROWSER_PKG,
    browser_version: /versionName=(\S+)/.exec(adb(['shell', 'dumpsys', 'package', BROWSER_PKG]))?.[1],
    battery: battery(),
  };
  console.log(JSON.stringify(out));
  return out;
}

async function d3(ctx) {
  const { chrome } = ctx;
  await chrome.openTab('about:blank');
  await chrome.prepare({ camera: false });
  await chrome.goto(`${ORIGIN}/measure.html`);
  const info = await chrome.eval('JSON.stringify({ua: navigator.userAgent, secure: isSecureContext, visible: document.visibilityState})');
  console.log(info);
  const results = {};
  for (const mem of [64, 256, 512]) {
    const n = mem === 512 ? 1 : runs;
    results[mem] = [];
    for (let i = 0; i < n; i++) {
      const before = await coolDown(mem === 64 ? 38 : 36);
      const r = JSON.parse(await chrome.eval(`window.runProfile(${mem}).then(JSON.stringify)`));
      const after = guard(`after ${mem} MiB run ${i + 1}`);
      results[mem].push({ run: i + 1, ms: Math.round(r.ms), outcome: r.outcome, temp_before_c: before.tempC, temp_after_c: after.tempC, level: after.level, digest: r.out ? r.out.slice(0, 8) : r.error });
      console.log(JSON.stringify({ mem, ...results[mem][i] }));
    }
    console.log(JSON.stringify({ mem, median_ms: median(results[mem].map((x) => x.ms)), worst_ms: Math.max(...results[mem].map((x) => x.ms)) }));
  }
  return results;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  let code = 0;
  try {
    if (cmd === 'recon') recon();
    else if (cmd === 'cleanup') console.log('cleanup only');
    else {
      recon();
      guard('start');
      const ctx = await setup();
      console.log(`chrome ${ctx.browser}`);
      if (cmd === 'd3') await d3(ctx);
      else if (cmd === 'page') {
        const { runPage } = await import('./phone-page-flows.mjs');
        // a watchdog so that a stuck page still ends in the cleanup below
        await Promise.race([runPage({ ...ctx, ORIGIN, clearSiteData, coolDown, guard, sleep }), sleep(20 * 60 * 1000).then(() => { throw new Error('page flows timed out after 20 minutes'); })]);
      } else throw new Error(`unknown command ${cmd}`);
    }
  } catch (e) {
    console.log(`FAILED: ${e.stack || e.message}`);
    code = 1;
  } finally {
    if (cmd !== 'recon') await cleanup(state);
  }
  process.exit(code);
}
void createRequire;
