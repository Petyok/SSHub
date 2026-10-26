// Playwright `_android` against the emulator project's adb server: opens a page in the device's Google Chrome.
// Run in the tools image (node 22, Playwright 1.64.1) on the emulator's compose network:
//   node android/ci/chrome-probe.mjs   (ADB_HOST=android ADB_PORT=5037; /out is where diagnostics are written)
import { createRequire } from 'node:module';
import { writeFileSync } from 'node:fs';
const { _android: android } = createRequire(import.meta.url)('playwright');

const host = process.env.ADB_HOST || 'android';
const port = Number(process.env.ADB_PORT || 5037);
const within = (ms, what, p) =>
  Promise.race([p, new Promise((_, rej) => setTimeout(() => rej(new Error(`timeout after ${ms} ms: ${what}`)), ms))]);

const devices = await android.devices({ host, port });
if (devices.length !== 1) throw new Error(`expected 1 device on ${host}:${port}, got ${devices.length}`);
const device = devices[0];
console.log(`device ${device.serial()} model=${device.model()}`);

async function diagnose(tag) {
  try {
    console.log(`--- diagnostics (${tag})`);
    console.log('chrome pid:', (await device.shell('pidof com.android.chrome')).toString().trim());
    console.log('sockets:', (await device.shell('cat /proc/net/unix | grep -c devtools')).toString().trim());
    console.log('top activity:', (await device.shell('dumpsys activity activities | grep -m1 topResumedActivity')).toString().trim());
    writeFileSync(`/out/chrome-${tag}.png`, await device.screenshot());
  } catch (e) {
    console.log('diagnose failed:', e.message);
  }
}

// Chrome's first run (terms, sign-in, notifications) blocks everything until it is answered; the guest is a user
// build, so no command-line flag can skip it. Tap through it with uiautomator text matching, bounded.
const FRE_TEXTS = [/^accept (&|and) continue$/i, /^no thanks$/i, /^skip$/i, /^got it$/i, /^continue$/i, /^next$/i, /^not now$/i, /^don.t allow$/i];
async function firstRun() {
  await device.shell('am start -n com.android.chrome/com.google.android.apps.chrome.Main');
  for (let i = 0; i < 12; i++) {
    await new Promise((r) => setTimeout(r, 2500));
    const top = (await device.shell('dumpsys activity activities | grep -m1 topResumedActivity')).toString();
    if (!/firstrun|signin|FirstRun|Permission/i.test(top) && i > 0) { console.log('first run done:', top.trim()); return; }
    const xml = (await device.shell('uiautomator dump /dev/tty')).toString();
    const nodes = [...xml.matchAll(/<node [^>]*?text="([^"]*)"[^>]*?clickable="true"[^>]*?bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"/g)];
    const hit = FRE_TEXTS.map((re) => nodes.find((n) => re.test(n[1]))).find(Boolean);
    if (!hit) { console.log('first run step', i, 'no known button; texts:', nodes.map((n) => n[1]).filter(Boolean).join(' | ')); continue; }
    const [x, y] = [(+hit[2] + +hit[4]) / 2, (+hit[3] + +hit[5]) / 2];
    console.log('first run tap:', JSON.stringify(hit[1]));
    await device.shell(`input tap ${x} ${y}`);
  }
  throw new Error('first run did not finish');
}

let context;
try {
  await within(60_000, 'first run', firstRun());
  context = await within(90_000, 'launchBrowser', device.launchBrowser());
  const page = await within(30_000, 'newPage', context.newPage());
  await within(30_000, 'goto', page.goto('data:text/html,<title>sshub-probe</title><h1 id=x>probe-ok</h1>'));
  console.log('title:', await page.title());
  console.log('h1:', await page.textContent('#x'));
  console.log('userAgent:', await page.evaluate(() => navigator.userAgent));
  console.log('secureContext:', await page.evaluate(() => window.isSecureContext));
} catch (e) {
  console.log('FAILED:', e.message);
  await diagnose('fail');
  process.exitCode = 1;
} finally {
  await context?.close().catch(() => {});
  await device.close().catch(() => {});
}
process.exit(process.exitCode ?? 0);
