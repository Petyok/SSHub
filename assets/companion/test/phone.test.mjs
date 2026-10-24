// Node-side tests of the phone modules, driven by bytes the Rust code produced
// (test/fixtures/*.json, regenerated with SSHUB_EMIT_PHONE_FIXTURES=1 cargo test sync::phone_fixtures)
// and by the op vectors of the op-log package (test/fixtures/ops.vectors.json).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { blake3 } from '../vendor/noble-hashes-2.4.0/blake3.js';
import { bytesOf, concat, edSign, edVerify, hexOf, identityFromIkm, sasWords, sealedBoxOpen, x25519Pub } from '../src/lib.mjs';
import { FountainDecoder, snapshotOpen } from '../src/quorum.mjs';
import { describeOp, encodeHeadAttest, encodePhoneReenrollCancel, parseOp, safeText } from '../src/ops.mjs';
import {
  KIND, commitOf, confirmMessage, decodeEnroll, decodeRecoveryNotice, encodeEnrollCommit, encodeEnrollConfirm, encodeEnrollReveal, encodePhoneOps,
  parseReplyPlain, transcriptHash, parseReveal, revealBytes,
} from '../src/frames.mjs';
import { generateSecret, identityOfSecret, secretFromWords, secretToWords, unwrapSecret, wrapSecret } from '../src/keys.mjs';
import { approveRelease, evaluateChallenge } from '../src/release.mjs';
import {
  appendHeadAttest, appendReenrollCancel, courierOps, emptyState, invalidateSnapshot, issueSnapshot, liveSnapshot, memoryBackend, loadState, recordSnapshot, saveState, SNAPSHOT_TTL_SECS,
} from '../src/state.mjs';
import { audioFrame, fskEncode, reverseQrFrames, FSK, FSK_FAST } from '../src/channels.mjs';
import { DocumentCollector } from '../src/scan.mjs';
import jsQR from '../vendor/jsqr-1.4.0/jsQR.cjs';

const here = new URL('./fixtures/', import.meta.url);
const load = (n) => JSON.parse(readFileSync(new URL(n, here), 'utf8'));
const fx = load('challenges.json');
const enrollFx = load('enroll.json');
const opVectors = load('ops.vectors.json');
const h = (x) => bytesOf(x);
const TEST_PROFILE = { mem_kib: 32 * 1024, iters: 1 };
const phone = identityOfSecret(h(fx.phone.secret_hex));

// ------------------------------------------------------------ ops

test('every op vector of the op-log package parses strictly, verifies and is described without dropping a leaf', () => {
  const signer = h(opVectors.meta.signer_ed25519_pub_hex);
  assert.ok(opVectors.vectors.length >= 35);
  for (const v of opVectors.vectors) {
    const bytes = h(v.canonical_hex);
    const op = parseOp(bytes);
    assert.equal(op.kind, Object.keys(v.op.body)[0], v.id);
    assert.ok(edVerify(signer, bytes, h(v.signature_hex)), `${v.id}: signature`);
    const d = describeOp(op);
    assert.ok(d.rows.length >= 3, v.id);
    if (op.kind !== 'ShareRelease' && op.kind !== 'HeadAttest' && !/Upsert/.test(op.kind)) {
      assert.ok(d.rows.filter((r) => r.label !== 'Op signed by' && r.label !== 'Op time' && r.label !== 'Op seq / epoch').every((r) => r.danger), `${v.id}: every row of a dangerous kind is highlighted`);
    }
  }
});

test('host upsert highlights exactly the dangerous fields of C5', () => {
  const v = opVectors.vectors.find((x) => x.id === 'host-upsert-full');
  const d = describeOp(parseOp(h(v.canonical_hex)));
  const danger = Object.fromEntries(d.rows.map((r) => [r.label, r.danger]));
  for (const f of ['fields.address', 'fields.port', 'fields.username', 'fields.proxy_jump']) if (f in danger) assert.equal(danger[f], true, f);
  for (const f of ['fields.name', 'fields.label', 'fields.notes', 'fields.tags']) if (f in danger) assert.equal(danger[f], false, f);
  assert.ok(Object.keys(danger).some((k) => k === 'fields.address') && Object.keys(danger).some((k) => k === 'fields.name'));
});

test('the phone encoders reproduce the canonical bytes of the op vectors', () => {
  const find = (id) => opVectors.vectors.find((v) => v.id === id);
  // The fixture is plain JSON (u64 values rounded), so read the exact integers from the bytes.
  const exact = (id) => parseOp(h(find(id).canonical_hex));
  const hdr = (o) => ({ device_id: o.device_id, seq: o.seq, lamport: o.lamport, prev_hash: o.prev_hash, timestamp: o.timestamp, epoch: o.epoch });
  const att = exact('head-attest-u64-max');
  assert.equal(
    hexOf(encodeHeadAttest(hdr(att), { device_id: att.fields.device_id, seq: String(att.fields.seq), chain_hash: att.fields.chain_hash })),
    find('head-attest-u64-max').canonical_hex,
  );
  const c = exact('phone-reenroll-cancel-none');
  assert.equal(hexOf(encodePhoneReenrollCancel(hdr(c), c.fields.reenroll_hash)), find('phone-reenroll-cancel-none').canonical_hex);
});

test('non-canonical or hostile op bytes are refused', () => {
  const good = new TextDecoder().decode(h(opVectors.vectors.find((v) => v.id === 'share-release-u64-max').canonical_hex));
  const enc = (s) => new TextEncoder().encode(s);
  parseOp(enc(good));
  assert.throws(() => parseOp(enc(good.replace('"seq":16', '"seq": 16'))), /canonical/);
  assert.throws(() => parseOp(enc(good.replace('"requester":"723ed85f0566df0c"', '"requester":"723ed85f0566df0c","requester":"0000000000000000"'))), /canonical/);
  assert.throws(() => parseOp(enc(good.replace('"lamport":16,"prev_hash"', '"prev_hash"'))), /order|canonical/);
  assert.throws(() => parseOp(enc(good.replace('ShareRelease', 'Mystery'))), /unknown op kind/);
  assert.throws(() => parseOp(Uint8Array.of(0xff, 0xfe)), /UTF-8/);
  assert.equal(safeText('a\u202eb\u0007'), 'a\\u202eb\\u0007');
});

// ------------------------------------------------------------ keys

test('the phone identity is derived from the 24 words and matches the Rust fixture', () => {
  assert.equal(secretToWords(h(fx.phone.secret_hex)), fx.phone.words);
  assert.deepEqual(secretFromWords(fx.phone.words), h(fx.phone.secret_hex));
  assert.equal(hexOf(phone.edPub), fx.phone.pub.ed25519_pub);
  assert.equal(hexOf(phone.xPub), fx.phone.pub.x25519_pub);
  assert.equal(hexOf(phone.mlkemPub), fx.phone.pub.mlkem_pub);
  assert.equal(phone.deviceId, fx.phone.pub.device_id);
  assert.throws(() => secretFromWords(`zzzzzz ${fx.phone.words.split(' ').slice(1).join(' ')}`), /./);
});

test('the secret is wrapped under the PIN; a wrong PIN and a hostile profile fail before work', async () => {
  const s = generateSecret();
  const blob = await wrapSecret(s, 'hunter22', TEST_PROFILE);
  assert.deepEqual(await unwrapSecret(blob, 'hunter22'), s);
  await assert.rejects(unwrapSecret(blob, 'hunter23'), /wrong PIN/);
  const bad = blob.slice();
  bad[5] = 0xff;
  await assert.rejects(unwrapSecret(bad, 'hunter22'), /profile/);
  assert.notDeepEqual(blob, await wrapSecret(s, 'hunter22', TEST_PROFILE), 'fresh salt and nonce every time');
});

// ------------------------------------------------------------ enrollment frames

test('enrollment bytes equal the Rust mirror of enroll.rs', () => {
  const joining = parseReveal(h(enrollFx.joining.reveal_hex));
  const existing = parseReveal(h(enrollFx.existing.reveal_hex));
  assert.equal(hexOf(revealBytes(joining)), enrollFx.joining.reveal_hex);
  assert.equal(hexOf(commitOf(0, joining)), enrollFx.joining.commit_hex);
  assert.equal(hexOf(commitOf(1, existing)), enrollFx.existing.commit_hex);
  const th = transcriptHash(joining, existing);
  assert.equal(hexOf(th), enrollFx.transcript_hash_hex);
  assert.equal(sasWords(th), enrollFx.sas);
  assert.equal(hexOf(confirmMessage(1, true, th)), enrollFx.confirm_message_existing_ok_hex);
  assert.ok(edVerify(h(fx.requester.pub.ed25519_pub), confirmMessage(1, true, th), h(enrollFx.confirm_existing_ok_sig_hex)));
  const frames = [
    encodeEnrollCommit(1, commitOf(1, existing)), encodeEnrollCommit(0, commitOf(0, joining)), encodeEnrollReveal(1, existing), encodeEnrollReveal(0, joining),
    encodeEnrollConfirm(1, true, h(enrollFx.confirm_existing_ok_sig_hex)),
  ];
  assert.deepEqual(frames.map(hexOf), enrollFx.frames_hex);
  for (const f of enrollFx.frames_hex) decodeEnroll(h(f));
  assert.throws(() => decodeEnroll(h(enrollFx.frames_hex[0]).slice(0, 20)), /./);
});

// ------------------------------------------------------------ release

const requesterPub = fx.requester.pub;
const ringWithRequester = { [requesterPub.device_id]: { name: fx.requester.name, ed25519_pub: requesterPub.ed25519_pub } };
const ctx = (over = {}) => ({ now: fx.now, identity: phone, ring: ringWithRequester, aliases: {}, ...over });

for (const ch of fx.challenges) {
  test(`challenge "${ch.name}": ${ch.expect}`, () => {
    const frame = h(ch.frame_hex);
    const ev = evaluateChallenge(frame, ctx(ch.name === 'unknown-requester' ? { ring: {} } : {}));
    if (ch.expect === 'ok') {
      assert.equal(ev.ok, true, JSON.stringify(ev.checks));
      assert.equal(ev.bindingHash, ch.binding_hash_hex, 'binding hash equals the Rust one');
      assert.equal(ev.bindingWords, ch.binding_words);
      assert.equal(hexOf(ev.share.slice(1)), fx.phone_share.bytes_hex);
      assert.equal(ev.share[0], fx.phone_share.index);
      assert.equal(ev.warnings.length, ch.name === 'unknown-requester' ? 1 : 0);
      const d = ev.description;
      assert.match(d.headline, new RegExp(ev.requester));
      const req = d.rows.find((r) => r.label === 'Requesting device');
      assert.equal(req.danger, true);
      assert.ok(ch.name === 'unknown-requester' || req.value.includes(fx.requester.name), 'the requesting device is named');
      assert.equal(d.rows.find((r) => r.label === 'Host credential').value, fx.host_uid);
    } else {
      assert.equal(ev.ok, false);
      assert.ok(ev.refusal.includes(ch.expect), `${ch.name}: "${ev.refusal}" should mention "${ch.expect}"`);
    }
  });
}

test('an unenrolled phone cannot catch a bad signature (it only warns), an enrolled one refuses', () => {
  const bad = fx.challenges.find((c) => c.name === 'bad-signature');
  const ev = evaluateChallenge(h(bad.frame_hex), ctx({ ring: {} }));
  assert.equal(ev.ok, true);
  assert.equal(ev.warnings.length, 1);
});

test('approve: the reply opens with the ephemeral key, carries the courier, and the chain continues', async () => {
  const good = fx.challenges.find((c) => c.name === 'good');
  const state = emptyState();
  const run = async (snapshot) => {
    const ev = evaluateChallenge(h(good.frame_hex), ctx());
    assert.equal(ev.ok, true);
    const out = approveRelease(ev, { identity: phone, state, now: fx.now + 3, snapshot });
    return { ev, out };
  };
  const { out } = await run(null);
  const resp = out.full;
  assert.deepEqual([...resp.slice(0, 7)], [0x53, 0x57, 0x52, 0x31, 0, 1, KIND.RESPONSE]);
  const len = Number(new DataView(resp.buffer, resp.byteOffset + 7 + 8, 8).getBigUint64(0));
  assert.equal(resp.length, 7 + 16 + len, 'exact ShareResponseWire');
  const plain = sealedBoxOpen(h(good.eph_secret_hex), resp.slice(23));
  const { share, ext } = parseReplyPlain(plain);
  assert.equal(hexOf(share.slice(1)), fx.phone_share.bytes_hex);
  assert.ok(edVerify(phone.edPub, h(good.binding_preimage_hex), ext.approvalSig), 'approval signature over the binding preimage');
  assert.equal(ext.ops.length, 1);
  assert.ok(edVerify(phone.edPub, ext.ops[0].canonical, ext.ops[0].signature));
  const op = parseOp(ext.ops[0].canonical);
  assert.equal(op.kind, 'HeadAttest');
  assert.equal(op.device_id, phone.deviceId);
  // the audio reply is share-only
  const short = parseReplyPlain(sealedBoxOpen(h(good.eph_secret_hex), out.short.slice(23)));
  assert.equal(short.ext, null);
  assert.equal(out.manual.index, fx.phone_share.index);
  // second approval: seq 2 continues from the first op
  await run(null);
  const ops = courierOps(state).map((o) => parseOp(o.canonical));
  assert.deepEqual(ops.map((o) => Number(o.seq)), [1, 2]);
  assert.equal(ops[1].prev_hash, hexOf(blake3(courierOps(state)[0].canonical)));
  assert.ok(ops[1].lamport > ops[0].lamport);
});

test('a short code is issued once per credential, opens with its code, and expires after 7 days', async () => {
  const state = emptyState();
  const host = fx.host_uid;
  const share = h(fx.phone_share.bytes_hex);
  const issued = await issueSnapshot(state, host, fx.phone_share.index, share, fx.now, { profiles: [TEST_PROFILE] });
  assert.match(issued.code, /^[0-9A-HJKMNP-TV-Z]{4}(-[0-9A-HJKMNP-TV-Z]{4}){3}$/);
  const opened = await snapshotOpen(issued.blob, issued.code, fx.now + 5);
  assert.equal(opened.ok, true);
  assert.equal(hexOf(opened.share_bytes), fx.phone_share.bytes_hex);
  recordSnapshot(state, host, issued.record);
  assert.equal(issued.record.expires_at, fx.now + SNAPSHOT_TTL_SECS);
  await assert.rejects(issueSnapshot(state, host, 2, share, fx.now + 60, { profiles: [TEST_PROFILE] }), /already live/);
  assert.ok(liveSnapshot(state, host, fx.now + 60));
  // another credential is independent
  const other = await issueSnapshot(state, fx.other_host_uid, 2, share, fx.now, { profiles: [TEST_PROFILE] });
  assert.ok(other.blob);
  // expiry frees the slot; invalidation does too
  assert.equal(liveSnapshot(state, host, fx.now + SNAPSHOT_TTL_SECS + 1), null);
  await issueSnapshot(state, host, 2, share, fx.now + SNAPSHOT_TTL_SECS + 1, { profiles: [TEST_PROFILE] });
  invalidateSnapshot(state, host);
  assert.equal(liveSnapshot(state, host, fx.now + 60), null);
});

test('state survives a save and a load', async () => {
  const kv = memoryBackend();
  const s = emptyState();
  appendHeadAttest(s, phone, fx.now, { lamport: 9, epoch: 0 });
  s.ring.abcd = { name: 'x', ed25519_pub: '00' };
  await saveState(kv, s);
  assert.deepEqual(await loadState(kv), s);
});

// ------------------------------------------------------------ recovery

test('a recovery notice is cancelled by the phone with its own signed op', () => {
  const n = fx.recovery_notice;
  const notice = decodeRecoveryNotice(h(n.frame_hex));
  const op = parseOp(notice.op_canonical);
  assert.equal(op.kind, 'PhoneReenroll');
  assert.equal(hexOf(blake3(notice.op_canonical)), n.op_hash_hex);
  const state = emptyState();
  const cancel = appendReenrollCancel(state, phone, n.op_hash_hex, fx.now, { lamport: op.lamport, epoch: op.epoch });
  const c = parseOp(cancel.canonical);
  assert.equal(c.kind, 'PhoneReenrollCancel');
  assert.deepEqual(c.fields, { reenroll_hash: n.op_hash_hex, by_phone: true, third_factor: null });
  assert.ok(edVerify(phone.edPub, cancel.canonical, cancel.signature));
  assert.ok(c.lamport > op.lamport);
  const frame = encodePhoneOps(courierOps(state));
  assert.equal(frame[6], KIND.PHONE_OPS);
});

// ------------------------------------------------------------ media

function collect(frames, order) {
  const col = new DocumentCollector();
  let doc = null;
  for (const i of order) doc = col.push(frames[i]) || doc;
  return doc;
}

test('animated frames made by Rust reassemble through the page collector, with losses repaired by parity', () => {
  for (const ch of fx.challenges.slice(0, 3)) {
    const frames = ch.qr_frames_hex.map(h);
    const all = frames.map((_, i) => i);
    assert.equal(hexOf(collect(frames, all)), ch.frame_hex);
    void frames;
  }
  const ch = fx.challenges[0];
  const frames = ch.qr_frames_hex.map(h);
  const missing = 3;
  const order = frames.map((_, i) => i).filter((i) => i !== missing);
  assert.equal(hexOf(collect(frames, order)), ch.frame_hex, 'one lost data frame is repaired from the parity frame');
  assert.equal(collect(frames, [0, 1]), null, 'a partial lap gives nothing');
});

function pngLess(qr) {
  const n = qr.getModuleCount();
  const scale = 4;
  const quiet = 4;
  const w = (n + 2 * quiet) * scale;
  const data = new Uint8ClampedArray(w * w * 4).fill(255);
  for (let y = 0; y < w; y++) {
    for (let x = 0; x < w; x++) {
      const r = Math.floor(y / scale) - quiet;
      const c = Math.floor(x / scale) - quiet;
      if (r >= 0 && c >= 0 && r < n && c < n && qr.isDark(r, c)) {
        const i = (y * w + x) * 4;
        data[i] = data[i + 1] = data[i + 2] = 0;
      }
    }
  }
  return { data, w };
}

test('the reverse QR of a response decodes with jsQR and reassembles to the exact frame', () => {
  const good = fx.challenges.find((c) => c.name === 'good');
  const state = emptyState();
  const ev = evaluateChallenge(h(good.frame_hex), ctx());
  const out = approveRelease(ev, { identity: phone, state, now: fx.now, snapshot: null });
  const qrs = reverseQrFrames(out.full);
  const col = new DocumentCollector();
  let doc = null;
  for (const qr of qrs) {
    const { data, w } = pngLess(qr);
    const r = jsQR(data, w, w);
    assert.ok(r, 'jsQR reads the frame');
    doc = col.push(Uint8Array.from(r.binaryData)) || doc;
  }
  assert.deepEqual(doc, out.full);
});

function loadShareLib() {
  const ctx2 = { console };
  ctx2.window = ctx2;
  vm.createContext(ctx2);
  vm.runInContext(readFileSync(new URL('../poc/share-lib.js', import.meta.url), 'utf8'), ctx2, { filename: 'share-lib.js' });
  return ctx2.ShareLib;
}

test('the audio reply decodes through the PoC H receive decoder, both profiles', () => {
  const L = loadShareLib();
  const good = fx.challenges.find((c) => c.name === 'good');
  const state = emptyState();
  const ev = evaluateChallenge(h(good.frame_hex), ctx());
  const out = approveRelease(ev, { identity: phone, state, now: fx.now, snapshot: null });
  for (const [name, F, ref] of [['fast', FSK_FAST, L.FSK_FAST], ['slow', FSK, L.FSK]]) {
    const ours = fskEncode(audioFrame(out.short), F);
    const reference = L.fskEncode(L.frame(out.short), ref);
    assert.equal(ours.length, reference.length, `${name}: same burst length`);
    let maxDiff = 0;
    for (let i = 0; i < ours.length; i++) maxDiff = Math.max(maxDiff, Math.abs(ours[i] - reference[i]));
    assert.ok(maxDiff < 1e-6, `${name}: identical samples (${maxDiff})`);
    // one second of silence in front, as the fake microphone loop has
    const padded = new Float32Array(48000 + ours.length + 96000);
    padded.set(ours, 48000);
    const d = L.fskDecode(padded, ref);
    assert.equal(d.ok, true, `${name}: ${d.error}`);
    assert.equal(hexOf(Uint8Array.from(d.payload)), hexOf(out.short));
  }
  assert.ok(fskEncode(audioFrame(out.short), FSK_FAST).length / 48000 < 9, 'fast burst under 9 s for the 128 byte reply');
});

test('x25519 pubkey helper agrees with the fixture ephemeral key', () => {
  assert.equal(hexOf(x25519Pub(h(fx.eph.secret_hex))), fx.eph.pub_hex);
  const id = identityFromIkm(h(fx.requester.ikm_hex), fx.requester.context);
  assert.equal(hexOf(id.edPub), requesterPub.ed25519_pub);
  void concat;
  void edSign;
  void FountainDecoder;
});
