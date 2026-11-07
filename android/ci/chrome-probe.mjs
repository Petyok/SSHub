// Playwright against the device's Google Chrome through the emulator project's adb server (Playwright `_android`).
// `device.launchBrowser()` cannot be used here: it starts Chrome with a command-line file, which Chrome ignores on a
// user build (no `--remote-debugging-socket-name`, no `--disable-fre`), so Playwright waits for a socket that never
// appears. Chrome always exposes `chrome_devtools_remote`; this script relays it to localhost with `device.open` and
// attaches with `chromium.connectOverCDP`.
//   node android/ci/chrome-probe.mjs   (ADB_HOST=android ADB_PORT=5037; /out receives diagnostics)
import { createRequire } from 'node:module';
import { writeFileSync } from 'node:fs';
import net from 'node:net';
const { _android: android, chromium } = createRequire(import.meta.url)('playwright');

const host = process.env.ADB_HOST || 'android';
const port = Number(process.env.ADB_PORT || 5037);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const within = (ms, what, p) =>
  Promise.race([p, new Promise((_, rej) => setTimeout(() => rej(new Error(`timeout after ${ms} ms: ${what}`)), ms))]);

const devices = await android.devices({ host, port });
if (devices.length !== 1) throw new Error(`expected 1 device on ${host}:${port}, got ${devices.length}`);
const device = devices[0];
console.log(`device ${device.serial()} model=${device.model()}`);
const sh = async (c) => (await device.shell(c)).toString();

// Chrome's first run (terms, sign-in, notifications) blocks the browser; the guest is a user build, so no flag can
// skip it. Tap through it with uiautomator text matching.
const FRE_TEXTS = [/^use without an account$/i, /^no,? thanks$/i, /^don.t turn on$/i, /^accept (&|and) continue$/i, /^skip$/i, /^got it$/i, /^continue$/i, /^next$/i, /^not now$/i, /^don.t allow$/i];
async function tapOnce() {
  const xml = await sh('uiautomator dump /dev/tty');
  const nodes = [...xml.matchAll(/<node [^>]*?text="([^"]*)"[^>]*?clickable="true"[^>]*?bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"/g)];
  const hit = FRE_TEXTS.map((re) => nodes.find((n) => re.test(n[1]))).find(Boolean);
  if (!hit) return false;
  console.log('first run tap:', JSON.stringify(hit[1]));
  await sh(`input tap ${(+hit[2] + +hit[4]) / 2} ${(+hit[3] + +hit[5]) / 2}`);
  return true;
}
async function startChrome() {
  await sh('am start -n com.android.chrome/com.google.android.apps.chrome.Main');
  for (let i = 0; i < 20; i++) {
    await sleep(2500);
    const tapped = await tapOnce();
    const socket = Number((await sh('cat /proc/net/unix | grep -c chrome_devtools_remote')).trim()) > 0;
    if (!tapped && socket && i >= 2) return;
  }
  throw new Error('Chrome did not reach its tab switcher');
}

// TCP 127.0.0.1:9222 <-> localabstract:chrome_devtools_remote on the device.
function relay() {
  const server = net.createServer(async (conn) => {
    const dev = await device.open('localabstract:chrome_devtools_remote');
    conn.on('data', (d) => dev.write(d));
    dev.on('data', (d) => conn.write(d));
    conn.on('close', () => dev.close().catch(() => {}));
    dev.on('close', () => conn.destroy());
    conn.on('error', () => {});
  });
  return new Promise((res) => server.listen(9222, '127.0.0.1', () => res(server)));
}

let server, browser;
try {
  await within(120_000, 'start Chrome', startChrome());
  server = await relay();
  browser = await within(30_000, 'connectOverCDP', chromium.connectOverCDP('http://127.0.0.1:9222'));
  console.log('browser version:', browser.version());
  const context = browser.contexts()[0] ?? (await browser.newContext());
  const page = await within(30_000, 'newPage', context.newPage());
  await within(30_000, 'goto', page.goto('data:text/html,<title>sshub-probe</title><h1 id=x>probe-ok</h1>'));
  console.log('title:', await page.title());
  console.log('h1:', await page.textContent('#x'));
  console.log('userAgent:', await page.evaluate(() => navigator.userAgent));
  console.log('secureContext:', await page.evaluate(() => window.isSecureContext));
  await page.close();
} catch (e) {
  console.log('FAILED:', e.message);
  console.log('top activity:', (await sh('dumpsys activity activities | grep -m1 topResumedActivity').catch(() => '')).trim());
  try { writeFileSync('/out/chrome-fail.png', await device.screenshot()); } catch {}
  process.exitCode = 1;
} finally {
  await browser?.close().catch(() => {});
  server?.close();
  await device.close().catch(() => {});
}
process.exit(process.exitCode ?? 0);
