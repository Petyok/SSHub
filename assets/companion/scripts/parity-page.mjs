// Parity scenario, page side. The SAME deterministic scenario runs through the page modules here and through the Rust
// phone core (crates/sshub-phone-core/tests/parity.rs); both must produce byte-identical results. This script prints the
// page's results as JSON; `--write` stores them at test/fixtures/parity-page.json, `--check` compares a fresh run with the
// committed file (so the page cannot drift). Run in the tools container (node 22), never on the laptop:
//   node scripts/parity-page.mjs --check
// Randomness never reaches the output: sealed boxes are compared after opening them, short codes are not issued, and
// every other input is fixed.
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { bytesOf, hexOf, sealedBoxOpen } from '../src/lib.mjs';
import { xchacha20poly1305 } from '../vendor/noble-ciphers-2.4.0/chacha.js';
import { blake3 } from '../vendor/noble-hashes-2.4.0/blake3.js';
import { identityOfSecret } from '../src/keys.mjs';
import { describeOp, isRedOp, opHashHex, parseOp } from '../src/ops.mjs';
import {
  appendCertExtend, appendCertIssue, appendCertRevoke, appendConfirmBatch, appendHeadAttest, appendReenrollCancel, appendRedPhoneSign, appendShareStorageKey,
  appendWrapKeyRotate, courierOps, emptyState,
} from '../src/state.mjs';
import { approveRelease, evaluateChallenge } from '../src/release.mjs';
import { certRequestProblems, describeCertRequest, describeTerm, issuedCerts } from '../src/cert.mjs';
import { envelopeAad, ownStorageKey, planReshare, publishReshare } from '../src/reshare.mjs';
import { commitOf, confirmMessage, encodeEnrollCommit, encodeEnrollConfirm, encodeEnrollReveal, transcriptHash } from '../src/frames.mjs';
import { fountainEncode } from '../src/quorum.mjs';
import { sasWords } from '../src/lib.mjs';
import { audioFrame, fskEncode, AUDIO_PROFILES } from '../src/channels.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const fixtures = path.resolve(here, '..', 'test', 'fixtures');
const OUT = path.join(fixtures, 'parity-page.json');
const load = (n) => JSON.parse(readFileSync(path.join(fixtures, n), 'utf8'));
const h = (x) => bytesOf(x);
const fill = (n, v) => new Uint8Array(n).fill(v);
const pattern = (n, seed) => Uint8Array.from({ length: n }, (_, i) => (i * 31 + seed) & 0xff);
// long outputs are keyed by their BLAKE3 (both sides have it)
const b3hex = (bytes) => hexOf(blake3(bytes));
const ops = (list) => list.map((o) => ({ canonical: hexOf(o.canonical), signature: hexOf(o.signature) }));
const rows = (d) => d.rows.map((r) => ({ label: r.label, value: r.value, danger: !!r.danger }));

const NOW0 = 1_900_000_000;
const SECRET = '42'.repeat(32);
const out = { meta: { note: 'written by scripts/parity-page.mjs; compared by crates/sshub-phone-core/tests/parity.rs', now0: NOW0 } };

// ---------------------------------------------------------------- identity and wrap generations
out.identity = ['42', '0b'].map((s) => Array.from({ length: 3 }, (_, g) => {
  const id = identityOfSecret(h(s.repeat(32)), g);
  return { secret: s, wrap_epoch: g, device_id: id.deviceId, ed_pub: hexOf(id.edPub), x_pub: hexOf(id.xPub), mlkem_pub_hash: b3hex(id.mlkemPub) };
})).flat();

// ---------------------------------------------------------------- the chain: every op the phone authors
{
  const g0 = identityOfSecret(h(SECRET));
  const g1 = identityOfSecret(h(SECRET), 1);
  const state = emptyState();
  const list = [];
  const keep = (r) => list.push(r);
  keep(appendHeadAttest(state, g0, NOW0, null));
  keep(appendHeadAttest(state, g0, NOW0 + 1, { lamport: 500n, epoch: 3n }));
  keep(appendRedPhoneSign(state, g0, '11'.repeat(32), NOW0 + 2, null));
  keep(appendWrapKeyRotate(state, g0, g1, NOW0 + 3, null));
  const issue = appendCertIssue(state, g1, { device_id: 'cccccccccccccccc', scope: { hosts: ['0a'.repeat(16), '0b'.repeat(16)] }, expiry: BigInt(NOW0 + 86400), max_new_hosts_per_day: 5, epoch_lo: 3n, epoch_hi: 4n }, NOW0 + 4, null);
  keep(issue);
  keep(appendCertIssue(state, g1, { device_id: 'dddddddddddddddd', scope: { group: '06'.repeat(16) }, expiry: BigInt(NOW0 + 30 * 86400), max_new_hosts_per_day: 50, epoch_lo: 3n, epoch_hi: 3n }, NOW0 + 5, null));
  const certId = issue.entry.hash;
  keep(appendCertExtend(state, g1, certId, 7, NOW0 + 6, null));
  keep(appendCertRevoke(state, g1, certId, NOW0 + 7, null));
  keep(appendReenrollCancel(state, g1, '22'.repeat(32), NOW0 + 8, { lamport: 9000n, epoch: 9n }));
  keep(appendShareStorageKey(state, g1, 4n, pattern(60, 1), NOW0 + 9, null));
  keep(appendConfirmBatch(state, g1, 4n, [{ host_uid: '0a'.repeat(16), sealed_share: pattern(70, 2) }, { host_uid: '0b'.repeat(16), sealed_share: pattern(70, 3) }], NOW0 + 10, null));
  out.chain = { ops: ops(list), head_hash: state.chain.at(-1).hash, cert_id: certId };
  out.courier_tail = ops(courierOps(state, 8));
  // the trust screen's list (structured) and the rows of every exported op as an approval screen would show them
  out.issued_certs = issuedCerts(state).map((c) => ({
    id: c.id, device_id: c.device_id, scope: c.scope, issued_at: c.issued_at, expiry: Number(c.expiry), max_new_hosts_per_day: c.max_new_hosts_per_day,
    revoked: c.revoked, extensions: c.extensions, term_text: describeTerm(c.expiry - BigInt(c.issued_at)),
  }));
  out.export_rows = courierOps(state, 8).map((o) => {
    const d = describeOp(parseOp(o.canonical), {});
    return { hash: opHashHex(o.canonical), headline: d.headline, rows: rows(d), danger_count: d.dangerCount };
  });
}

// ---------------------------------------------------------------- what an approval screen shows (op descriptions)
{
  const v = load('ops.vectors.json');
  const aliases = { '723ed85f0566df0c': 'terminal A' };
  out.describe = v.vectors.map((x) => {
    const d = describeOp(parseOp(h(x.canonical_hex)), aliases);
    return { id: x.id, hash: opHashHex(h(x.canonical_hex)), headline: d.headline, requester: d.requester, rows: rows(d), danger_count: d.dangerCount };
  });
  // The Red notice window as the page computes it (app.mjs redNotice): the op's own signed time, lapsing after 7 days.
  out.red_window = v.vectors.map((x) => ({ x, op: parseOp(h(x.canonical_hex)) })).filter(({ op }) => isRedOp(op)).map(({ x, op }) => ({
    id: x.id, timestamp: Number(op.timestamp), lamport: Number(op.lamport), seq: Number(op.seq), lapses_at: Number(op.timestamp) + 7 * 86400,
  }));
}

// ---------------------------------------------------------------- share release: every challenge of the fixture, with and without the requester in the ring
{
  const fx = load('challenges.json');
  const phone = identityOfSecret(h(fx.phone.secret_hex));
  const ringOf = (known) => (known ? { [fx.requester.pub.device_id]: { name: 'work laptop', ed25519_pub: fx.requester.pub.ed25519_pub } } : {});
  out.release = [];
  for (const known of [false, true]) {
    for (const c of fx.challenges) {
      const ctx = { now: fx.now, identity: phone, ring: ringOf(known), aliases: {} };
      const ev = evaluateChallenge(h(c.frame_hex), ctx);
      const rec = { name: c.name, ring_known: known, ok: ev.ok, checks: (ev.checks || []).map((k) => ({ text: k.text, warn: !!k.warn, ok: !!k.ok })) };
      if (!ev.ok) rec.refusal = ev.refusal;
      else {
        rec.binding_words = ev.bindingWords;
        rec.binding_hash = ev.bindingHash;
        rec.unknown_requester = !!ev.unknownRequester;
        rec.requester = ev.requester;
        rec.host = ev.hostHex;
        rec.description = { headline: ev.description.headline, rows: rows(ev.description), danger_count: ev.description.dangerCount };
        const state = emptyState();
        const r = approveRelease(ev, { identity: phone, state, now: fx.now, snapshot: null });
        const eph = h(c.eph_secret_hex);
        rec.reply_full = hexOf(sealedBoxOpen(eph, r.full.slice(23)));
        rec.reply_short = hexOf(sealedBoxOpen(eph, r.short.slice(23)));
        rec.head_attest = { canonical: state.chain[0].canonical, signature: state.chain[0].signature };
        rec.manual = r.manual;
      }
      out.release.push(rec);
    }
  }
}

// ---------------------------------------------------------------- trust-session requests: refusals and rows
{
  const NOW = 1_893_452_400;
  const DAY = 86400;
  const phoneId = identityOfSecret(h(SECRET)).deviceId;
  const base = { device_id: 'cccccccccccccccc', scope: { hosts: ['0a'.repeat(16)] }, expiry: BigInt(NOW + 7 * DAY), max_new_hosts_per_day: 2, epoch_lo: 3n, epoch_hi: 4n };
  const cases = {
    ok: {},
    ended: { expiry: BigInt(NOW - 1) },
    too_long: { expiry: BigInt(NOW + 91 * DAY) },
    budget_zero: { max_new_hosts_per_day: 0 },
    budget_over: { max_new_hosts_per_day: 51 },
    no_hosts: { scope: { hosts: [] } },
    group: { scope: { group: '06'.repeat(16) } },
    inverted: { epoch_lo: 5n, epoch_hi: 4n },
    wide: { epoch_lo: 3n, epoch_hi: 9n },
    wrong_start: { epoch_lo: 2n, epoch_hi: 3n },
    self: { device_id: phoneId },
  };
  out.cert_requests = Object.entries(cases).map(([name, over]) => {
    const req = { ...base, ...over };
    const d = describeCertRequest(req, NOW, { cccccccccccccccc: 'build box' });
    return { name, problems: certRequestProblems(req, NOW, phoneId, '3'), headline: d.headline, rows: rows(d), delegate_known: d.delegateKnown };
  });
}

// ---------------------------------------------------------------- reshare: the Rust-made fixture adopted by the page
{
  const fx = load('reshare.json');
  const phone = identityOfSecret(h(SECRET));
  const list = fx.ops.map((o) => ({ canonical: h(o.canonical_hex), signature: h(o.sig_hex) })).filter((o) => {
    const p = parseOp(o.canonical);
    return p.kind === 'ShareReshareBatch' || (p.kind === 'ShareReshareSession' && p.fields.to === phone.deviceId);
  });
  const ring = Object.fromEntries(fx.contributors.map((c) => [c.device_id, { name: `terminal ${c.device_id.slice(0, 4)}`, ed25519_pub: c.ed25519_pub }]));
  const plan = planReshare(phone, list, fx.epoch, { ring });
  const state = emptyState();
  const published = publishReshare(state, phone, plan, NOW0);
  const key = ownStorageKey(phone, state.chain, fx.epoch);
  const myIndex = plan.myIndex;
  const shares = {};
  for (const o of published) {
    const p = parseOp(o.canonical);
    if (p.kind !== 'ShareReshareConfirmBatch') continue;
    for (const it of p.fields.items) {
      const sealed = Uint8Array.from(atob(it.sealed_share), (c) => c.charCodeAt(0));
      shares[it.host_uid] = hexOf(xchacha20poly1305(key, sealed.slice(0, 24), envelopeAad(h(it.host_uid), fx.epoch, myIndex)).decrypt(sealed.slice(24)));
    }
  }
  out.reshare = { ok: plan.ok, holders: plan.holders, my_index: myIndex, contributors: plan.contributors.map((c) => ({ id: c.id, verified: c.verified, known: c.known, session: c.session })), credentials: plan.items.length, shares, kinds: published.map((o) => parseOp(o.canonical).kind) };
}

// ---------------------------------------------------------------- enrollment: commitments, reveals, SAS and confirmations
{
  const reveal = (v, name, attended = true) => ({ ed25519_pub: fill(32, v), x25519_pub: fill(32, v + 1), mlkem_pub: fill(1184, v + 2), eph: fill(32, v + 3), nonce: fill(32, v + 4), name, attended });
  const me = reveal(1, 'my phone');
  const them = reveal(0x21, 'work laptop');
  const th = transcriptHash(me, them);
  out.enroll = {
    phone_commit: hexOf(commitOf(0, me)),
    terminal_commit: hexOf(commitOf(1, them)),
    commit_frame: hexOf(encodeEnrollCommit(0, commitOf(0, me))),
    reveal_frame: hexOf(encodeEnrollReveal(0, me)),
    transcript_hash: hexOf(th),
    sas_words: sasWords(th),
    confirm_message_ok: hexOf(confirmMessage(0, true, th)),
    confirm_message_no: hexOf(confirmMessage(0, false, th)),
    confirm_frame: hexOf(encodeEnrollConfirm(0, true, fill(64, 9))),
  };
}

// ---------------------------------------------------------------- channels: fountain frames and the FSK burst
{
  const payload = pattern(1000, 5);
  const frames = fountainEncode(payload, 106, 25);
  out.fountain = { payload_len: payload.length, frames: frames.length, frames_hash: b3hex(Uint8Array.from(frames.flatMap((f) => Array.from(f)))) };
  const burst = (profile) => {
    const pcm = fskEncode(audioFrame(pattern(128, 7)), AUDIO_PROFILES[profile]);
    const i16 = new Int16Array(pcm.length);
    for (let i = 0; i < pcm.length; i++) i16[i] = Math.round(pcm[i] * 32767);
    // every 101st sample (the Rust side must agree within one least significant bit: libm may differ by an ulp)
    const probe = [];
    for (let i = 0; i < i16.length; i += 101) probe.push(i16[i]);
    return { samples: pcm.length, step: 101, probe };
  };
  out.fsk = { fast: burst('fast'), slow: burst('slow') };
}

const text = `${JSON.stringify(out, null, 1)}\n`;
const mode = process.argv[2];
if (mode === '--write') {
  writeFileSync(OUT, text);
  console.log(`wrote ${OUT} (${text.length} bytes)`);
} else if (mode === '--check') {
  const committed = readFileSync(OUT, 'utf8');
  if (committed !== text) {
    console.error('parity-page.json is stale: run `node scripts/parity-page.mjs --write` in the tools container and commit it');
    process.exit(1);
  }
  console.log('parity-page.json matches the page');
} else {
  process.stdout.write(text);
}
