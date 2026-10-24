# Stage 5 harness: phone companion, replay results

Status: PASS for the crypto replay gate, single-file build, headless run and external request count. Op vectors not run (see Not run).

## Claims

| Claim | Result | Evidence |
|---|---|---|
| Crypto vectors reproduced (`tests/vectors/crypto/vectors.json`) | 58 of 58, 0 mismatches, 0 skipped | gpu-serv container, Node 22.23.4: `node --test test/replay.test.js`, pass 1 fail 0 (duration 2.9 s). Log `gpu-verify/logs/companion-stage5-20261024T142736.log` |
| Key file vector (`key-file`) | reproduced | layout per contract (ed25519 seed, current x25519, current ML-KEM dk, `n`, older generations), commit `bc3d302` |
| Node suite on host (before build) | 58 of 58 | `cd assets/companion && node --test test/replay.test.js`, pass 1 fail 0 (16.6 s, host Node 24) |
| Single-file build reproducible | yes | built twice in the container, `cmp` identical; sha256 `4b83fd27fee6c6a30ed86e8546090f175d8ae0dea0801ae021e96dceda84ea71` |
| Build output committed | `assets/companion/sshub-companion.html` | fetched from gpu-serv, sha256 matches the container build |
| Headless Chromium run from `http://localhost` | 58 of 58 pass, `error: null` | Playwright 1.64.1, Chromium from the tools image, fresh browser, page served by `scripts/headless-check.mjs` |
| External requests during run | 0 | request interception: 1 request total (the page itself), 0 leaving the origin; CSP `default-src 'none'; connect-src 'none'` |
| Page size | 297,899 bytes raw, 106,634 bytes gzip -9 | `sshub-companion.html` |
| Cold start | 4,228 ms | fresh Chromium launch to `window.__done`, including the full replay (in-page replay 3,307 ms) |

## Corrections

- The earlier "0 mismatches" statement was wrong. The count assertion threw before the failure list was checked, and the label was hard-coded. The count now comes from the vector file.
- The Shamir handler ignored `params.indices`. Fixed in `a0f88f8`.
- The SAS handler was missing. Added as `lib.sasWords` (four 11-bit BIP-39 indices of the transcript hash, hyphen-joined).
- The earlier 57 of 58 run (log `companion-suite-a0f88f8c-20261024T142035.log`) is stale. Its only failure, `key-file`, was a layout error, now fixed.
- The earlier op-rules 13 of 13 count is withdrawn: `tests/vectors/op-rules/ops.json` was deleted by the contract owner as contradicting the contract. The replay and page no longer reference it.

## Build notes

- The build bundles with esbuild, pinned exactly in `devDependencies` (`0.25.12`) with `package-lock.json` integrity hashes. esbuild is installed with `npm ci` and run only in the tools container. It is not vendored and not shipped.
- The PoC A inliner (`poc/crypto`, `poc/crypto-interop/build-single-file.mjs`) also bundles with esbuild 0.25.12, so it is not bundler-free. The companion graph (bare `@noble/*`, `@scure/bip39`, ESM vendor files) needs the same step.
- `link-vendor.mjs` replaces only the vendor links it owns, so an npm-installed esbuild survives in `node_modules`.
- Vendored deps are symlinked into `node_modules` (`scripts/link-vendor.mjs`) because the npm registry install of vendor deps fails offline.
- `hash-wasm` prints a `MODULE_TYPELESS_PACKAGE_JSON` warning under Node. Harmless, no action taken.

## Commands

- Host: `cd assets/companion && node scripts/link-vendor.mjs && node --test test/replay.test.js`
- gpu-serv build, suite and headless run (one run):
  `gpu-verify/tools.sh <worktree> companion-stage5 -- 'cd /work/assets/companion && npm ci --no-audit --no-fund && node scripts/link-vendor.mjs && node build.mjs && cp sshub-companion.html /tmp/b1.html && node build.mjs && cmp sshub-companion.html /tmp/b1.html && node --test test/replay.test.js; NODE_PATH=/opt/node/lib/node_modules node scripts/headless-check.mjs'`
  - Log: `gpu-verify/logs/companion-stage5-20261024T142736.log`
  - Headless result line: `{"bytes":297899,"cold_start_ms":4228,"requests":1,"external_requests":0,"result":{"crypto":{"total":58,"expected":58,"failures":[]},"pass":true},"error":null}`

## Not run

- Op vectors: the old `tests/vectors/op-rules/ops.json` is gone. The contract owner's op vectors (`tests/vectors/op/`, stage 1) are not in this branch yet. Discrepancy to fix: `feature/host-sync` tip still tracks `tests/vectors/op-rules/ops.json`; removal belongs to the contract owner's commit.
- Argon2id: the four `argon2id` vectors run in the same suite with the profiles listed in `vectors.json`. No separate run was made for the 1 GiB MASTER profile, and this report does not claim it.
- Browser replay of a real phone (no device test) and of any Safari/Firefox engine: not run; only headless Chromium.
