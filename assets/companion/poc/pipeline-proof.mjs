#!/usr/bin/env node
// Headless pipeline proof for PoC H (mic FSK and camera QR receive) and PoC C4 (QR scanner).
// Renders fake-capture inputs from share-lib.js: a WAV burst for the mic and y4m
// video of looping QR frames for the camera. Serves this directory on loopback and
// drives Chromium with fake media devices. Writes results.json to the output dir.
// Usage: PLAYWRIGHT_MJS=<playwright index.mjs> node pipeline-proof.mjs [outdir]
// Exits 1 if any pipeline fails.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { createServer } from "node:http";
import { join, dirname, extname, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import { createHash } from "node:crypto";

const here = dirname(fileURLToPath(import.meta.url));
const outDir = process.argv[2] || "/tmp/pipeline-proof";
mkdirSync(outDir, { recursive: true });
const { chromium } = await import(process.env.PLAYWRIGHT_MJS);

// share-lib in a vm context, the same loading as terminal-player.mjs.
const vctx = { console };
vctx.window = vctx;
vm.createContext(vctx);
for (const f of ["vendor/qrcode.js", "share-lib.js"]) {
  vm.runInContext(readFileSync(join(here, f), "utf8"), vctx, { filename: f });
}
const L = vctx.ShareLib;
const sha256 = (b) => createHash("sha256").update(b).digest("hex");

// 16-bit mono PCM WAV.
function writeWav(path, samples, sr) {
  const n = samples.length;
  const b = Buffer.alloc(44 + 2 * n);
  b.write("RIFF", 0); b.writeUInt32LE(36 + 2 * n, 4); b.write("WAVE", 8);
  b.write("fmt ", 12); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22);
  b.writeUInt32LE(sr, 24); b.writeUInt32LE(sr * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34);
  b.write("data", 36); b.writeUInt32LE(2 * n, 40);
  for (let i = 0; i < n; i++) {
    b.writeInt16LE(Math.max(-32767, Math.min(32767, Math.round(samples[i] * 32767))), 44 + 2 * i);
  }
  writeFileSync(path, b);
}

// y4m (4:2:0) of QR matrices, quiet zone 4 modules, black on white, chroma neutral.
// Pixel size is scale per module; both sides are even so 4:2:0 is valid.
function writeY4m(path, qrs, scale, fps) {
  const n = qrs[0].getModuleCount();
  const quiet = 4;
  const W = (n + 2 * quiet) * scale;
  const H = W;
  if (W % 2) throw new Error("y4m size must be even");
  const Y = Buffer.alloc(W * H);
  const C = Buffer.alloc((W / 2) * (H / 2), 128);
  const parts = [Buffer.from(`YUV4MPEG2 W${W} H${H} F${fps}:1 Ip A1:1 C420jpeg\n`)];
  for (const qr of qrs) {
    for (let y = 0; y < H; y++) {
      const r = Math.floor(y / scale) - quiet;
      for (let x = 0; x < W; x++) {
        const c = Math.floor(x / scale) - quiet;
        const dark = r >= 0 && c >= 0 && r < n && c < n && qr.isDark(r, c);
        Y[y * W + x] = dark ? 0 : 255;
      }
    }
    parts.push(Buffer.from("FRAME\n"), Buffer.from(Y), C, C);
  }
  writeFileSync(path, Buffer.concat(parts));
}

// Mic input: 1 s silence, the FSK burst, 2 s silence. The fake device loops the file.
function burstWav(profileName) {
  const F = profileName === "fast" ? L.FSK_FAST : L.FSK;
  const sr = L.FSK.sampleRate;
  const burst = L.fskEncode(L.frame(L.seededBytes(56, 1)), F);
  const s = new Float32Array(sr + burst.length + 2 * sr);
  s.set(burst, sr);
  const path = join(outDir, `burst-${profileName}.wav`);
  writeWav(path, s, sr);
  return path;
}

// Camera input: looping indexed QR frames for a seeded payload, scale 6 (both sizes even).
function qrY4m(name, len, version, fps) {
  const payload = L.seededBytes(len, 1);
  const frames = L.fountainChunks(payload, L.byteCapacity(version, "M") - L.CHUNK_HDR);
  const qrs = frames.map((f) => L.qrFor(f, version, "M"));
  const path = join(outDir, `${name}.y4m`);
  writeY4m(path, qrs, 6, fps);
  return { path, frames: frames.length, sha: sha256(payload) };
}

// Static server for this directory, loopback only, random port.
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css" };
const server = createServer((req, res) => {
  const p = resolve(here, "." + decodeURIComponent(new URL(req.url, "http://x").pathname));
  if (!p.startsWith(here + sep)) { res.writeHead(403).end(); return; }
  try {
    res.writeHead(200, { "content-type": TYPES[extname(p)] || "application/octet-stream" });
    res.end(readFileSync(p));
  } catch {
    res.writeHead(404).end();
  }
});
await new Promise((ok) => server.listen(0, "127.0.0.1", ok));
const base = `http://127.0.0.1:${server.address().port}`;

const results = [];
const record = (row) => { results.push(row); console.log(JSON.stringify(row)); };

async function launch(extraArgs) {
  return chromium.launch({
    headless: true,
    args: ["--use-fake-ui-for-media-stream", "--use-fake-device-for-media-stream", ...extraArgs],
  });
}

// (a) Receive page, audio path only (camera off). Up to 3 trials; stop at the first match.
async function audioPipeline(profile) {
  const wav = burstWav(profile);
  const browser = await launch([`--use-file-for-fake-audio-capture=${wav}`]);
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/share-receive.html`);
  await page.fill("#seed", "1");
  await page.fill("#len", "56");
  await page.selectOption("#profile", profile);
  await page.uncheck("#useqr");
  await page.click("#arm");
  await page.waitForFunction(() => document.getElementById("status").textContent === "armed");
  let row = null;
  for (let t = 1; t <= 3 && !(row && row[1] === "true"); t++) {
    await page.click("#trial");
    await page.waitForFunction((n) => document.querySelectorAll("#rows tr").length >= n, t, { timeout: 30000 });
    row = await page.$eval(`#rows tr:nth-child(${t})`, (tr) => [...tr.cells].map((c) => c.textContent));
  }
  await browser.close();
  record({
    pipeline: `H audio receive (${profile})`,
    pass: row[1] === "true" && errors.length === 0,
    trial: row[0], audio_ok: row[1], audio_ms: row[2], page_errors: errors.length,
  });
}

// (b1) Receive page, camera path: looping v10 QR frames for seed 1, 56 bytes.
async function receiveCameraPipeline() {
  const y = qrY4m("qr-v10-56", 56, 10, 4);
  const browser = await launch([`--use-file-for-fake-video-capture=${y.path}`]);
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/share-receive.html`);
  await page.fill("#seed", "1");
  await page.fill("#len", "56");
  await page.click("#arm");
  await page.waitForFunction(() => document.getElementById("status").textContent === "armed");
  await page.click("#trial");
  await page.waitForFunction(() => document.querySelectorAll("#rows tr").length >= 1, null, { timeout: 30000 });
  const row = await page.$eval("#rows tr:nth-child(1)", (tr) => [...tr.cells].map((c) => c.textContent));
  await browser.close();
  record({
    pipeline: "H camera QR receive (v10, 56 B)",
    pass: row[3] === "true" && errors.length === 0,
    qr_ok: row[3], qr_ms: row[4], frames: y.frames, page_errors: errors.length,
  });
}

// (b2) C4 scanner: looping v5 QR frames for seed 1, 4096 bytes. Time to MATCH.
async function scannerPipeline() {
  const y = qrY4m("qr-v5-4096", 4096, 5, 4);
  const browser = await launch([`--use-file-for-fake-video-capture=${y.path}`]);
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(`${base}/qr-scanner.html?seed=1&len=4096`);
  await page.click("#start");
  await page.waitForSelector("#result .pass, #result .fail", { timeout: 180000 });
  const text = await page.$eval("#result", (el) => el.textContent);
  const pass = (await page.$("#result .pass")) !== null;
  await browser.close();
  const secs = Number((text.match(/complete in ([\d.]+) s/) || [])[1]);
  record({
    pipeline: "C4 QR scanner (v5, 4096 B)",
    pass: pass && errors.length === 0,
    elapsed_s: secs, frames: y.frames, sha256_expected: y.sha, result: text.trim(), page_errors: errors.length,
  });
}

try {
  for (const p of ["slow", "fast"]) await audioPipeline(p);
  await receiveCameraPipeline();
  await scannerPipeline();
} finally {
  server.close();
}
writeFileSync(join(outDir, "results.json"), JSON.stringify(results, null, 1));
process.exit(results.every((r) => r.pass) ? 0 : 1);
