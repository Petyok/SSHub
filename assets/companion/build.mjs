// Builds sshub-companion.html: one self-contained page. Vectors and the EFF word list are
// embedded as globals; the page makes no network requests (CSP connect-src/default-src none).
// esbuild is a pinned devDependency, installed with `npm ci` in the tools container only.
import * as esbuild from 'esbuild';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const read = (p) => fs.readFileSync(path.join(here, p), 'utf8');
const vectors = read('../../tests/vectors/crypto/vectors.json');
const eff = read('../wordlists/eff_large.txt');

const entry = `import { start } from './src/page.mjs';
globalThis.__VECTORS__ = ${JSON.stringify(vectors)};
globalThis.__EFF__ = ${JSON.stringify(eff)};
start();
`;

const out = await esbuild.build({
  stdin: { contents: entry, resolveDir: here, sourcefile: 'entry.mjs', loader: 'js' },
  bundle: true,
  write: false,
  format: 'iife',
  platform: 'browser',
  target: 'es2020',
  minify: true,
  legalComments: 'none',
  charset: 'utf8',
  logLevel: 'warning',
});
// A literal "</script" inside the bundle would end the inline script early.
const js = out.outputFiles[0].text.replace(/<\/script/gi, '<\\/script');

const html = `<!doctype html>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline' 'wasm-unsafe-eval'; connect-src 'none'; img-src 'none'; style-src 'none'">
<title>sshub companion (stage 5 harness)</title>
<script>
${js}
</script>
`;

const target = path.join(here, 'sshub-companion.html');
fs.writeFileSync(target, html);
const sha = crypto.createHash('sha256').update(html).digest('hex');
console.log(JSON.stringify({ file: 'assets/companion/sshub-companion.html', bytes: Buffer.byteLength(html), sha256: sha }));
