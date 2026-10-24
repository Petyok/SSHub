// Browser entry of the shipped page (sshub-companion.html).
import { startApp } from './app.mjs';

export function start() {
  const fp = document.querySelector('meta[name="sshub-fingerprint"]');
  startApp(document.getElementById('app'), { fingerprint: fp ? fp.content : '' }).catch((e) => {
    document.getElementById('app').textContent = `The page failed to start: ${e && e.message ? e.message : e}`;
  });
}
