# Stage 5 harness: phone companion, replay results

Status: PARTIAL. Replay gate not green (1 vector FAIL). Build and browser run not done.

## Claims

| Claim | Result | Evidence |
|---|---|---|
| Crypto vectors reproduced (`tests/vectors/crypto/vectors.json`) | 57 of 58 | `node --test test/replay.test.js`, host and gpu-serv (Node 22.23.4) |
| Failing vector | `key-file`: MISMATCH | plaintext is 2465 bytes, contract section 1 defines 2464 (seed 32 + x25519 32 + ML-KEM dk 2400); extra trailing `0x00` |
| Op-rules reproduced (`tests/vectors/op-rules/ops.json`) | 13 of 13 | same run |
| Unreproduced vectors | skipped: 0 | no `skipProfiles` in gate runs |
| Single-file page build | NOT RUN | needs bundler decision (esbuild not vendored, not in gpu-serv image) |
| Headless Chromium run from `http://localhost` | NOT RUN | depends on build |
| External requests during run | NOT MEASURED | depends on browser run |
| Page size, cold start | NOT MEASURED | depends on build |

## Corrections

- The earlier "0 mismatches" statement was wrong. The count assertion threw before the failure list was checked, and the label is hard-coded text. The count is now derived from the vector files.
- The Shamir handler ignored `params.indices`. Fixed in `a0f88f8`.
- The SAS handler was missing. Added as `lib.sasWords` (four 11-bit BIP-39 indices of the transcript hash, hyphen-joined). All four SAS vectors match.

## Open decisions

1. `key-file` layout: contract section 1 (2464 bytes) or vector (2465 bytes with a trailing `0x00`).
2. Bundler for `sshub-companion.html`: vendor esbuild, or install `esbuild@0.25.12` in the build step only.

## Commands

- Host: `cd assets/companion && npm test`
- gpu-serv: `gpu-verify/tools.sh <worktree> <name> -- 'cd /work/assets/companion && node scripts/link-vendor.mjs && node --test test/replay.test.js'`
  - Logs: `gpu-verify/logs/companion-suite-a0f88f8c-20261024T142035.log` (58 vectors, 1 fail)

## Not run

- Single-file build, headless Chromium run, external request count, page size, cold start.
