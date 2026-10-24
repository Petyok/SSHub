// Offline runner for PoC C (QR budget) and PoC H (share channel) self-tests.
// No camera, no phone, no network beyond 127.0.0.1. Prints one JSON document on stdout.
//
// env: OUT_DIR (PNG and case files), PORT (http.server port), PY (python with zxingcpp and PIL),
//      PLAYWRIGHT_MJS (path to playwright index.mjs when not resolvable from here).
import { spawn, execFile } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { promisify } from "node:util";

const execFileP = promisify(execFile);
const root = new URL(".", import.meta.url).pathname;
const outDir = process.env.OUT_DIR ?? "/tmp/poc-c-png";
const port = process.env.PORT ?? "8771";
const py = process.env.PY ?? "python3";
const decoder = new URL("decode-zxing.py", import.meta.url).pathname;
mkdirSync(outDir, { recursive: true });

const server = spawn(
  "python3",
  ["-m", "http.server", port, "--bind", "127.0.0.1", "--directory", root],
  { stdio: "ignore" },
);
const base = `http://127.0.0.1:${port}`;

let browser = null;
try {
  let up = false;
  for (let i = 0; i < 50 && !up; i++) {
    try { await fetch(base + "/"); up = true; } catch { await new Promise((r) => setTimeout(r, 100)); }
  }
  if (!up) throw new Error("http.server did not start");

  const { chromium } = await import(process.env.PLAYWRIGHT_MJS ?? "playwright");
  browser = await chromium.launch({ headless: true });
  const out = { chromium: browser.version(), port, outDir };

  // ---- C1 + C2: render, screenshot, decode ----
  const ctx = await browser.newContext({ viewport: { width: 1400, height: 900 }, deviceScaleFactor: 2 });
  const page = await ctx.newPage();
  page.setDefaultTimeout(120000);
  const pageErrors = [];
  page.on("pageerror", (e) => pageErrors.push(String(e)));
  await page.goto(base + "/qr-budget-rig.html");
  await page.waitForFunction(() => window.ready === true);
  out.capacity = await page.evaluate(() => window.capacityTable(40));

  const geoms = [[80, 24], [120, 40]];
  const rows = [];
  const cases = [];
  for (const theme of ["light", "dark"]) {
    for (const [cols, rowsN] of geoms) {
      for (let v = 1; v <= 16; v++) {
        const r = await page.evaluate(
          (o) => window.renderQR(o),
          { version: v, cols, rows: rowsN, theme, seed: 1000 + v, quiet: 4 },
        );
        if (!r.fits) { rows.push({ ...r, theme }); continue; }
        const file = `${outDir}/${theme}-${cols}x${rowsN}-v${v}.png`;
        await page.locator(".xterm").first().screenshot({ path: file });
        const row = { ...r, theme, file };
        cases.push(row);
        rows.push(row);
      }
    }
  }
  const casesFile = `${outDir}/cases.json`;
  writeFileSync(casesFile, JSON.stringify(cases));
  const { stdout } = await execFileP(py, [decoder, casesFile], { maxBuffer: 64 * 1024 * 1024 });
  const decoded = new Map(JSON.parse(stdout).map((c) => [c.file, c]));

  const c1 = rows.map((r) => {
    if (!r.fits) return { theme: r.theme, geometry: `${r.cols}x${r.rows}`, version: r.version, modules: r.modules, lines: r.lines, width: r.width, fits: false };
    const d = decoded.get(r.file);
    const ok = (hex) => hex !== null && hex === r.expected_hex;
    return {
      theme: r.theme, geometry: `${r.cols}x${r.rows}`, version: r.version,
      modules: r.modules, lines: r.lines, width: r.width, fits: true,
      payload_bytes: r.payload_bytes, cell_w_px: r.cell_w_px, cell_h_px: r.cell_h_px,
      cell_aspect_h_over_w: r.cell_aspect_h_over_w,
      legible_plain: ok(d.plain_hex),
      legible_invert_allowed: ok(d.plain_hex) || ok(d.invert_hex),
      file: r.file,
    };
  });
  out.c1 = c1;

  // Largest legible version per geometry and theme (invert allowed), plus the plain-only figure.
  const maxLegible = (theme, geometry, key) => {
    const ok = c1.filter((r) => r.fits && r.theme === theme && r.geometry === geometry && r[key]);
    return ok.length ? Math.max(...ok.map((r) => r.version)) : 0;
  };
  out.c1_summary = {};
  for (const [cols, rowsN] of geoms) {
    const g = `${cols}x${rowsN}`;
    out.c1_summary[g] = {
      light: maxLegible("light", g, "legible_invert_allowed"),
      dark: maxLegible("dark", g, "legible_invert_allowed"),
      light_plain: maxLegible("light", g, "legible_plain"),
      dark_plain: maxLegible("dark", g, "legible_plain"),
      min_across_themes: Math.min(maxLegible("light", g, "legible_invert_allowed"), maxLegible("dark", g, "legible_invert_allowed")),
    };
  }
  const vStar = out.c1_summary["80x24"].min_across_themes;
  out.version_used_for_c3 = vStar;

  // ---- C3: frames, decode in memory, offline reassembly (no loss, looped cycle not needed) ----
  // Challenge sizing (1519 B ShareRelease challenge, 5109 B four-holder CredUpdate estimate) at v5 and v13.
  out.c3 = vStar > 0
    ? await page.evaluate((o) => window.runC3(o), { version: vStar, sizes: [4096, 6144, 1519, 5109], fps: [4, 8] })
    : [];
  out.c3_challenge = [];
  for (const v of [5, 13]) out.c3_challenge.push(...(await page.evaluate((o) => window.runC3(o), { version: v, sizes: [1519, 5109], fps: [8] })));

  // ---- H self-tests (offline) ----
  const hp = await ctx.newPage();
  hp.setDefaultTimeout(900000);
  hp.on("pageerror", (e) => pageErrors.push("H: " + String(e)));
  await hp.goto(base + "/share-channel-bench.html");
  await hp.waitForFunction(() => typeof window.runSelfTests === "function");
  out.h = await hp.evaluate(() => window.runSelfTests({ fskClean: 50, fskNoise: 50, snrs: [20, 10, 5, 0], qrVersion: 10 }));

  out.page_errors = pageErrors;
  console.log(JSON.stringify(out, null, 2));
} finally {
  if (browser) await browser.close();
  server.kill();
}
