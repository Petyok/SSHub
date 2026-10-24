// Headless Chromium check for sshub-companion.html, served from http://localhost.
// Fresh browser per run (cold start). Counts every request that leaves the page origin;
// the page must make none. Prints one JSON line; exits non-zero unless pass and external === 0.
// Needs playwright (global install in the tools image; NODE_PATH points at it).
import { createRequire } from 'node:module';
const { chromium } = createRequire(import.meta.url)('playwright');
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const file = path.resolve(here, '..', 'build', 'selftest.html');
const body = fs.readFileSync(file);

const server = http.createServer((req, res) => {
  if (req.url !== '/' && req.url !== '/index.html') {
    res.writeHead(404).end();
    return;
  }
  res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'content-length': body.length }).end(body);
});
await new Promise((r) => server.listen(0, 'localhost', r));
const origin = `http://localhost:${server.address().port}`;

const t0 = Date.now();
const browser = await chromium.launch({ headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
const page = await browser.newPage();
const external = [];
const requests = [];
page.on('request', (req) => {
  requests.push(req.url());
  if (!req.url().startsWith(origin)) external.push(req.url());
});
let result = null;
let error = null;
try {
  await page.goto(`${origin}/`, { waitUntil: 'load', timeout: 60000 });
  await page.waitForFunction(() => globalThis.__done === true, null, { timeout: 900000, polling: 250 });
  result = await page.evaluate(() => globalThis.__result);
} catch (e) {
  error = String(e && e.message ? e.message : e);
}
const coldMs = Date.now() - t0;
await browser.close();
server.close();

const out = {
  url: `${origin}/`,
  bytes: body.length,
  cold_start_ms: coldMs,
  requests: requests.length,
  external_requests: external.length,
  external_urls: external,
  result,
  error,
};
console.log(JSON.stringify(out));
const ok = error === null && result && result.pass === true && external.length === 0;
process.exit(ok ? 0 : 1);
