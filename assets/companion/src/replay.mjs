// Vector replay shared by the node test (interop.test.js) and the browser page
// (emit-js-blobs.page.entry.mjs). Pure ES module: no node built-ins, so it
// bundles for a browser unchanged.
import * as lib from './lib.mjs';

const bytes = lib.bytesOf;
const hx = lib.hexOf;
const eqHex = (a, b, what) => {
  if (a !== b) throw new Error(`MISMATCH ${what}\n  js  = ${a}\n  rust= ${b}`);
};
const assert = {
  equal(a, b, msg) {
    if (a !== b) throw new Error(`MISMATCH ${msg}: ${a} !== ${b}`);
  },
  deepEqual(a, b, msg) {
    if (hx(a) !== hx(b)) throw new Error(`MISMATCH ${msg}`);
  },
};
const h2b = (x) => (typeof x === 'string' ? bytes(x) : x);
const opIn = (op) => ({ ...op, prev_hash: h2b(op.prev_hash), signature: h2b(op.signature) });
const checkIdentity = (id, o, vid) => {
  eqHex(hx(id.edPub), o.ed25519_pub, `${vid} ed25519_pub`);
  eqHex(hx(id.mlkemPub), o.mlkem_pub, `${vid} mlkem_pub`);
  return eqHex(hx(id.xPub), o.x25519_pub, `${vid} x25519_pub`);
};

// Replays every vector in vectors.json text. Returns one record per vector:
// { id, primitive, profile, ok, ms, error }. Never throws for a failing vector.
export async function replayVectors(vectorsText, effWordsText, opts = {}) {
  const EFF_WORDS = effWordsText.split('\n').map((s) => s.trim()).filter(Boolean);
  const vectors = lib.parseJSONExact(vectorsText).vectors;

  const run = async (v) => {
    const p = v.params ?? {};
    const i = v.inputs ?? {};
    const o = v.outputs ?? {};
    switch (v.primitive) {
      case 'chain_hash':
        return eqHex(hx(lib.chainHash(h2b(i.bytes))), o.hash, v.id);
      case 'derive_key':
        return eqHex(hx(lib.deriveKey(p.context, h2b(i.ikm))), o.key, v.id);
      case 'ed25519_sign': {
        const seed = lib.deriveKey(p.context, h2b(i.ikm));
        eqHex(hx(lib.edPubOfSeed(seed)), o.pub, `${v.id} pub`);
        return eqHex(hx(lib.edSign(seed, h2b(i.msg))), o.sig, `${v.id} sig`);
      }
      case 'ed25519_verify': {
        let ok;
        try { ok = lib.edVerify(h2b(i.pub), h2b(i.msg), h2b(i.sig)); } catch { ok = false; }
        return assert.equal(ok, String(o.valid).toLowerCase() === 'true', v.id);
      }
      case 'x25519_dh':
        eqHex(hx(lib.x25519Pub(h2b(i.a_sec))), o.a_pub, `${v.id} a_pub`);
        return eqHex(hx(lib.x25519DH(h2b(i.a_sec), h2b(i.b_pub))), o.shared, `${v.id} shared`);
      case 'mlkem_encaps': {
        const enc = lib.mlkemEncapsulate(h2b(i.ek), h2b(p.m));
        eqHex(hx(enc.cipherText), o.ct, `${v.id} ct`);
        return eqHex(hx(enc.sharedSecret), o.ss, `${v.id} ss`);
      }
      case 'mlkem_decaps':
        return eqHex(hx(lib.mlkemDecapsulate(h2b(i.ct), h2b(i.dk))), o.ss, `${v.id}`);
      case 'dek_seal':
        return eqHex(hx(lib.dekSeal(h2b(i.dek), h2b(i.plaintext), h2b(p.nonce))), o.ct, v.id);
      case 'argon2id':
        return eqHex(hx(await lib.argon2idDerive(i.input, h2b(i.salt), { mem_kib: p.mem_kib, iters: p.iters })), o.key, v.id);
      case 'sas_words':
        if (lib.sasWords(h2b(i.transcript_hash)) !== o.words) throw new Error(`MISMATCH ${v.id}`);
        return;
      case 'bip39':
        eqHex(lib.bip39Encode(h2b(i.entropy)), o.phrase, `${v.id} phrase`);
        if (o.decode_ok !== undefined && o.decode_ok) lib.bip39Decode(o.phrase);
        return;
      case 'eff7776':
        return eqHex(lib.effEncode(EFF_WORDS, h2b(i.entropy)), o.phrase, v.id);
      case 'shamir_split': {
        const rng = lib.chachaRngStream(h2b(p.rng_seed), 32 * Math.max(0, (p.k | 0) - 1));
        const indices = p.indices ?? Array.from({ length: p.n }, (_, idx) => idx + 1);
        const shares = lib.splitAtIndicesSeeded(h2b(i.secret), p.k, indices, rng);
        const rustShares = o.shares.map((s) => `${String(s.index).padStart(2, '0')}${s.bytes_hex}`);
        eqHex(shares.map((s) => `${String(s.index).padStart(2, '0')}${hx(s.bytes)}`).join(''), rustShares.join(''), `${v.id} shares`);
        return;
      }
      case 'shamir_combine':
        return eqHex(hx(lib.shamirCombine(i.shares.map((s) => ({ index: s.index, bytes: bytes(s.bytes_hex) })))), o.secret, v.id);
      case 'reshare': {
        const rng = lib.chachaRngStream(h2b(p.rng_seed), 32 * Math.max(0, (p.k | 0) - 1));
        const c = lib.reshareContribute({ index: i.share.index, bytes: bytes(i.share.bytes_hex) }, p.new_indices, p.k, rng);
        eqHex(String(c.from), String(o.from), `${v.id} from`);
        return eqHex(
          c.sub_shares.map(([x, b]) => `${String(x).padStart(2, '0')}${hx(b)}`).join(''),
          o.sub_shares.map((s) => `${String(s.index).padStart(2, '0')}${s.bytes_hex}`).join(''),
          `${v.id} sub_shares`,
        );
      }
      case 'op_canonical':
        eqHex(hx(lib.opCanonicalBytes(opIn(i.op))), o.canonical_hex, v.id);
        // the canonical text of a Rust-produced op must also survive a parse and re-serialise round trip
        return eqHex(lib.canonicalJSON(lib.parseJSONExact(new TextDecoder().decode(bytes(o.canonical_hex)))), new TextDecoder().decode(bytes(o.canonical_hex)), `${v.id} reparse`);
      case 'op_hash':
        return eqHex(hx(lib.opHash(opIn(i.op))), o.hash, v.id);
      case 'op_verify':
        return assert.equal(lib.opVerify(opIn(i.op), h2b(i.pub)), o.valid, v.id);
      case 'binding_hash': {
        const pre = lib.bindingHashPreimage(opIn(i.op), h2b(i.nonce), i.expiry, h2b(i.eph), i.request_id);
        eqHex(hx(pre), o.preimage, `${v.id} preimage`);
        return eqHex(hx(lib.bindingHash(opIn(i.op), h2b(i.nonce), i.expiry, h2b(i.eph), i.request_id)), o.binding_hash, v.id);
      }
      case 'hybrid_wrap': {
        const m = p.m !== undefined ? h2b(p.m) : lib.chachaRngStream(h2b(p.rng_seed), 32);
        const blob = lib.hybridWrap({ x25519_pub: h2b(i.recipient.x25519_pub), mlkem_pub: h2b(i.recipient.mlkem_pub) }, h2b(i.plaintext), h2b(p.eph_secret), h2b(p.nonce), m);
        return eqHex(hx(blob), o.blob, v.id);
      }
      case 'sealed_box': {
        const rid = lib.identityFromIkm(h2b(i.recipient_ikm), p.recipient_context);
        eqHex(hx(rid.xPub), i.recipient_x25519_pub, `${v.id} recipient pub`);
        const sealed = lib.sealedBoxSeal(h2b(i.recipient_x25519_pub), h2b(i.plaintext), h2b(p.eph_secret), h2b(p.nonce));
        eqHex(hx(sealed), o.sealed, `${v.id} seal`);
        return assert.deepEqual(lib.sealedBoxOpen(rid.xSecret, sealed), h2b(i.plaintext), `${v.id} open`);
      }
      case 'signed_approval': {
        const preimage = lib.approvalPreimage(opIn(i.op), h2b(i.nonce), i.expiry, h2b(i.eph), i.request_id);
        eqHex(hx(preimage), o.preimage, `${v.id} preimage`);
        eqHex(hx(lib.approvalSign(h2b(p.holder_seed ?? i.holder_seed), opIn(i.op), h2b(i.nonce), i.expiry, h2b(i.eph), i.request_id)), o.sig, `${v.id} sig`);
        return assert.equal(lib.approvalVerify(h2b(i.holder_pub), opIn(i.op), h2b(i.nonce), i.expiry, h2b(i.eph), i.request_id, h2b(o.sig)), true, `${v.id} verify`);
      }
      case 'derive_xof':
        return eqHex(hx(lib.deriveXof(p.context, h2b(i.ikm), p.out_len)), o.out, v.id);
      case 'identity_from_seed':
        return checkIdentity(lib.identityFromIkm(h2b(i.ikm), p.context), o, v.id);
      case 'paper_derive':
        return checkIdentity(lib.paperDerive(i.phrase, p.domain === 'recover' ? lib.CONTEXTS.paperRecover : lib.CONTEXTS.paperUnwrap), o, v.id);
      case 'vault_recipient':
        return checkIdentity(await lib.vaultRecipient(i.phrase, h2b(i.ring_secret), h2b(i.salt), { mem_kib: p.mem_kib, iters: p.iters }), o, v.id);
      case 'key_file': {
        const id = lib.identityFromIkm(h2b(i.ikm), p.context);
        return eqHex(hx(await lib.keyFileSeal(id, i.passphrase, p.profile, h2b(p.salt), h2b(p.nonce))), o.blob, v.id);
      }
      case 'hybrid_unwrap_reject': {
        const id = lib.identityFromIkm(h2b(i.recipient_ikm), p.recipient_context);
        let opens = true;
        try {
          lib.hybridUnwrap(id.xSecret, id.mlkemDk, h2b(i.blob));
        } catch {
          opens = false;
        }
        return assert.equal(String(opens), String(o.opens), `${v.id} opens`);
      }
      case 'share_response':
        return; // A2 direction: JS produces, Rust opens - nothing to replay
      default:
        throw new Error(`unknown primitive ${v.primitive} (vector ${v.id})`);
    }
  };

  const out = [];
  for (const v of vectors) {
    if (opts.skipProfiles && v.params && opts.skipProfiles.includes(v.params.profile)) {
      out.push({ id: v.id, primitive: v.primitive, profile: v.params.profile, ok: true, skipped: true, ms: 0, error: null });
      continue;
    }
    const t0 = performance.now();
    let ok = true;
    let error = null;
    try {
      await run(v);
    } catch (e) {
      ok = false;
      error = String(e.message || e).split('\n')[0];
    }
    out.push({ id: v.id, primitive: v.primitive, profile: v.params && v.params.profile ? v.params.profile : null, ok, ms: Math.round(performance.now() - t0), error });
  }
  return out;
}
