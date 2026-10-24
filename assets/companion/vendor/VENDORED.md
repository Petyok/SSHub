# Vendored JS libraries — PoC A

Downloaded from the npm registry on 2026-09-21 (`npm pack <name>@<version>`,
packages extracted verbatim; nothing edited). All licences MIT.

| package | version | licence | provenance | sha256 of tarball |
|---|---|---|---|---|
| @noble/post-quantum | 0.7.1 | MIT | paulmillr noble family; independently audited by Quantankrypto (Aug 2026, 2 rounds); FIPS 203 final ML-KEM | e98947485ff33c322ce75f8ae32cf7041a510dca9b8135f10475c8c10c956bab |
| @noble/hashes | 2.4.0 | MIT | paulmillr noble family; Cure53-audited (Sep 2024); BLAKE3 (incl. deriveKey) | e1946149b780017b2564fcc092cb01c04e1d7f20627d0296c17da8868f4436dc |
| @noble/curves | 2.4.0 | MIT | paulmillr noble family; Cure53-audited; ed25519 + x25519 | 55279c71b0201d6c0e58d0206d15825359e02a3932f326ba7b7909ac2d334aa9 |
| @noble/ciphers | 2.4.0 | MIT | paulmillr noble family; Cure53-audited; RFC 8439 XChaCha20-Poly1305 + raw hchacha/chacha20orig/poly1305 | 4a398bcc280c8742d474f9162f6d3c153e52ab1078e8ab592b4720410fb3c3e9 |
| @scure/bip39 | 2.4.0 | MIT | paulmillr scure family; BIP39 (2048-word english list) | 1bb104db8c58b59902096deae953c86c920b1da3dd8dcad3ba34c2fea14435f8 |
| hash-wasm | 4.12.0 | MIT | Daninet; hand-tuned WASM, Argon2id v1.3; WASM inlined as base64 in dist/index.esm.js (no external .wasm fetch) | 1db32a125fb46177932ec8ac438d3cd8214ebdfaccb5d6611b657d88eb586f92 |

Sizes on disk (whole packages, incl. .d.ts and UMD builds — the single-file
build inlines only what the page imports):

```
2.1M  hash-wasm-4.12.0
1.7M  noble-curves-2.4.0
812K  noble-hashes-2.4.0
796K  noble-post-quantum-0.7.1
612K  noble-ciphers-2.4.0
292K  scure-bip39-2.4.0
```

Notes:
- @noble/post-quantum 0.7.1 depends on @noble/{curves,hashes,ciphers}@2.4.0 —
  exactly the versions vendored here.
- @scure/bip39 2.4.0 depends on @noble/hashes@2.4.0 — same version vendored.
- hash-wasm has zero runtime deps; its ESM build (dist/index.esm.js) embeds the
  WASM binaries, which is what makes the one-file page possible.
- ML-KEM-768 entry points: `vendor/noble-post-quantum-0.7.1/ml-kem.js` exports
  `ml_kem768` with `keygen(seed64)` (seed = d‖z), `encapsulate(ek, m?)`,
  `decapsulate(ct, dk)`; decapsulate performs FIPS-203 implicit rejection and
  never throws on a well-formed (ct, dk) pair.
- EFF-7776 wordlist is NOT vendored as a package: it is read from
  `poc/crypto-interop/wordlists/eff_large.txt` (7776 words) and inlined by the
  build script.

## Added for the phone page (stage 5 UI)

Copied unmodified from `poc/vendor/` (fetched there with `npm pack`, tarball hashes in `poc/vendor/VENDORED.md`).
These two have no `package.json` here, so `scripts/link-vendor.mjs` ignores them; the sources import them by path.

| file | package | version | licence | sha256 of the file |
|---|---|---|---|---|
| `qrcode-generator-1.4.4/qrcode.cjs` (renamed from qrcode.js: package.json says type module, the UMD needs .cjs) | qrcode-generator | 1.4.4 | MIT (header in file) | 18ae399f81182bc9de916e9c77b195df20cc58d6f2d55a62b085a299f1bf1780 |
| `jsqr-1.4.0/jsQR.cjs` | jsqr | 1.4.0 | Apache-2.0 (`jsqr-1.4.0/LICENSE-jsqr`) | bc40c8a15196236b2314db0856f72ca0b49980cd5413b8c852a7349f5fee0859 |
