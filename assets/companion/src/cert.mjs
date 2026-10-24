// Trust-session certificates on the phone (contract 4c-2, P1). The phone AUTHORS and signs CertIssue,
// CertRevoke and CertExtend; this module decides what it may sign and renders what it was asked, both
// from the bytes of the request (kind 9, kind 10). Everything the phone knows about the certificates
// it issued is read back from its own chain, so there is no second record to drift.
import { CERT_NEW_HOSTS_PER_DAY, MAX_CERT_REQUEST_HOSTS } from './frames.mjs';
import { parseOp, safeText } from './ops.mjs';
import { appendCertExtend, appendCertIssue, appendCertRevoke } from './state.mjs';

export const CERT_TERM_MAX_SECS = 90 * 86400; // limits.rs
export const EXTEND_WINDOW_SECS = 86400; // one extension per certificate per rolling 24 hours

const fromHex = (s) => Uint8Array.from(s.match(/../g) ?? [], (b) => parseInt(b, 16));
const iso = (t) => new Date(Number(t) * 1000).toISOString().replace('.000Z', 'Z');

export function describeTerm(secs) {
  const s = Number(secs);
  const d = Math.floor(s / 86400);
  const hours = Math.floor((s % 86400) / 3600);
  const mins = Math.floor((s % 3600) / 60);
  return d ? `${d} day${d === 1 ? '' : 's'}${hours ? ` ${hours} h` : ''}` : hours ? `${hours} h${mins ? ` ${mins} min` : ''}` : mins ? `${mins} min` : `${s} s`;
}

// What the screen shows for a kind 9 request, every row from the decoded bytes. aliases: deviceId -> name.
export function describeCertRequest(req, now, aliases = {}) {
  const who = aliases[req.device_id] ? `${safeText(aliases[req.device_id])} [${req.device_id}]` : `(no name known) [${req.device_id}]`;
  const term = req.expiry - BigInt(Math.floor(now));
  const scopeRows = req.scope.group
    ? [{ label: 'Scope: every host of group', value: req.scope.group, danger: true }]
    : [{ label: `Scope: ${req.scope.hosts.length} host${req.scope.hosts.length === 1 ? '' : 's'} only`, value: req.scope.hosts.join('\n'), danger: true }];
  return {
    headline: `Let ${who} open ${req.scope.group ? 'the hosts of a group' : `${req.scope.hosts.length} listed host${req.scope.hosts.length === 1 ? '' : 's'}`} without asking this phone again`,
    rows: [
      { label: 'Trusted device', value: who, danger: true },
      ...scopeRows,
      { label: 'Term', value: `${describeTerm(term)} (until ${iso(req.expiry)})`, danger: true },
      { label: 'New hosts per day', value: String(req.max_new_hosts_per_day), danger: true },
      { label: 'Phone epochs it holds for', value: `${req.epoch_lo} to ${req.epoch_hi}`, danger: false },
    ],
    delegateKnown: !!aliases[req.device_id],
  };
}

// The refusals, each a sentence a person can act on. Returns [] when the phone may sign.
export function certRequestProblems(req, now, ownId) {
  const out = [];
  const term = req.expiry - BigInt(Math.floor(now));
  if (term < 1n) out.push('The term has already ended: the expiry is not in the future.');
  else if (term > BigInt(CERT_TERM_MAX_SECS)) out.push(`The term is longer than 90 days (${term} s, ${describeTerm(term)}). Ask for a shorter one.`);
  if (req.max_new_hosts_per_day < 1 || req.max_new_hosts_per_day > CERT_NEW_HOSTS_PER_DAY) out.push(`The daily budget of new hosts must be between 1 and ${CERT_NEW_HOSTS_PER_DAY}, not ${req.max_new_hosts_per_day}.`);
  if (req.scope.hosts && (req.scope.hosts.length < 1 || req.scope.hosts.length > MAX_CERT_REQUEST_HOSTS)) out.push(`A list names 1 to ${MAX_CERT_REQUEST_HOSTS} hosts; use a group for more.`);
  if (req.epoch_lo > req.epoch_hi) out.push('The epoch range is inverted.');
  if (req.device_id === ownId) out.push('The request names this phone as the trusted device. A phone does not delegate to itself.');
  return out;
}

// The certificates this phone issued, read from its own chain: { id, req fields, issued_at, revoked,
// extensions: [{ at, extra }] }. The id of a certificate is the hash of its CertIssue op.
export function issuedCerts(state) {
  const text = new TextDecoder();
  const certs = new Map();
  for (const e of state.chain) {
    const bytes = fromHex(e.canonical);
    if (!/"body":\{"Cert(Issue|Revoke|Extend)"/.test(text.decode(bytes))) continue;
    const op = parseOp(bytes);
    if (op.kind === 'CertIssue') {
      const f = op.fields;
      certs.set(e.hash, {
        id: e.hash,
        device_id: f.device_id,
        scope: f.scope,
        expiry: BigInt(f.expiry),
        max_new_hosts_per_day: Number(f.max_new_hosts_per_day),
        issued_at: Number(op.timestamp),
        revoked: false,
        extensions: [],
      });
    } else {
      const c = certs.get(op.fields.cert_id);
      if (!c) continue;
      if (op.kind === 'CertRevoke') c.revoked = true;
      else c.extensions.push({ at: Number(op.timestamp), extra: Number(op.fields.extra_hosts) });
    }
  }
  return [...certs.values()];
}

// A kind 10 request against what this phone issued. { ok, problems, cert }.
export function extendProblems(state, req, now) {
  const cert = issuedCerts(state).find((c) => c.id === req.cert_id);
  const out = [];
  if (!cert) return { ok: false, problems: ['This phone did not issue that certificate (or it was issued by an earlier phone). Only the issuing phone extends it.'], cert: null };
  if (cert.revoked) out.push('That certificate was revoked; there is nothing to extend.');
  if (BigInt(Math.floor(now)) >= cert.expiry) out.push('That certificate has expired; an extension never revives it.');
  if (req.extra_hosts < 1 || req.extra_hosts > CERT_NEW_HOSTS_PER_DAY) out.push(`An extension adds 1 to ${CERT_NEW_HOSTS_PER_DAY} hosts.`);
  const last = cert.extensions.reduce((m, x) => Math.max(m, x.at), 0);
  if (last && now - last < EXTEND_WINDOW_SECS) out.push(`This certificate was extended at ${iso(last)}; the next extension is possible after ${iso(last + EXTEND_WINDOW_SECS)} (one per 24 hours).`);
  return { ok: out.length === 0, problems: out, cert };
}

export function describeCert(c, aliases = {}) {
  const who = aliases[c.device_id] ? `${safeText(aliases[c.device_id])} [${c.device_id}]` : `[${c.device_id}]`;
  const scope = c.scope.Group ? `group ${c.scope.Group}` : `${c.scope.Hosts.length} listed host${c.scope.Hosts.length === 1 ? '' : 's'}`;
  return `${who}: ${scope}, until ${iso(c.expiry)}, ${c.max_new_hosts_per_day} new hosts per day${c.extensions.length ? `, extended ${c.extensions.length}x` : ''}${c.revoked ? ', REVOKED' : ''}`;
}

// The three signing entry points: each re-checks its limits at the moment of signing, then chains and
// signs the op on the phone's own chain. They throw a sentence for the screen.
export function issueCert(state, identity, req, now) {
  const problems = certRequestProblems(req, now, identity.deviceId);
  if (problems.length) throw new Error(problems.join(' '));
  return appendCertIssue(state, identity, req, now, null);
}

export function revokeCert(state, identity, certId, now) {
  const c = issuedCerts(state).find((x) => x.id === certId);
  if (!c) throw new Error('This phone did not issue that certificate.');
  if (c.revoked) throw new Error('That certificate is already revoked.');
  return appendCertRevoke(state, identity, certId, now, null);
}

export function extendCert(state, identity, req, now) {
  const r = extendProblems(state, req, now);
  if (!r.ok) throw new Error(r.problems.join(' '));
  return appendCertExtend(state, identity, req.cert_id, req.extra_hosts, now, null);
}
