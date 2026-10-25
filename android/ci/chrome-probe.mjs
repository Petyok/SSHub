// Playwright `_android` against the emulator project's adb server: opens a page in the device's Google Chrome.
// Run in the tools image (node 22, Playwright 1.64.1) on the emulator's compose network:
//   node android/ci/chrome-probe.mjs   (ADB_HOST=android ADB_PORT=5037)
import { createRequire } from 'node:module';
const { _android: android } = createRequire(import.meta.url)('playwright');

const host = process.env.ADB_HOST || 'android';
const port = Number(process.env.ADB_PORT || 5037);
const devices = await android.devices({ host, port });
if (devices.length !== 1) throw new Error(`expected 1 device on ${host}:${port}, got ${devices.length}`);
const device = devices[0];
console.log(`device ${device.serial()} model=${device.model()}`);
const context = await device.launchBrowser();
try {
  const page = await context.newPage();
  await page.goto('data:text/html,<title>sshub-probe</title><h1 id=x>probe-ok</h1>');
  console.log('title:', await page.title());
  console.log('h1:', await page.textContent('#x'));
  console.log('userAgent:', await page.evaluate(() => navigator.userAgent));
  console.log('secureContext:', await page.evaluate(() => window.isSecureContext));
} finally {
  await context.close();
  await device.close();
}
