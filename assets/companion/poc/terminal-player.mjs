#!/usr/bin/env node
// Terminal player for PoC C4 (handheld read). Loops the indexed QR frames as half-block text.
// Usage: node terminal-player.mjs [--version 5] [--ecl M] [--bytes 4096] [--seed 1] [--fps 8]
//                                 [--seconds 0] [--once] [--report FILE]
// --report writes the real terminal size and the size the code needs; when the code does not fit
// (a wrapped half-block QR is striped and unreadable) the player exits with status 3 instead of drawing.
// Loads vendor/qrcode.js and share-lib.js into a vm context; no npm dependencies.
// Dark modules are explicit truecolor black, light modules explicit white, quiet zone 4.
// Expected payload (seed, length, sha256) goes to stderr so the phone side can compare.
import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import vm from "node:vm";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const opt = (k, d) => {
  const i = argv.indexOf(`--${k}`);
  return i < 0 ? d : argv[i + 1];
};
const once = argv.includes("--once");
const version = Number(opt("version", 5));
const ecl = opt("ecl", "M");
const nBytes = Number(opt("bytes", 4096));
const seed = Number(opt("seed", 1));
const fps = Number(opt("fps", 8));
const seconds = Number(opt("seconds", 0));

const ctx = { console };
ctx.window = ctx;
vm.createContext(ctx);
for (const f of ["vendor/qrcode.js", "share-lib.js"]) {
  vm.runInContext(readFileSync(join(here, f), "utf8"), ctx, { filename: f });
}
const L = ctx.ShareLib;

const bytes = L.seededBytes(nBytes, seed);
const chunk = L.byteCapacity(version, ecl) - L.CHUNK_HDR;
const frames = L.fountainChunks(bytes, chunk);
const qrs = frames.map((f) => L.qrFor(f, version, ecl));
const sha = createHash("sha256").update(bytes).digest("hex");
process.stderr.write(`expected seed=${seed} bytes=${nBytes} sha256=${sha}\n`);
process.stderr.write(`frames=${frames.length} chunk=${chunk} version=${version} ecl=${ecl} fps=${fps}\n`);

const QUIET = 4;
const colour = (dark) => (dark ? "0;0;0" : "255;255;255");

// One text line per two module rows: fg = top module, bg = bottom module, glyph U+2580.
function render(qr) {
  const n = qr.getModuleCount();
  const size = n + 2 * QUIET;
  const dark = (r, c) => r >= QUIET && c >= QUIET && r < QUIET + n && c < QUIET + n && qr.isDark(r - QUIET, c - QUIET);
  let out = "";
  for (let r = 0; r < size; r += 2) {
    for (let c = 0; c < size; c++) {
      out += `\x1b[38;2;${colour(dark(r, c))}m\x1b[48;2;${colour(dark(r + 1, c))}m\u2580`;
    }
    out += "\x1b[0m\n";
  }
  return { text: out, lines: Math.ceil(size / 2), width: size };
}

const report = opt("report", "");
if (report) {
  const need = render(qrs[0]);
  const cols = process.stdout.columns || 0;
  const rowsT = process.stdout.rows || 0;
  const fits = cols >= need.width && rowsT >= need.lines;
  writeFileSync(report, JSON.stringify({ columns: cols, rows: rowsT, need_width: need.width, need_lines: need.lines, fits }));
  if (!fits) {
    process.stderr.write(`QR needs ${need.width}x${need.lines} cells, terminal is ${cols}x${rowsT}\n`);
    process.exit(3);
  }
}

if (once) {
  const f = render(qrs[0]);
  process.stdout.write(f.text);
  process.stderr.write(`check lines=${f.lines} width=${f.width}\n`);
  process.exit(0);
}

process.stdout.write("\x1b[2J");
let i = 0;
const timer = setInterval(() => {
  process.stdout.write("\x1b[H" + render(qrs[i]).text);
  i = (i + 1) % qrs.length;
}, 1000 / fps);
if (seconds > 0) {
  setTimeout(() => {
    clearInterval(timer);
    process.stdout.write("\x1b[0m\n");
    process.exit(0);
  }, seconds * 1000);
}
