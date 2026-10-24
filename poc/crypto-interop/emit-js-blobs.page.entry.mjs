// PoC A - browser entry for emit-js-blobs.html (A2 direction).
// Runs the same produceBlobs() as the node producer, renders the result and
// offers blobs.json as a download. In node (cold-start measurement) the DOM
// branch is skipped; the bundle exports blobsPromise.
import { produceBlobs } from './test/blobs.mjs';
const __t0 = typeof performance !== 'undefined' ? performance.now() : 0;
export const blobsPromise = produceBlobs();
blobsPromise.then(() => {
  if (typeof document !== 'undefined') {
    window.__blobMs = (typeof performance !== 'undefined' ? performance.now() : 0) - __t0;
  }
});

if (typeof document !== 'undefined') {
  const esc = (s) => String(s).replace(/[&<>]/g, (c) => ({ '&': '&', '<': '<', '>': '>' }[c]));
  blobsPromise
    .then((data) => {
      document.getElementById('status').textContent =
        `blobs produced in ${(window.__blobMs ?? 0).toFixed(0)} ms (script exec, excludes browser startup) - offline, no network used`;
      const out = document.getElementById('out');
      for (const [kind, b] of Object.entries(data.blobs)) {
        for (const [name, hexv] of Object.entries(b.output)) {
          const row = document.createElement('div');
          row.className = 'row';
          const title = document.createElement('div');
          title.className = 'title';
          title.textContent = `${kind} · ${name} · ${hexv.length / 2} bytes`;
          const pre = document.createElement('pre');
          pre.textContent = hexv;
          row.append(title, pre);
          out.append(row);
        }
      }
      // download link for blobs.json (runtime Blob URL, no external resource)
      const a = document.createElement('a');
      a.textContent = 'Download blobs.json';
      a.download = 'blobs.json';
      a.href = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' }));
      a.className = 'dl';
      out.prepend(a);
    })
    .catch((e) => {
      document.getElementById('status').textContent = `ERROR: ${e.message}`;
      document.getElementById('status').className = 'err';
      console.error(e);
    });
}
