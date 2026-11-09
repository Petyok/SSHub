// Trust-session certificates on the phone (contract 4c-2, P1). The phone AUTHORS and signs CertIssue,
// CertRevoke and CertExtend; this module decides what it may sign and renders what it was asked, both
// from the bytes of the request (kind 9, kind 10). Everything the phone knows about the certificates
// it issued is read back from its own chain, so there is no second record to drift.
import { CERT_NEW_HOSTS_PER_DAY, MAX_CERT_REQUEST_HOSTS } from './frames.mjs';
import { iso, parseOp, safeText } from './ops.mjs';
import { HEAD_PENDING_MESSAGE, appendCertExtend, appendCertIssue, appendCertRenew, appendCertRevoke } from './state.mjs';

export const CERT_TERM_MAX_SECS = 90 * 86400; // limits.rs
// A terminal asks for the phone's current epoch plus this much slack (src/app/sync_trust.rs passes 1): a certificate then
// ends with the next reshare that revokes someone. A wider range would outlive it.
export const CERT_EPOCH_SLACK = 1n;
export const EXTEND_WINDOW_SECS = 86400; // one extension per certificate per rolling 24 hours
export const CERT_MAX_RENEWALS = 2; // limits.rs
export const CERT_LIFETIME_MAX_SECS = 180 * 86400; // limits.rs
export const CERT_RENEW_EVIDENCE_MAX_AGE_SECS = 6 * 3600; // limits.rs

const fromHex = (s) => Uint8Array.from(s.match(/../g) ?? [], (b) => parseInt(b, 16));

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
      { label: 'Phone epochs it holds for', value: `${req.epoch_lo} to ${req.epoch_hi}`, danger: true },
    ],
    delegateKnown: !!aliases[req.device_id],
  };
}

// The refusals, each a sentence a person can act on. Returns [] when the phone may sign.
export function certRequestProblems(req, now, ownId, ownEpoch = null) {
  const out = [];
  const term = req.expiry - BigInt(Math.floor(now));
  if (term < 1n) out.push('The term has already ended: the expiry is not in the future.');
  else if (term > BigInt(CERT_TERM_MAX_SECS)) out.push(`The term is longer than 90 days (${term} s, ${describeTerm(term)}). Ask for a shorter one.`);
  if (req.max_new_hosts_per_day < 1 || req.max_new_hosts_per_day > CERT_NEW_HOSTS_PER_DAY) out.push(`The daily budget of new hosts must be between 1 and ${CERT_NEW_HOSTS_PER_DAY}, not ${req.max_new_hosts_per_day}.`);
  if (req.scope.hosts && (req.scope.hosts.length < 1 || req.scope.hosts.length > MAX_CERT_REQUEST_HOSTS)) out.push(`A list names 1 to ${MAX_CERT_REQUEST_HOSTS} hosts; use a group for more.`);
  if (req.epoch_lo > req.epoch_hi) out.push('The epoch range is inverted.');
  else if (req.epoch_hi - req.epoch_lo > CERT_EPOCH_SLACK) out.push(`The epoch range ${req.epoch_lo} to ${req.epoch_hi} is wider than a terminal asks for (this phone's epoch plus ${CERT_EPOCH_SLACK}): the certificate would outlive the next reshare.`);
  if (ownEpoch !== null && req.epoch_lo !== BigInt(ownEpoch)) out.push(`The range starts at epoch ${req.epoch_lo}, but this phone is at epoch ${ownEpoch}. Deliver this phone's newest ops to the terminal (Export my ops) and ask again.`);
  if (req.device_id === ownId) out.push('The request names this phone as the trusted device. A phone does not delegate to itself.');
  return out;
}

// The certificates this phone issued, read from its own chain: { id, req fields, issued_at, revoked,
// extensions: [{ at, extra }], renewals: [BigInt end dates that count], effectiveExpiry }. The id of a certificate is the hash of its CertIssue op.
export function issuedCerts(state) {
  const text = new TextDecoder();
  const certs = new Map();
  for (const e of state.chain) {
    const bytes = fromHex(e.canonical);
    if (!/"body":\{"Cert(Issue|Revoke|Extend|Renew)"/.test(text.decode(bytes))) continue;
    const op = parseOp(bytes);
    if (op.kind === 'CertIssue') {
      const f = op.fields;
      certs.set(e.hash, {
        id: e.hash,
        device_id: f.device_id,
        scope: f.scope,
        expiry: BigInt(f.expiry),
        max_new_hosts_per_day: Number(f.max_new_hosts_per_day),
        epoch_lo: BigInt(f.epoch_lo),
        epoch_hi: BigInt(f.epoch_hi),
        issued_at: Number(op.timestamp),
        revoked: false,
        extensions: [],
        renewals: [],
        effectiveExpiry: BigInt(f.expiry),
      });
    } else {
      const c = certs.get(op.fields.cert_id);
      if (!c) continue;
      if (op.kind === 'CertRevoke') c.revoked = true;
      else if (op.kind === 'CertRenew') {
        // contract 4j validity of CertRenew, read in chain order: the same clock-free rule the host applies
        const end = BigInt(op.fields.expiry);
        if (!c.revoked && end > c.effectiveExpiry && end <= c.effectiveExpiry + (c.expiry - BigInt(c.issued_at)) && end - BigInt(c.issued_at) <= BigInt(CERT_LIFETIME_MAX_SECS) && c.renewals.length < CERT_MAX_RENEWALS) {
          c.renewals.push(end);
          c.effectiveExpiry = end;
        }
      } else c.extensions.push({ at: Number(op.timestamp), extra: Number(op.fields.extra_hosts) });
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
  if (BigInt(Math.floor(now)) >= cert.effectiveExpiry) out.push('That certificate has expired; an extension never revives it.');
  if (req.extra_hosts < 1 || req.extra_hosts > CERT_NEW_HOSTS_PER_DAY) out.push(`An extension adds 1 to ${CERT_NEW_HOSTS_PER_DAY} hosts.`);
  const last = cert.extensions.reduce((m, x) => Math.max(m, x.at), 0);
  if (last && now - last < EXTEND_WINDOW_SECS) out.push(`This certificate was extended at ${iso(last)}; the next extension is possible after ${iso(last + EXTEND_WINDOW_SECS)} (one per 24 hours).`);
  return { ok: out.length === 0, problems: out, cert };
}

export function describeCert(c, aliases = {}) {
  const who = aliases[c.device_id] ? `${safeText(aliases[c.device_id])} [${c.device_id}]` : `[${c.device_id}]`;
  const scope = c.scope.Group ? `group ${c.scope.Group}` : `${c.scope.Hosts.length} listed host${c.scope.Hosts.length === 1 ? '' : 's'}`;
  return `${who}: ${scope}, until ${iso(c.effectiveExpiry)}, ${c.max_new_hosts_per_day} new hosts per day${c.extensions.length ? `, extended ${c.extensions.length}x` : ''}${c.renewals.length ? `, renewed ${c.renewals.length}x` : ''}${c.revoked ? ', REVOKED' : ''}`;
}

// A kind 11 request against what this phone issued (contract 4j, design 6.2). { problems, cert, age } where age is the
// freshest evidence of a terminal other than the delegate, in seconds, or null.
export function renewProblems(state, req, now) {
  const out = [];
  if (state.headPending) out.push(HEAD_PENDING_MESSAGE);
  const cert = issuedCerts(state).find((c) => c.id === req.cert_id);
  if (!cert) {
    out.push('This phone did not issue that certificate (or an earlier phone did). Only the issuing phone renews it.');
    return { problems: out, cert: null, age: null };
  }
  const soFar = cert.effectiveExpiry;
  const issued = BigInt(cert.issued_at);
  if (cert.revoked) out.push('That certificate was revoked; there is nothing to renew.');
  if (cert.renewals.length >= CERT_MAX_RENEWALS) out.push(`This trust session was renewed ${CERT_MAX_RENEWALS} times already: a new one needs the full approval.`);
  if (req.expiry <= soFar) out.push('The new end date is not later than the current one.');
  else if (req.expiry > soFar + (cert.expiry - issued)) out.push('A renewal is at most as long as the first term.');
  if (req.expiry - issued > BigInt(CERT_LIFETIME_MAX_SECS)) out.push('A trust session never lives past 180 days from its start, renewals included.');
  if (BigInt(state.epoch) > cert.epoch_hi) out.push(`The certificate holds for epochs ${cert.epoch_lo} to ${cert.epoch_hi} and this phone is at epoch ${state.epoch}: a renewal would be refused at use. A new trust session is needed.`);
  const ages = req.evidence.filter((e) => e.terminal !== cert.device_id).map((e) => Math.max(0, Number(BigInt(Math.floor(now)) - e.head_ts)));
  const age = ages.length ? Math.min(...ages) : null;
  if (age === null) out.push('The list of opened hosts came only through the trusted device itself: no other terminal vouches for it. Ask a terminal that is not that device.');
  else if (age > CERT_RENEW_EVIDENCE_MAX_AGE_SECS) out.push(`No other terminal has news fresher than ${describeTerm(CERT_RENEW_EVIDENCE_MAX_AGE_SECS)} (the newest is ${describeTerm(age)} old): its list of opened hosts may be out of date. Sync the terminals and ask again.`);
  return { problems: out, cert, age };
}

// RenewView (contract 4j): what the screen shows for a kind 11 request. A certificate this phone does not know does not
// throw: the view carries the refusal. `opened` stays empty: kind 11 carries no list of opened hosts and the phone keeps none.
export function describeCertRenew(state, req, now) {
  const aliases = { ...Object.fromEntries(Object.entries(state.ring).map(([id, d]) => [id, d.name])), ...state.aliases };
  const { problems, cert, age } = renewProblems(state, req, now);
  const refusal = problems.length ? problems.join(' ') : null;
  if (!cert) return { delegate: '', scope: '', term_secs: 0, budget: 0, renewals_left: 0, new_end: Number(req.expiry), opened: [], evidence_age_secs: age, refusal };
  const who = aliases[cert.device_id] ? `${safeText(aliases[cert.device_id])} [${cert.device_id}]` : `(no name known) [${cert.device_id}]`;
  const n = cert.scope.Group ? 0 : cert.scope.Hosts.length;
  return {
    delegate: who,
    scope: cert.scope.Group ? `every host of group ${cert.scope.Group}` : `${n} listed host${n === 1 ? '' : 's'}`,
    term_secs: Number(req.expiry - cert.effectiveExpiry),
    budget: cert.max_new_hosts_per_day,
    renewals_left: Math.max(0, CERT_MAX_RENEWALS - cert.renewals.length),
    new_end: Number(req.expiry),
    opened: [],
    evidence_age_secs: age,
    refusal,
  };
}

// The signing entry point of contract 4j buildCertRenew: re-checks every limit at the moment of signing, then chains and
// signs the CertRenew op on the phone's own chain.
export function buildCertRenew(state, identity, req, now) {
  const { problems } = renewProblems(state, req, now);
  if (problems.length) throw new Error(problems.join(' '));
  return appendCertRenew(state, identity, req.cert_id, req.expiry, now, null);
}

// The three signing entry points: each re-checks its limits at the moment of signing, then chains and
// signs the op on the phone's own chain. They throw a sentence for the screen.
export function issueCert(state, identity, req, now) {
  const problems = certRequestProblems(req, now, identity.deviceId, state.epoch);
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
