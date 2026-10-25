// Canonical ops as the phone sees them: strict parse of the received bytes, a human description
// rendered FROM those bytes (there is no description field anywhere), and the explicit encoders
// of the few ops the phone authors itself (HeadAttest, PhoneReenrollCancel). Field order and
// forms mirror src/sync/oplog/canonical.rs (contract C2). Nothing here re-serialises a received
// op to obtain bytes: canonicalJSON() is used only as a strictness CHECK (the received bytes must
// equal the one canonical form, so duplicate keys, escapes or whitespace cannot hide a field).
import { blake3 } from '../vendor/noble-hashes-2.4.0/blake3.js';
import { canonicalJSON, edSign, hexOf, parseJSONExact, utf8 } from './lib.mjs';

const HEADER_KEYS = ['device_id', 'seq', 'lamport', 'prev_hash', 'timestamp', 'epoch', 'alg', 'body'];
const HEX = (n) => new RegExp(`^[0-9a-f]{${n}}$`);
export const ZERO32 = '0'.repeat(64);

// Fields of a host or group patch that C5 classes as dangerous.
const DANGEROUS_HOST = new Set([
  'address', 'port', 'username', 'proxy_jump', 'remote_command', 'forward_agent', 'transport',
  'environment', 'secret_lookup', 'has_password', 'identity_uid', 'group_uids',
]);
const DANGEROUS_GROUP = new Set([
  'default_proxy_jump', 'default_identity_uid', 'default_username', 'default_port',
  'default_transport', 'default_forward_agent', 'parent_uid',
]);
// Every field the Rust bodies know (op.rs HostFields, GroupFields): a key outside the list is shown as dangerous.
const KNOWN_FIELDS = {
  HostUpsert: new Set([...DANGEROUS_HOST, 'name', 'label', 'os_icon', 'tags', 'notes', 'favorite']),
  GroupUpsert: new Set([...DANGEROUS_GROUP, 'name']),
};
// kind -> which leaf rows are dangerous: '*' all, a Set of leaf names, or null for none.
const DANGER = {
  HostUpsert: DANGEROUS_HOST, GroupUpsert: DANGEROUS_GROUP, HostDelete: '*', GroupDelete: '*',
  IdentityUpsert: '*', IdentityDelete: '*', CredUpdate: '*', DeviceAdd: '*', DeviceRevoke: '*',
  ShareRelease: new Set(['host_uid', 'requester']), ShareReshareSession: '*', ShareStorageKey: '*',
  ShareReshareBatch: '*', ShareReshareConfirmBatch: '*', WrapKeyRotate: '*', CertIssue: '*',
  CertRevoke: '*', CertHostOpened: '*', CertExtend: '*', PhoneReenroll: '*', PhoneReenrollCancel: '*',
  RecoveryAck: '*', ModeChange: '*', HeadAttest: null,
  // Red ceremony (C17): the three proof ops are inert, the other three are Red themselves.
  RedApprove: null, RedPhoneSign: null, RedFactor: null, FactorRegister: '*', DeviceRoleChange: '*', BulkExport: '*', ExemptionSet: '*',
  // Added by the op-log package: a share handed to a requester, a recovery approval, checkpoints.
  ShareGrant: '*', RecoveryApprove: '*', CheckpointPropose: null, CheckpointSign: null,
};
export const KNOWN_KINDS = Object.keys(DANGER);

// Control and bidi characters never reach the screen raw: a name from another machine must not
// be able to reorder or hide the text around it.
// Every control, format, line and paragraph separator character (\p{Cc} \p{Cf} \p{Zl} \p{Zp}: bidi controls, zero-width
// and word-joiner characters, soft hyphen, tag characters) plus the invisible fillers and variation selectors.
// Also the blank-looking ones: braille blank, hangul and khmer fillers, and every space separator except U+0020.
const UNSAFE = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}\u034f\u115f\u1160\u17b4\u17b5\u180e\u2800\u3164\uffa0\ufe00-\ufe0f\u{e0100}-\u{e01ef}]|(?!\u0020)\p{Zs}/gu;
export const safeText = (s) =>
  String(s).replace(UNSAFE, (c) => {
    const cp = c.codePointAt(0);
    return cp > 0xffff ? `\\u{${cp.toString(16)}}` : `\\u${cp.toString(16).padStart(4, '0')}`;
  });

const asBig = (v) => (typeof v === 'bigint' ? v : BigInt(v));

// Strict parse of canonical op bytes. Throws with a reason a person can read.
export const MAX_OP_BYTES = 256 * 1024; // wire.rs MAX_COURIER_OP_BYTES
const U64_MAX = (1n << 64n) - 1n;
const I64_MIN = -(1n << 63n);
const I64_MAX = (1n << 63n) - 1n;
const inRange = (v, lo, hi) => (typeof v === 'number' || typeof v === 'bigint') && BigInt(v) >= lo && BigInt(v) <= hi;
export const isU64 = (v) => inRange(v, 0n, U64_MAX);
export function parseOp(bytes) {
  if (bytes.length > MAX_OP_BYTES) throw new Error(`op is larger than ${MAX_OP_BYTES} bytes`);
  let text;
  try {
    text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    throw new Error('op is not valid UTF-8');
  }
  let v;
  try {
    v = parseJSONExact(text);
  } catch (e) {
    throw new Error(`op is not canonical JSON: ${e.message}`);
  }
  if (canonicalJSON(v) !== text) throw new Error('op bytes are not the canonical form (duplicate key, escape or whitespace)');
  if (!v || typeof v !== 'object' || Array.isArray(v)) throw new Error('op is not an object');
  if (Object.keys(v).join(',') !== HEADER_KEYS.join(',')) throw new Error('op header fields are not in canonical order');
  if (typeof v.device_id !== 'string' || !HEX(16).test(v.device_id)) throw new Error('op device_id is not 16 hex characters');
  if (typeof v.prev_hash !== 'string' || !HEX(64).test(v.prev_hash)) throw new Error('op prev_hash is not 64 hex characters');
  for (const k of ['seq', 'lamport', 'timestamp', 'epoch']) {
    if (typeof v[k] !== 'number' && typeof v[k] !== 'bigint') throw new Error(`op ${k} is not an integer`);
  }
  // The Rust field types: seq, lamport, epoch are u64, timestamp is i64 (canonical.rs get_u64 / get_i64).
  for (const k of ['seq', 'lamport', 'epoch']) if (!isU64(v[k])) throw new Error(`op ${k} is outside u64`);
  if (!inRange(v.timestamp, I64_MIN, I64_MAX)) throw new Error('op timestamp is outside i64');
  if (!Array.isArray(v.alg) || v.alg.length !== 2 || !v.alg.every((n) => Number.isInteger(n) && n >= 0 && n < 256)) {
    throw new Error('op alg tag is malformed');
  }
  const b = v.body;
  const names = b && typeof b === 'object' && !Array.isArray(b) ? Object.keys(b) : [];
  if (names.length !== 1) throw new Error('op body must carry exactly one variant');
  const kind = names[0];
  if (!KNOWN_KINDS.includes(kind)) throw new Error(`unknown op kind ${safeText(kind)}`);
  if (!b[kind] || typeof b[kind] !== 'object' || Array.isArray(b[kind])) throw new Error('op body fields are not an object');
  return { ...v, kind, fields: b[kind] };
}

const BLOB = /^[A-Za-z0-9+/]{40,}={0,2}$/;
function fmtLeaf(v) {
  if (v === null) return '(cleared)';
  if (typeof v === 'string') {
    if (BLOB.test(v)) return `${v.length} base64 chars: ${safeText(v.slice(0, 16))}...`;
    return safeText(v);
  }
  return String(v);
}

// Every leaf of the body becomes a row; nothing is dropped, nothing is summarised away except
// long base64 blobs (shown with length and prefix). danger comes from the kind table.
function flatten(value, path, out, top) {
  if (value !== null && typeof value === 'object') {
    const entries = Array.isArray(value) ? value.map((x, i) => [String(i), x]) : Object.entries(value);
    if (entries.length === 0) out.push({ path, leaf: top, value: Array.isArray(value) ? '(empty list)' : '(none)' });
    for (const [k, x] of entries) flatten(x, path ? `${path}.${k}` : k, out, top ?? k);
  } else {
    out.push({ path, leaf: path.split('.').pop(), top, value: fmtLeaf(value) });
  }
}

export const iso = (t) => {
  try {
    return new Date(Number(t) * 1000).toISOString().replace('.000Z', 'Z');
  } catch {
    return String(t);
  }
};

// description = { kind, headline, requester, rows: [{label, value, danger}], dangerCount }
// aliases: deviceId -> name the user gave. The requesting device is always named by id and alias.
export function describeOp(op, aliases = {}, { now = null, staleAfter = 120 } = {}) {
  const who = (id) => `${aliases[id] ? `${safeText(aliases[id])} ` : '(no name known) '}[${safeText(id)}]`;
  const danger = DANGER[op.kind];
  const rows = [];
  let headline;
  let requester = null;
  if (op.kind === 'ShareRelease') {
    const f = op.fields;
    requester = f.requester;
    headline = `Release your share of host credential ${safeText(f.host_uid)} to ${who(f.requester)}`;
    rows.push({ label: 'Requesting device', value: who(f.requester), danger: true });
    rows.push({ label: 'Host credential', value: safeText(f.host_uid), danger: true });
    rows.push({ label: 'Request id', value: String(f.request_id), danger: false });
    rows.push({ label: 'Op signed by', value: who(op.device_id), danger: op.device_id !== f.requester });
  } else {
    headline = `${op.kind} by ${who(op.device_id)}`;
    requester = op.device_id;
    rows.push({ label: 'Op signed by', value: who(op.device_id), danger: false });
    const leaves = [];
    flatten(op.fields, '', leaves, null);
    const known = KNOWN_FIELDS[op.kind];
    for (const l of leaves) {
      const segs = l.path.split('.');
      let hot = danger === '*' || (danger instanceof Set && segs.some((x) => danger.has(x)));
      if (known && segs[0] === 'fields' && segs.length > 1 && !known.has(segs[1])) hot = true;
      rows.push({ label: safeText(l.path || op.kind), value: l.value, danger: !!hot });
    }
  }
  // A signed op much older than its request is highlighted (the caller passes the phone clock).
  const stale = now !== null && Math.abs(now - Number(op.timestamp)) > staleAfter;
  rows.push({ label: 'Op time', value: stale ? `${iso(op.timestamp)} (${Math.round(Math.abs(now - Number(op.timestamp)))} s from now)` : iso(op.timestamp), danger: stale });
  rows.push({ label: 'Op seq / epoch', value: `${op.seq} / ${op.epoch}`, danger: false });
  return { kind: op.kind, headline, requester, rows, dangerCount: rows.filter((r) => r.danger).length };
}

// ------------------------------------------------------------ ops the phone authors

const str = (s) => JSON.stringify(s);
function header(h) {
  return (
    `{"device_id":${str(h.device_id)},"seq":${asBig(h.seq)},"lamport":${asBig(h.lamport)},` +
    `"prev_hash":${str(h.prev_hash)},"timestamp":${asBig(h.timestamp)},"epoch":${asBig(h.epoch)},"alg":[1,1],"body":`
  );
}

// OpBody::HeadAttest { device_id, seq, chain_hash }
export function encodeHeadAttest(h, subject) {
  return utf8(`${header(h)}{"HeadAttest":{"device_id":${str(subject.device_id)},"seq":${asBig(subject.seq)},"chain_hash":${str(subject.chain_hash)}}}}`);
}

// OpBody::PhoneReenrollCancel { reenroll_hash, by_phone: true, third_factor: null }
export function encodePhoneReenrollCancel(h, reenrollHashHex) {
  return utf8(`${header(h)}{"PhoneReenrollCancel":{"reenroll_hash":${str(reenrollHashHex)},"by_phone":true,"third_factor":null}}}`);
}

export const opHashHex = (canonical) => hexOf(blake3(canonical));
export const signOp = (edSeed, canonical) => edSign(edSeed, canonical);

// OpBody::RedPhoneSign { op_hash }: the phone's proof that it saw the Red op with this hash.
export function encodeRedPhoneSign(h, opHashHexStr) {
  return utf8(`${header(h)}{"RedPhoneSign":{"op_hash":${str(opHashHexStr)}}}}`);
}

// OpBody::CertIssue { device_id, scope, expiry, max_new_hosts_per_day, epoch_lo, epoch_hi } with the fields of a
// kind 9 request UNCHANGED (CertRequestWire::to_body). scope is { group } or { hosts: [hex] } as decoded.
export function encodeCertIssue(h, r) {
  const uid = (u) => str(u);
  const scope = r.scope.group ? `{"Group":${uid(r.scope.group)}}` : `{"Hosts":[${r.scope.hosts.map(uid).join(',')}]}`;
  return utf8(`${header(h)}{"CertIssue":{"device_id":${str(r.device_id)},"scope":${scope},"expiry":${asBig(r.expiry)},"max_new_hosts_per_day":${asBig(r.max_new_hosts_per_day)},"epoch_lo":${asBig(r.epoch_lo)},"epoch_hi":${asBig(r.epoch_hi)}}}}`);
}

// OpBody::CertRevoke { cert_id }
export const encodeCertRevoke = (h, certIdHex) => utf8(`${header(h)}{"CertRevoke":{"cert_id":${str(certIdHex)}}}}`);

// OpBody::CertExtend { cert_id, extra_hosts }
export const encodeCertExtend = (h, certIdHex, extraHosts) => utf8(`${header(h)}{"CertExtend":{"cert_id":${str(certIdHex)},"extra_hosts":${asBig(extraHosts)}}}}`);

// Red operations (design "The Red ceremony"): a Red op needs the phone's proof. A mirror
// DeviceAdd and a DeviceRevoke of a mirror are Yellow in the ring; the phone cannot tell a
// holder from a mirror in a revoke, so it signs any DeviceRevoke the user approves (the proof is inert).
export function isRedOp(op) {
  if (op.kind === 'DeviceAdd') return op.fields.role === 'Holder';
  return ['DeviceRevoke', 'CertIssue', 'ModeChange', 'FactorRegister', 'DeviceRoleChange', 'BulkExport', 'ExemptionSet'].includes(op.kind);
}

export const b64enc = (bytes) => {
  let s = '';
  for (let i = 0; i < bytes.length; i += 8192) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 8192));
  return btoa(s);
};
export const b64dec = (text) => {
  if (typeof text !== 'string' || !/^[A-Za-z0-9+/]*={0,2}$/.test(text) || text.length % 4) throw new Error('not base64');
  return Uint8Array.from(atob(text), (c) => c.charCodeAt(0));
};

// OpBody::WrapKeyRotate { device_id, wrap_epoch, x25519_pub, mlkem_pub }
export function encodeWrapKeyRotate(h, r) {
  return utf8(`${header(h)}{"WrapKeyRotate":{"device_id":${str(r.device_id)},"wrap_epoch":${asBig(r.wrap_epoch)},"x25519_pub":${str(hexOf(r.x25519_pub))},"mlkem_pub":${str(b64enc(r.mlkem_pub))}}}}`);
}

// OpBody::ShareStorageKey { epoch, sealed }
export function encodeShareStorageKey(h, r) {
  return utf8(`${header(h)}{"ShareStorageKey":{"epoch":${asBig(r.epoch)},"sealed":${str(b64enc(r.sealed))}}}}`);
}

// OpBody::ShareReshareConfirmBatch { epoch, items: [{ host_uid, sealed_share }] }
export function encodeShareReshareConfirmBatch(h, r) {
  const items = r.items.map((i) => `{"host_uid":${str(i.host_uid)},"sealed_share":${str(b64enc(i.sealed_share))}}`).join(',');
  return utf8(`${header(h)}{"ShareReshareConfirmBatch":{"epoch":${asBig(r.epoch)},"items":[${items}]}}}`);
}
