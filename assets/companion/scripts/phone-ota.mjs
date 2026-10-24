// Over-the-air benches with a real phone and the laptop facing each other (same safety rules and
// helpers as phone-device.mjs).
//
//   node phone-ota.mjs camera   C4: the laptop shows animated QR codes in ONE terminal window (kitty,
//                               own class and title, closed after each block), the phone page
//                               qr-scanner.html reads them with its rear camera.
//   node phone-ota.mjs audio    H3/H4: the phone page share-transmit.html plays FSK bursts (fast, slow)
//                               and shows a QR; the laptop receives with share-receive.html in a
//                               THROWAWAY Chromium profile (real microphone and webcam, permission
//                               prompt auto-accepted, never the owner's profile).
//
// Env: PHONE_SERIAL (adb serial, not committed), PLAYER (terminal-player.mjs path, default the poc one).
// One block at a time (the laptop has four weak threads); every block ends by closing the window or
// browser it opened and deleting the temporary profile.
import { execFileSync, spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import vm from 'node:vm';
import zlib from 'node:zlib';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { BROWSER_PKG, ORIGIN, adb, cleanup, guard, screencap, setup, sleep, state } from './phone-device.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const cmd = process.argv[2];
const arg = (k, d) => {
  const i = process.argv.indexOf(`--${k}`);
  return i < 0 ? d : process.argv[i + 1];
};
const PLAYER = process.env.PLAYER || path.resolve(here, '..', 'poc', 'terminal-player.mjs');
const median = (a) => (a.length ? [...a].sort((x, y) => x - y)[Math.floor(a.length / 2)] : null);
const out = [];
const record = (row) => {
  out.push(row);
  console.log(JSON.stringify(row));
};

const q = JSON.stringify;
const phoneEval = (expr, opts) => state.chrome.eval(expr, opts);
const phoneClick = (sel) => phoneEval(`(() => { const e = document.querySelector(${q(sel)}); if (!e) throw new Error('missing ${sel}'); e.click(); return true; })()`, { gesture: true });
const stop = (child) =>
  new Promise((res) => {
    if (!child || child.exitCode !== null) return res();
    child.once('exit', res);
    child.kill('SIGTERM');
    setTimeout(() => child.exitCode === null && child.kill('SIGKILL'), 3000);
    setTimeout(res, 5000);
  });


// ------------------------------------------------------------ sender preflight (laptop side)
const require = createRequire(import.meta.url);
const jsQR = require('../vendor/jsqr-1.4.0/jsQR.cjs');
function pngToRgba(buf) {
  let at = 8;
  let w = 0, h = 0, ct = 0;
  const idat = [];
  while (at < buf.length) {
    const len = buf.readUInt32BE(at);
    const type = buf.toString('latin1', at + 4, at + 8);
    const body = buf.subarray(at + 8, at + 8 + len);
    if (type === 'IHDR') {
      w = body.readUInt32BE(0);
      h = body.readUInt32BE(4);
      ct = body[9];
      if (body[8] !== 8 || body[12] !== 0 || (ct !== 2 && ct !== 6)) throw new Error('unsupported PNG layout');
    } else if (type === 'IDAT') idat.push(body);
    at += 12 + len;
  }
  const bpp = ct === 6 ? 4 : 3;
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const stride = w * bpp;
  const out = new Uint8ClampedArray(w * h * 4);
  let prev = Buffer.alloc(stride);
  for (let y = 0; y < h; y++) {
    const f = raw[y * (stride + 1)];
    const line = Buffer.from(raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1)));
    for (let i = 0; i < stride; i++) {
      const a = i >= bpp ? line[i - bpp] : 0;
      const b = prev[i];
      const c = i >= bpp ? prev[i - bpp] : 0;
      let add = 0;
      if (f === 1) add = a;
      else if (f === 2) add = b;
      else if (f === 3) add = (a + b) >> 1;
      else if (f === 4) {
        const p = a + b - c;
        const pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
        add = pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      line[i] = (line[i] + add) & 255;
    }
    for (let x = 0; x < w; x++) {
      out[(y * w + x) * 4] = line[x * bpp];
      out[(y * w + x) * 4 + 1] = line[x * bpp + 1];
      out[(y * w + x) * 4 + 2] = line[x * bpp + 2];
      out[(y * w + x) * 4 + 3] = 255;
    }
    prev = line;
  }
  return { data: out, w, h };
}
function shareLib() {
  const ctx = { console };
  ctx.window = ctx;
  vm.createContext(ctx);
  for (const f of ['vendor/qrcode.js', 'share-lib.js']) vm.runInContext(fs.readFileSync(path.resolve(here, '..', 'poc', f), 'utf8'), ctx, { filename: f });
  return ctx.ShareLib;
}
// Crops ONLY our own window (geometry of its class from hyprctl), decodes the QR in it and requires
// a valid chunk (CRC) of the expected document. The capture is deleted at once.
function senderPreflight(cls, report) {
  let rep = null;
  try {
    rep = JSON.parse(fs.readFileSync(report, 'utf8'));
  } catch {
    throw new Error('SENDER PREFLIGHT FAILED: the player wrote no size report (the window did not start)');
  }
  console.log(JSON.stringify({ sender_window_cells: `${rep.columns}x${rep.rows}`, qr_needs_cells: `${rep.need_width}x${rep.need_lines}`, fits: rep.fits }));
  if (!rep.fits) throw new Error(`SENDER PREFLIGHT FAILED: the QR needs ${rep.need_width}x${rep.need_lines} cells but the window has ${rep.columns}x${rep.rows}`);
  const mine = JSON.parse(execFileSync('hyprctl', ['clients', '-j'], { encoding: 'utf8' })).find((c) => c.class === cls);
  if (!mine) throw new Error('SENDER PREFLIGHT FAILED: our window is not on the screen');
  const png = '/tmp/c4-sender-check.png';
  try {
    execFileSync('grim', ['-g', `${mine.at[0]},${mine.at[1]} ${mine.size[0]}x${mine.size[1]}`, png]);
    const img = pngToRgba(fs.readFileSync(png));
    const r = jsQR(img.data, img.w, img.h, { inversionAttempts: 'attemptBoth' });
    if (!r) throw new Error('SENDER PREFLIGHT FAILED: the QR in the window does not decode');
    const L = shareLib();
    const st = new L.Reassembler().add(Uint8Array.from(r.binaryData));
    if (!(st.have >= 1 && st.total > 0)) throw new Error('SENDER PREFLIGHT FAILED: the decoded chunk is not valid (CRC)');
    console.log(JSON.stringify({ sender_preflight: 'window QR decodes', chunks_total: st.total, window_px: `${mine.size[0]}x${mine.size[1]}` }));
  } finally {
    fs.rmSync(png, { force: true });
  }
}

// Our own kitty window, made floating and sized to fit the QR (a tiled window shares the workspace and
// is too small). hyprctl is addressed to OUR window only. The player starts 2 s after the window has
// its final size, so it measures the real tty.
async function benchWindow(cls, cols, rows, font, argv) {
  const win = spawn('kitty', [
    '--class', cls, '--title', cls,
    '-o', 'remember_window_size=no', '-o', `initial_window_width=${cols}c`, '-o', `initial_window_height=${rows}c`,
    '-o', 'confirm_os_window_close=0', '-o', 'background_opacity=1', ...(font ? ['-o', `font_size=${font}`] : []),
    'sh', '-c', 'sleep 2; exec "$@"', 'sh', ...argv,
  ], { stdio: 'ignore' });
  for (let i = 0; i < 20; i++) {
    await sleep(100);
    const mine = JSON.parse(execFileSync('hyprctl', ['clients', '-j'], { encoding: 'utf8' })).find((c) => c.class === cls);
    if (!mine) continue;
    const a = `address:${mine.address}`;
    const mon = JSON.parse(execFileSync('hyprctl', ['monitors', '-j'], { encoding: 'utf8' }))[0];
    const w = Math.min(mon.width - 40, 800), h = Math.min(mon.height - 60, 870);
    const dsp = (e) => execFileSync('hyprctl', ['dispatch', `hl.dsp.window.${e}`], { encoding: 'utf8' });
    dsp(`float({ action = "set", window = "${a}" })`);
    dsp(`resize({ x = ${w}, y = ${h}, window = "${a}" })`);
    dsp(`center({ window = "${a}" })`);
    break;
  }
  return win;
}
// Exactly one camera request per block: release the page's stream before it is replaced or left.
const stopCamera = () => phoneEval(`(() => { const v = document.querySelector('#cam'); if (v && v.srcObject) { v.srcObject.getTracks().forEach((t) => t.stop()); v.srcObject = null; } return true; })()`, { timeoutMs: 5000 }).catch(() => {});

// ------------------------------------------------------------ C4: terminal to phone camera
// Hard preflight: never start a timed block on an input that is not live. The camera must deliver a
// live video track with a picture within 10 s and the scanner must decode a first frame of the
// terminal's QR within 10 s more; otherwise the block aborts with the exact reason.
async function preflightCamera({ decode = true } = {}) {
  const status = () => phoneEval(`JSON.stringify((() => { const v = document.querySelector('#cam'); const t = v && v.srcObject ? v.srcObject.getVideoTracks()[0] : null; return { stream: !!(v && v.srcObject), w: v && v.videoWidth, track: t && t.readyState, label: t && t.label, have: document.querySelector('#have').innerText, total: document.querySelector('#total').innerText }; })())`, { timeoutMs: 8000 }).then(JSON.parse);
  const perm = await phoneEval(`navigator.permissions.query({ name: 'camera' }).then((p) => p.state)`, { timeoutMs: 5000 }).catch(() => 'unknown');
  await phoneClick('#start');
  let st = null;
  const holdS = Number(arg('hold', 10));
  for (let i = 0; i < holdS * 2; i++) {
    st = await status();
    if (st.track === 'live' && st.w > 0) break;
    await sleep(500);
  }
  if (!(st.track === 'live' && st.w > 0)) throw new Error(`PREFLIGHT FAILED: the camera is not live after ${holdS} s (permission state ${perm}, stream ${st.stream}, track ${st.track}); Chrome probably lacks the Android camera permission`);
  console.log(JSON.stringify({ preflight: 'camera live', track: st.label, width: st.w }));
  if (!decode) return;
  for (let i = 0; i < 20; i++) {
    st = await status();
    if (st.total !== '?' || Number(st.have) > 0) break;
    await sleep(500);
  }
  if (st.total === '?' && !(Number(st.have) > 0)) throw new Error('PREFLIGHT FAILED: the camera is live but decoded no QR frame in 10 s (aim, distance, focus or screen brightness)');
  console.log(JSON.stringify({ preflight: 'first QR frame decoded', have: st.have, total: st.total }));
}

async function terminalBlock({ label, version, bytes, cols, rows, runs, font }) {
  const cls = 'sshub-bench-c4';
  const report = `/tmp/c4-player-report-${process.pid}.json`;
  const win = await benchWindow(cls, cols, rows, font, [process.execPath, PLAYER, '--version', String(version), '--bytes', String(bytes), '--seed', '1', '--fps', '8', '--report', report]);
  try {
    await sleep(4500);
    senderPreflight(cls, report);
    await state.chrome.prepare({ camera: true, silent: true });
    await stopCamera();
    await state.chrome.goto(`${ORIGIN}/qr-scanner.html?seed=1&len=${bytes}&facing=${arg('facing', 'environment')}`);
    await phoneEval(`navigator.wakeLock && navigator.wakeLock.request('screen').then((l) => { window.__wl = l; }, () => {})`);
    await preflightCamera();
    const rowsOut = [];
    for (let run = 1; run <= runs; run++) {
      const b = guard(`${label} run ${run}`);
      await phoneClick('#reset');
      const t0 = Date.now();
      let res = null;
      while (Date.now() - t0 < 30000) {
        const t = await phoneEval(`document.querySelector('#result').innerText`);
        if (/complete in/.test(t)) {
          res = t;
          break;
        }
        await sleep(100);
      }
      const wall = (Date.now() - t0) / 1000;
      const m = res && /complete in ([\d.]+) s.*?(MATCH|MISMATCH)/.exec(res);
      const st = await phoneEval(`JSON.stringify({have: document.querySelector('#have').innerText, total: document.querySelector('#total').innerText, bad: document.querySelector('#bad').innerText, dupes: document.querySelector('#dupes').innerText})`).then(JSON.parse);
      const row = { block: label, run, ok: !!m && m[2] === 'MATCH', elapsed_s: m ? Number(m[1]) : null, wall_s: Number(wall.toFixed(2)), ...st, temp_c: b.tempC };
      rowsOut.push(row);
      record(row);
      await sleep(2000);
    }
    const okRows = rowsOut.filter((r) => r.ok);
    record({ block: label, summary: true, runs, ok: okRows.length, worst_elapsed_s: okRows.length ? Math.max(...okRows.map((r) => r.elapsed_s)) : null, median_elapsed_s: median(okRows.map((r) => r.elapsed_s)), worst_wall_s: Math.max(...rowsOut.map((r) => r.wall_s)) });
  } finally {
    await stopCamera();
    await stop(win);
    fs.rmSync(report, { force: true });
  }
}

async function camera() {
  if (process.argv.includes('--adopt')) {
    console.log(`WAITING: open ${ORIGIN}/qr-scanner.html?seed=1&len=1714 in Chrome on the phone, tap Start camera, tap Allow`);
    await state.chrome.adoptTab(`${ORIGIN}/qr-scanner.html`, 600);
    await state.chrome.send('Page.bringToFront').catch(() => {});
    console.log('adopted the tab the owner opened');
  } else {
    await state.chrome.openTab('about:blank');
    await state.chrome.grant(['videoCapture']);
  }
  const runs = Number(arg('runs', 5));
  const blocks = [
    { label: 'v5 80x24 1714 B', version: 5, bytes: 1714, cols: 80, rows: 24 },
    { label: 'v5 80x24 4096 B', version: 5, bytes: 4096, cols: 80, rows: 24 },
    { label: 'v13 120x40 1714 B', version: 13, bytes: 1714, cols: 120, rows: 40, font: arg('font13', '11') },
    { label: 'v13 120x40 4096 B', version: 13, bytes: 4096, cols: 120, rows: 40, font: arg('font13', '11') },
  ].filter((b) => !arg('only') || b.label.startsWith(arg('only')));
  for (const b of blocks) {
    await terminalBlock({ ...b, runs });
    await sleep(8000);
  }
}

// ------------------------------------------------------------ H3 and H4: phone to laptop
async function laptopBrowser() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ota-profile-'));
  const port = 9333;
  const proc = spawn('chromium', [
    `--user-data-dir=${dir}`, `--remote-debugging-port=${port}`, '--no-first-run', '--no-default-browser-check',
    '--use-fake-ui-for-media-stream', '--autoplay-policy=no-user-gesture-required', '--disable-extensions',
    '--class=sshub-bench-rx', '--window-size=1200,760',
    'about:blank',
  ], { stdio: 'ignore' });
  let version = null;
  for (let i = 0; i < 100 && !version; i++) {
    try {
      version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
    } catch {
      await sleep(200);
    }
  }
  if (!version) {
    await stop(proc);
    fs.rmSync(dir, { recursive: true, force: true });
    throw new Error('laptop chromium did not start');
  }
  const ws = new WebSocket(version.webSocketDebuggerUrl);
  await new Promise((res, rej) => {
    ws.onopen = res;
    ws.onerror = rej;
  });
  let id = 0;
  const pend = new Map();
  let sessionId = null;
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data);
    if (m.id && pend.has(m.id)) (pend.get(m.id)(m), pend.delete(m.id));
  };
  const send = (method, params = {}, session = true) => new Promise((resolve, reject) => {
    const i = ++id;
    pend.set(i, (m) => (m.error ? reject(new Error(`${method}: ${m.error.message}`)) : resolve(m.result)));
    ws.send(JSON.stringify({ id: i, method, params, ...(session && sessionId ? { sessionId } : {}) }));
  });
  const { targetId } = await send('Target.createTarget', { url: 'about:blank' }, false);
  await send('Target.activateTarget', { targetId }, false).catch(() => {});
  // Float and centre OUR window (found by its own class) so the live preview is large and visible.
  for (let i = 0; i < 30; i++) {
    const mine = JSON.parse(execFileSync('hyprctl', ['clients', '-j'], { encoding: 'utf8' })).find((c) => c.class === 'sshub-bench-rx');
    if (!mine) { await sleep(200); continue; }
    const a = `address:${mine.address}`;
    const dsp = (e) => execFileSync('hyprctl', ['dispatch', `hl.dsp.window.${e}`], { encoding: 'utf8' });
    dsp(`float({ action = "set", window = "${a}" })`);
    dsp(`resize({ x = 1200, y = 760, window = "${a}" })`);
    dsp(`center({ window = "${a}" })`);
    break;
  }
  sessionId = (await send('Target.attachToTarget', { targetId, flatten: true }, false)).sessionId;
  await send('Runtime.enable');
  await send('Page.enable');
  const ev = async (expression, gesture = false) => {
    const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true, userGesture: gesture });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
    return r.result.value;
  };
  return {
    ev,
    version: version.Browser,
    async goto(url) {
      await send('Page.navigate', { url });
      for (let i = 0; i < 100 && (await ev('document.readyState')) !== 'complete'; i++) await sleep(100);
    },
    async close() {
      try { ws.close(); } catch { /* already closed */ }
      await stop(proc);
      fs.rmSync(dir, { recursive: true, force: true });
    },
  };
}

async function audioBlock({ label, profile, len, runs }) {
  const laptop = await laptopBrowser();
  try {
    await state.chrome.prepare({ camera: false, silent: false });
    await state.chrome.goto(`${ORIGIN}/share-transmit.html?seed=1&len=${len}&version=10`);
    await phoneEval(`navigator.wakeLock && navigator.wakeLock.request('screen').then((l) => { window.__wl = l; }, () => {})`);
    await phoneEval(`document.querySelector('#profile').value = ${q(profile)}`);
    await laptop.goto(`http://127.0.0.1:8443/share-receive.html`);
    await laptop.ev(`(() => { document.querySelector('#seed').value = 1; document.querySelector('#len').value = ${len}; document.querySelector('#profile').value = ${q(profile)}; document.querySelector('#useqr').checked = false; return true; })()`);
    await laptop.ev(`document.querySelector('#arm').click()`, true);
    for (let i = 0; i < 50 && (await laptop.ev(`document.querySelector('#status').textContent`)) !== 'armed'; i++) await sleep(200);
    if ((await laptop.ev(`document.querySelector('#status').textContent`)) !== 'armed') throw new Error('laptop microphone did not arm');
    const rows = [];
    for (let trial = 1; trial <= runs; trial++) {
      const b = guard(`${label} trial ${trial}`);
      const before = await laptop.ev(`document.querySelectorAll('#rows tr').length`);
      const tClick = Date.now();
      await laptop.ev(`document.querySelector('#trial').click()`, true);
      await sleep(600);
      await phoneClick('#send');
      const tSend = Date.now();
      const burst = Number(/audio sent: ([\d.]+) s/.exec(await phoneEval(`document.querySelector('#status').textContent`))?.[1] || 8);
      let row = null;
      for (let i = 0; i < 160; i++) {
        const n = await laptop.ev(`document.querySelectorAll('#rows tr').length`);
        if (n > before) {
          row = await laptop.ev(`Array.from(document.querySelector('#rows tr:last-child').cells).map((c) => c.textContent)`);
          break;
        }
        await sleep(100);
      }
      const ok = row && row[1] === 'true';
      const audioMs = row && row[2] !== '-' ? Number(row[2]) : null;
      const latency = ok ? audioMs - (tSend - tClick) : null;
      const r = { block: label, trial, ok, latency_ms_from_send: latency, burst_s: burst, temp_c: b.tempC };
      rows.push(r);
      record(r);
      await sleep(Math.max(1500, burst * 1000 + 1500 - (Date.now() - tSend)));
      if (trial === 3 && rows.every((x) => !x.ok)) {
        record({ block: label, summary: true, aborted: 'the first three trials heard nothing: media volume of the phone or the laptop microphone', trials: 3, ok: 0 });
        return 0;
      }
    }
    const okRows = rows.filter((r) => r.ok);
    record({ block: label, summary: true, trials: runs, ok: okRows.length, median_latency_ms: median(okRows.map((r) => r.latency_ms_from_send)), burst_s: rows[0]?.burst_s });
    return okRows.length;
  } finally {
    await laptop.close();
  }
}
// One diagnostic look at the phone's reverse-QR page: numbers from the page plus ONE screen capture
// (temp file, viewed by the director and deleted at once).
async function diag() {
  await state.chrome.openTab('about:blank');
  await state.chrome.prepare({ camera: false, silent: true });
  await state.chrome.goto(`${ORIGIN}/share-transmit.html?${arg('query', 'seed=1&len=56&version=10')}`);
  await phoneClick('#qr');
  await sleep(800);
  console.log(await phoneEval(`JSON.stringify((() => { const c = document.querySelector('#qrc'); const r = c.getBoundingClientRect(); return { dpr: devicePixelRatio, innerWidth, innerHeight, canvas_css_w: r.width, canvas_css_h: r.height, canvas_px_w: c.width, css_left: r.left, css_top: r.top, visual_w: visualViewport.width, screen_w: screen.width }; })())`));
  const png = `/tmp/phone-diag-${process.pid}.png`;
  screencap(png);
  console.log(`SCREENCAP ${png}`);
  await sleep(Number(arg('hold', 1)) * 1000);
}


// Reverse QR: the phone shows a full-width looping code, the laptop webcam reads it. The laptop page
// shows the live preview with an overlay so the holder can aim. One getUserMedia on the laptop.
// Per version: patient preflight (first valid chunk, up to 60 s while the holder moves 15-45 cm), a
// survey (module size / success by phone-screen width in webcam px), then timed trials (30 s cap).
// A version that fails steps one version down (once).
async function reverseQr() {
  const runs = Number(arg('runs', 10));
  const len = Number(arg('len', 160));
  const fps = Number(arg('fps', 4));
  const ladder = String(arg('versions', '6,4')).split(',').map(Number);
  const surveyS = Number(arg('survey', 20));
  const laptop = await laptopBrowser();
  const rx = (expr, gesture) => laptop.ev(expr, gesture);
  const phoneScreenMm = 67.7; // [INFERENCE] Samsung SM-A536E 6.5 in 20:9, width from the diagonal
  try {
    await state.chrome.openTab('about:blank');
    await state.chrome.prepare({ camera: false, silent: true });
    await laptop.goto(`http://127.0.0.1:8443/reverse-qr-receive.html`);
    const cam = await rx(`__rx.start()`, true);
    if (!(cam.track === 'live' && cam.w > 0)) throw new Error(`PREFLIGHT FAILED: the laptop webcam is not live (track ${cam.track}, width ${cam.w})`);
    console.log(JSON.stringify({ preflight: 'laptop webcam live', ...cam }));
    const results = [];
    for (let vi = 0; vi < ladder.length; vi++) {
      const version = ladder[vi];
      const tag = `reverse QR v${version} ${len} B ${fps} fps`;
      await state.chrome.goto(`${ORIGIN}/share-transmit.html?seed=1&len=${len}&version=${version}&full=1&auto=1&fps=${fps}`);
      await phoneEval(`navigator.wakeLock && navigator.wakeLock.request('screen').then((l) => { window.__wl = l; }, () => {})`);
      await sleep(800);
      const tx = await phoneEval(`JSON.stringify(window.__tx.info)`).then(JSON.parse);
      console.log(JSON.stringify({ sender_page: tx }));
      await rx(`__rx.reset(${len}, 1)`);
      // ---- patient preflight
      await rx(`__rx.setBanner('PREFLIGHT v${version}: hold the phone screen to the webcam, move SLOWLY between 15 and 45 cm. Waits up to 60 s for the first QR.')`);
      const first = await rx(`__rx.waitFirst(60000)`);
      if (!first) {
        const lg = await rx(`__rx.takeLog()`);
        const seen = lg.filter((e) => e[4]).length, bright = lg.filter((e) => e[1] > 0).length;
        const widths = lg.map((e) => e[1]).filter(Boolean).sort((a, b) => a - b);
        record({ block: tag, preflight: 'FAILED', frames: lg.length, qr_seen: seen, frames_with_phone_region: bright, phone_region_px_median: widths[Math.floor(widths.length / 2)] ?? null, phone_region_px_max: widths[widths.length - 1] ?? null });
        results.push({ version, failed: 'preflight' });
        if (vi + 1 < ladder.length) continue;
        throw new Error(`PREFLIGHT FAILED for every version tried (${ladder.slice(0, vi + 1).join(', ')}): no valid chunk decoded within 60 s each; see the recorded diagnostics`);
      }
      console.log(JSON.stringify({ preflight: 'first valid chunk decoded by the laptop webcam', version, ms: first.ms, module_px: +first.modulePx.toFixed(2), phone_region_px: first.brightW, sharp: +first.sharp.toFixed(1) }));
      // ---- survey: keep moving, count decode success by phone-screen width
      await rx(`__rx.setBanner('SURVEY v${version} (${surveyS} s): keep moving SLOWLY, near (15 cm) and far (45 cm), the overlay shows what works')`);
      await rx(`__rx.takeLog()`);
      await sleep(surveyS * 1000);
      const lg = await rx(`__rx.takeLog()`);
      const bins = {};
      for (const e of lg) {
        if (!e[1]) continue;
        const k = Math.floor(e[1] / 100) * 100;
        const b = (bins[k] ||= { frames: 0, seen: 0, valid: 0 });
        b.frames++; b.seen += e[4]; b.valid += e[3];
      }
      const vm = lg.filter((e) => e[3]).map((e) => e[5]).filter(Boolean).sort((a, b) => a - b);
      const vw = lg.filter((e) => e[3]).map((e) => e[1]).sort((a, b) => a - b);
      const estCm = (modPx) => +((phoneScreenMm * tx.width_fraction * (1280 / (modPx * tx.modules))) / 1.33 / 10).toFixed(0); // [INFERENCE] webcam horizontal field = 1.33 x distance
      record({ block: tag, survey: true, frames: lg.length, valid_frames: vm.length, module_px_min_valid: vm[0] ?? null, module_px_max_valid: vm[vm.length - 1] ?? null, module_px_median_valid: vm[Math.floor(vm.length / 2)] ?? null, phone_region_px_min_valid: vw[0] ?? null, phone_region_px_max_valid: vw[vw.length - 1] ?? null, est_distance_cm_at_min_module: vm[0] ? estCm(vm[0]) : null, est_distance_cm_at_max_module: vm.length ? estCm(vm[vm.length - 1]) : null, by_phone_region_px: bins, sender: tx });
      // ---- timed trials
      await rx(`__rx.setBanner('TRIALS v${version}: hold the phone STILL where the overlay was green (${runs} short runs)')`);
      await sleep(3000);
      const rows = [];
      for (let trial = 1; trial <= runs; trial++) {
        const b = guard(`${tag} trial ${trial}`);
        const r = await rx(`__rx.trial(30000)`);
        const row = { block: tag, trial, ok: r.ok, ms: r.ok ? r.ms : null, have: r.have, total: r.total, frames: r.frames, qr_seen: r.seen, valid: r.valid, temp_c: b.tempC };
        rows.push(row);
        record(row);
        await sleep(800);
      }
      const okRows = rows.filter((r) => r.ok);
      record({ block: tag, summary: true, trials: runs, ok: okRows.length, worst_ms: okRows.length ? Math.max(...okRows.map((r) => r.ms)) : null, median_ms: median(okRows.map((r) => r.ms)) });
      results.push({ version, ok: okRows.length });
      if (okRows.length >= 8) break;
    }
    await rx(`__rx.setBanner('done - you can put the phone down')`);
    await sleep(1500);
  } finally {
    await rx(`__rx.stop()`).catch(() => {});
    await laptop.close();
  }
}

async function audio() {
  await state.chrome.openTab('about:blank');
  const runs = Number(arg('runs', 10));
  const only = arg('only', '');
  const blocks = [
    ['fast', 56], ['slow', 56], ['fast', 128],
  ].filter(([p, l]) => (!only && true) || only === `${p}${l}`);
  let firstOk = null;
  for (const [profile, len] of blocks) {
    const ok = await audioBlock({ label: `audio ${profile} ${len} B`, profile, len, runs });
    if (firstOk === null) firstOk = ok;
    if (firstOk === 0) {
      console.log('STOP: the first audio block heard nothing; the media volume of the phone or the laptop microphone needs attention (not changed by this script)');
      break;
    }
    await sleep(8000);
  }
}

async function preflightOnly() {
  const cam = adb(['shell', 'dumpsys', 'package', BROWSER_PKG]).split('\n').filter((l) => /android\.permission\.CAMERA: granted/.test(l)).map((l) => l.trim().split(',')[0]);
  console.log(JSON.stringify({ android_camera_permission: [...new Set(cam)] }));
  await state.chrome.openTab('about:blank');
  await state.chrome.prepare({ camera: true, silent: true });
  await state.chrome.goto(`${ORIGIN}/qr-scanner.html?seed=1&len=1714`);
  await preflightCamera({ decode: false });
}

// Sender side only (no phone involved): open each window, check size and decode the QR in it.
async function senderOnly() {
  for (const [version, bytes, cols, rows, font] of [[5, 1714, 80, 24, null], [13, 1714, 120, 40, arg('font13', '11')]]) {
    const cls = 'sshub-bench-c4';
    const report = `/tmp/c4-player-report-${process.pid}.json`;
    const win = await benchWindow(cls, cols, rows, font, [process.execPath, PLAYER, '--version', String(version), '--bytes', String(bytes), '--seed', '1', '--fps', '8', '--report', report]);
    try {
      await sleep(4500);
      senderPreflight(cls, report);
    } catch (e) {
      console.log(`v${version}: ${e.message}`);
    } finally {
      await stop(win);
      fs.rmSync(report, { force: true });
    }
    await sleep(2000);
  }
}

let code = 0;
try {
  if (cmd === 'sender') {
    await senderOnly();
    process.exit(0);
  }
  guard('start');
  await setup();
  if (cmd === 'preflight') await preflightOnly();
  else if (cmd === 'camera') await camera();
  else if (cmd === 'diag') await diag();
  else if (cmd === 'reverseqr') await reverseQr();
  else if (cmd === 'rxsmoke') {
    // laptop side only: webcam live, overlay running, per-frame measurements present
    const laptop = await laptopBrowser();
    try {
      await laptop.goto('http://127.0.0.1:8443/reverse-qr-receive.html');
      console.log(JSON.stringify(await laptop.ev(`__rx.start()`, true)));
      await sleep(3000);
      console.log(JSON.stringify({ counters: await laptop.ev(`__rx.counters()`), log_sample: (await laptop.ev(`__rx.takeLog()`)).slice(-2), hud: await laptop.ev(`document.getElementById('hud').textContent`) }));
    } finally {
      await laptop.ev(`__rx.stop()`).catch(() => {});
      await laptop.close();
    }
  }
  else if (cmd === 'audio') await audio();
  else throw new Error(`unknown command ${cmd}`);
} catch (e) {
  console.log(`FAILED: ${e.stack || e.message}`);
  code = 1;
} finally {
  await cleanup(state);
  fs.writeFileSync(`/tmp/phone-ota-${cmd}.json`, JSON.stringify(out, null, 1));
}
process.exit(code);
