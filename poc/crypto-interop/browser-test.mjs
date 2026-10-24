// PoC A browser test: loads dist/emit-js-blobs.html in headless Chromium from
// http://localhost (a secure context, unlike a data: URL), with every non-local
// request blocked, and checks A1 (vector replay), A2 (blob production), A4
// (size, cold start, offline). Writes blobs-browser.json for verify-js-blobs.
// Run inside the tools container: node browser-test.mjs [runs]
import http from 'node:http';
import fs from 'node:fs';
import { createRequire } from 'node:module';

const require = createRequire('/opt/node/lib/node_modules/');
const { chromium } = require('playwright');

const html = fs.readFileSync('dist/emit-js-blobs.html');
const server = http.createServer((req, res) => {
  res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
  res.end(html);
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const port = server.address().port;
const base = `http://localhost:${port}/`;
const runs = Number(process.argv[2] || 5);

async function oneRun(query, timeoutMs) {
  const t0 = performance.now(); // browser launch is part of a cold start
  const browser = await chromium.launch();
  try {
    const context = await browser.newContext();
    let external = 0;
    let requests = 0;
    await context.route('**/*', (route) => {
      requests++;
      if (!route.request().url().startsWith(base)) {
        external++;
        return route.abort();
      }
      return route.continue();
    });
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', (e) => errors.push(String(e)));
    page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
    const tn = performance.now();
    await page.goto(base + query);
    await page.waitForFunction(() => window.__done === true, null, { timeout: timeoutMs });
    const wall = performance.now() - tn;
    const result = await page.evaluate(() => window.__result);
    const nav = await page.evaluate(() => {
      const n = performance.getEntriesByType('navigation')[0];
      return { dcl: n.domContentLoadedEventEnd, load: n.loadEventEnd };
    });
    const mem = await page.evaluate(() => (performance.memory ? performance.memory.jsHeapSizeLimit : null));
    return { result, wall, sinceLaunch: performance.now() - t0, nav, requests, external, errors, version: browser.version(), mem };
  } finally {
    await browser.close();
  }
}

const report = { page_bytes: html.length, base, runs: [], version: null };
for (let k = 0; k < runs; k++) {
  const r = await oneRun('', 120000);
  report.version = r.version;
  report.runs.push({
    ok: r.result.ok,
    secureContext: r.result.secureContext,
    vectors: r.result.vectors,
    skipped: r.result.skipped,
    failed: r.result.failed,
    blobMs: Math.round(r.result.blobMs),
    replayMs: Math.round(r.result.replayMs),
    coldWallMs: Math.round(r.wall),
    sinceLaunchMs: Math.round(r.sinceLaunch),
    requests: r.requests,
    externalBlocked: r.external,
    errors: r.errors,
  });
  if (k === 0) fs.writeFileSync('blobs-browser.json', JSON.stringify(r.result.blobs, null, 2) + '\n');
}
// the 1 GiB Argon2 profile inside a tab (informational, PoC D owns performance)
try {
  const m = await oneRun('?master=1', 180000);
  report.master = { ok: m.result.ok, failed: m.result.failed, replayMs: Math.round(m.result.replayMs), errors: m.errors };
} catch (e) {
  report.master = { ok: false, error: String(e.message).split('\n')[0] };
}
server.close();
const med = (a) => a.slice().sort((x, y) => x - y)[Math.floor(a.length / 2)];
report.summary = {
  all_ok: report.runs.every((r) => r.ok && r.secureContext && r.errors.length === 0),
  external_requests_blocked: report.runs.reduce((s, r) => s + r.externalBlocked, 0),
  median_cold_wall_ms: med(report.runs.map((r) => r.coldWallMs)),
  median_since_launch_ms: med(report.runs.map((r) => r.sinceLaunchMs)),
};
console.log(JSON.stringify(report, null, 2));
process.exit(report.summary.all_ok ? 0 : 1);
