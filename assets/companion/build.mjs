// Builds the phone page. `node build.mjs` writes sshub-companion.html and FINGERPRINT;
// `node build.mjs --selftest` writes build/selftest.html (the vector replay harness, never shipped).
// One self-contained file: the script and the style are inline and pinned in the CSP by sha256,
// there is no other script source and no network access of any kind. esbuild is a pinned
// devDependency, installed with `npm ci` in the tools container only.
//
// Fingerprint: sha256 of the finished file with its own 64 hex characters replaced by 64 zeros
// (a file cannot contain its own hash otherwise). The page shows it, the terminal embeds it,
// and anyone can recompute it with the sed line printed by the About screen.
import * as esbuild from 'esbuild';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const read = (p) => fs.readFileSync(path.join(here, p), 'utf8');
const selftest = process.argv.includes('--selftest');
const sha256 = (s) => crypto.createHash('sha256').update(s).digest();
const ZERO = '0'.repeat(64);

const entry = selftest
  ? `import { start } from './src/selftest.mjs';
globalThis.__VECTORS__ = ${JSON.stringify(read('../../tests/vectors/crypto/vectors.json'))};
globalThis.__EFF__ = ${JSON.stringify(read('../wordlists/eff_large.txt'))};
start();
`
  : `import { start } from './src/page.mjs';
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
const css = read('src/style.css');
const pin = (s) => `'sha256-${sha256(s).toString('base64')}'`;
const csp = [
  "default-src 'none'",
  `script-src ${pin(js)} 'wasm-unsafe-eval'`,
  `style-src ${pin(css)}`,
  "connect-src 'none'",
  "img-src 'none'",
  "base-uri 'none'",
  "form-action 'none'",
].join('; ');

const withFp = (fp) => `<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<meta http-equiv="Content-Security-Policy" content="${csp}">
<meta name="sshub-fingerprint" content="${fp}">
<title>sshub companion</title>
<style>${css}</style>
<div id="app"></div>
<script>${js}</script>
</html>
`;

const html = withFp(ZERO);
const fingerprint = sha256(html).toString('hex');
const final = withFp(fingerprint);
const target = selftest ? path.join(here, 'build', 'selftest.html') : path.join(here, 'sshub-companion.html');
fs.mkdirSync(path.dirname(target), { recursive: true });
fs.writeFileSync(target, final);
if (!selftest) fs.writeFileSync(path.join(here, 'FINGERPRINT'), `${fingerprint}\n`);
console.log(JSON.stringify({ file: path.relative(path.join(here, '..', '..'), target), bytes: Buffer.byteLength(final), file_sha256: sha256(final).toString('hex'), fingerprint }));
