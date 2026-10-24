// Builds dist/emit-js-blobs.html: one self-contained file with every vendored
// library, the vectors and the EFF word list inlined. Needs esbuild (the build
// script installs a pinned copy under /tmp, nothing is added to the repo).
import { createRequire } from 'node:module';
import fs from 'node:fs';
import zlib from 'node:zlib';

const ESB = process.env.ESBUILD_DIR || '/tmp/esbuild-tool';
const { build, version } = createRequire(`${ESB}/`)('esbuild');
console.log(`esbuild ${version}`);
const out = await build({
  entryPoints: ['emit-js-blobs.page.entry.mjs'],
  bundle: true,
  write: false,
  format: 'iife',
  platform: 'browser',
  target: 'es2020',
  minify: true,
  legalComments: 'none',
  loader: { '.json': 'text', '.txt': 'text' },
  // vendored libs import node:crypto only on node-specific paths; the browser build must not need them
  logLevel: 'warning',
});
const js = out.outputFiles[0].text;
const tpl = fs.readFileSync('emit-js-blobs.template.html', 'utf8');
// "</script" inside the bundle would end the tag early
const safe = js.replace(/<\/script/gi, '<\\/script');
const html = tpl.replace('/*BUNDLE*/', () => safe);
fs.mkdirSync('dist', { recursive: true });
fs.writeFileSync('dist/emit-js-blobs.html', html);
const bytes = Buffer.byteLength(html);
console.log(JSON.stringify({ file: 'dist/emit-js-blobs.html', bytes, mib: +(bytes / 1048576).toFixed(3), gzip_bytes: zlib.gzipSync(html).length }));
