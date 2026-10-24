// The phone page: screens and glue. Every decision (what to show, what to refuse) is made by the
// pure modules (release, ops, frames, state); this file renders their results and moves bytes
// between the camera, the screen, the clipboard and the speaker. Text goes in with textContent
// only, never innerHTML.
import { bytesOf, edSign, edVerify, hexOf, sasWords, x25519Pub, deviceIdOf } from './lib.mjs';
import {
  PIN_MIN_CHARS, generateSecret, identityOfSecret, randomBytes, secretFromWords, secretToWords, unwrapSecret, wrapSecret,
} from './keys.mjs';
import {
  COURIER_TAIL, appendRedPhoneSign, appendWrapKeyRotate, appendReenrollCancel, courierOps, emptyState, indexedDbBackend, invalidateSnapshot, issueSnapshot, liveSnapshot, loadState, recordSnapshot, saveState,
} from './state.mjs';
import { TIMESTAMP_SKEW_GRACE_SECS, approveRelease, evaluateChallenge } from './release.mjs';
import { describeOp, isRedOp, opHashHex, parseOp, safeText } from './ops.mjs';
import {
  KIND, ROLE_EXISTING, ROLE_JOINING, commitOf, confirmMessage, decodeEnroll, decodeFrame, decodePhoneOps, decodeRecoveryNotice, encodeEnrollCommit,
  encodeEnrollConfirm, encodeEnrollReveal, encodePhoneOps, transcriptHash,
} from './frames.mjs';
import { AUDIO_PROFILES, loopQr, playSamples, responseBurst, reverseQrFrames, toBase64 } from './channels.mjs';
import { DocumentCollector, startCamera } from './scan.mjs';
import { ownStorageKey, planReshare, publishReshare } from './reshare.mjs';

const RECOVERY_TIMELOCK_SECS = 7 * 86400;
const RED_CEREMONY_TTL_SECS = 3 * 86400; // limits.rs
const MAX_QR_DOC_BYTES = 12000; // beyond this the animation is too long: text only

function h(tag, attrs = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === false || v == null) continue;
    if (k === 'class') e.className = v;
    else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
    else e.setAttribute(k, v === true ? '' : v);
  }
  for (const c of kids.flat()) if (c != null && c !== false) e.append(c.nodeType ? c : document.createTextNode(String(c)));
  return e;
}
const btn = (text, onclick, cls = '', id) => h('button', { class: cls, onclick, id, type: 'button' }, text);
const wordsBox = (phrase, id) => h('div', { class: 'words', id }, phrase.split(' ').map((w) => h('span', {}, w)));

export async function startApp(root, { kv = indexedDbBackend(), now = () => Date.now() / 1000, fingerprint = '' } = {}) {
  const app = { kv, now, fingerprint, identity: null, state: await loadState(kv), cleanup: [] };
  const show = (...nodes) => {
    for (const f of app.cleanup.splice(0)) f();
    root.replaceChildren(...nodes.flat().filter(Boolean));
    globalThis.scrollTo && globalThis.scrollTo(0, 0);
  };
  const save = () => saveState(app.kv, app.state);
  const lock = () => {
    if (app.identity) app.identity.secret.fill(0);
    app.identity = null;
    home();
  };
  document.addEventListener('visibilitychange', () => document.visibilityState === 'hidden' && app.identity && lock());

  const title = (t, sub) => [h('h1', {}, t), sub ? h('p', { class: 'mut' }, sub) : null];
  const back = () => btn('Back', home, '', 'back');

  // ---------------------------------------------------------------- first run, unlock, backup

  async function boot() {
    const blob = await app.kv.get('key');
    if (!blob) return setup();
    return app.identity ? home() : unlock();
  }

  function setup() {
    const pin1 = h('input', { id: 'pin1', type: 'password', autocomplete: 'new-password', placeholder: `PIN or passphrase, at least ${PIN_MIN_CHARS} characters` });
    const pin2 = h('input', { id: 'pin2', type: 'password', autocomplete: 'new-password', placeholder: 'Repeat it' });
    const words = h('textarea', { id: 'words-input', placeholder: '24 backup words, separated by spaces', spellcheck: 'false', autocapitalize: 'none' });
    const msg = h('p', { class: 'err', id: 'setup-error' });
    const check = () => {
      if (pin1.value.length < PIN_MIN_CHARS) return `The PIN needs at least ${PIN_MIN_CHARS} characters.`;
      if (pin1.value !== pin2.value) return 'The two PINs differ.';
      return null;
    };
    const finish = async (secret, fresh) => {
      const e = check();
      if (e) return void (msg.textContent = e);
      msg.textContent = 'Deriving the key...';
      await app.kv.set('key', await wrapSecret(secret, pin1.value));
      app.identity = identityOfSecret(secret);
      app.state.enrolled = null;
      await save();
      if (fresh) backup(secretToWords(secret), true);
      else home();
    };
    show(
      title('SSHub companion', 'This phone holds one key share holder identity. Nothing leaves the page: no network, no account.'),
      h('div', { class: 'card' }, h('h2', {}, 'Protect it'), pin1, pin2, msg),
      btn('Create a new key', () => finish(generateSecret(), true), 'primary', 'create'),
      h('h2', {}, 'Or restore'),
      words,
      btn('Restore from 24 words', () => {
        try {
          finish(secretFromWords(words.value), false);
        } catch (e) {
          msg.textContent = e.message;
        }
      }, '', 'restore'),
    );
  }

  function backup(phrase, first) {
    const ok = h('input', { id: 'backup-ok', type: 'checkbox' });
    const done = btn('Continue', home, 'primary', 'backup-done');
    done.disabled = first;
    ok.addEventListener('change', () => (done.disabled = !ok.checked));
    show(
      title('Your 24 backup words', 'They ARE the key. Anyone with them can act as this phone; with them you can restore it on a new phone. Write them on paper, in order. Do not photograph them.'),
      wordsBox(phrase, 'backup-words'),
      first ? h('label', { class: 'inline' }, ok, 'I wrote the words down on paper.') : null,
      done,
    );
  }

  function unlock(error) {
    const pin = h('input', { id: 'pin', type: 'password', autocomplete: 'current-password', placeholder: 'PIN' });
    const msg = h('p', { class: 'err', id: 'unlock-error' }, error || '');
    const go = async () => {
      msg.textContent = 'Unlocking...';
      try {
        app.identity = identityOfSecret(await unwrapSecret(await app.kv.get('key'), pin.value), Number(app.state.wrapEpoch || 0));
        home();
      } catch (e) {
        msg.textContent = e.message;
      }
    };
    pin.addEventListener('keydown', (e) => e.key === 'Enter' && go());
    show(title('SSHub companion', 'Locked'), pin, msg, btn('Unlock', go, 'primary', 'unlock'), btn('Restore from 24 words instead', () => restoreOver(), '', 'restore-over'));
  }

  // Replacing the stored key is a deliberate act: everything tied to the old identity goes.
  function restoreOver() {
    const words = h('textarea', { id: 'words-input', spellcheck: 'false', autocapitalize: 'none', placeholder: '24 backup words' });
    const pin = h('input', { id: 'pin1', type: 'password', autocomplete: 'new-password', placeholder: `New PIN, at least ${PIN_MIN_CHARS} characters` });
    const msg = h('p', { class: 'err', id: 'setup-error' });
    show(
      title('Restore', 'Replaces the key stored on this phone. Enrollments and the short-code list of the old key are erased.'),
      words, pin, msg,
      btn('Replace and restore', async () => {
        try {
          const secret = secretFromWords(words.value);
          if (pin.value.length < PIN_MIN_CHARS) throw new Error(`The PIN needs at least ${PIN_MIN_CHARS} characters.`);
          await app.kv.set('key', await wrapSecret(secret, pin.value));
          app.identity = identityOfSecret(secret);
          app.state = emptyState();
          await save();
          home();
        } catch (e) {
          msg.textContent = e.message;
        }
      }, 'danger', 'restore'),
      btn('Cancel', () => unlock(), '', 'back'),
    );
  }

  function showBackup() {
    const pin = h('input', { id: 'pin', type: 'password', placeholder: 'PIN' });
    const msg = h('p', { class: 'err' });
    show(
      title('Show the backup words', 'Enter the PIN again. Nobody should be looking at the screen.'),
      pin, msg,
      btn('Show', async () => {
        try {
          backupAgain(secretToWords(await unwrapSecret(await app.kv.get('key'), pin.value)));
        } catch (e) {
          msg.textContent = e.message;
        }
      }, 'primary', 'show'),
      back(),
    );
  }
  const backupAgain = (phrase) => show(title('Your 24 backup words'), wordsBox(phrase, 'backup-words'), btn('Hide', home, 'primary', 'backup-done'));

  // ---------------------------------------------------------------- home

  function home() {
    if (!app.identity) return void boot();
    const ring = Object.entries(app.state.ring);
    const snaps = Object.entries(app.state.snapshots).filter(([k]) => liveSnapshot(app.state, k, app.now()));
    show(
      title('SSHub companion', null),
      h('div', { class: 'card' },
        h('div', { class: 'mut' }, 'This phone is device'),
        h('div', { class: 'big', id: 'device-id' }, app.identity.deviceId),
        h('div', { class: 'mut' }, ring.length ? `Enrolled with: ${ring.map(([id, d]) => `${safeText(d.name)} [${id}]`).join(', ')}` : 'Not enrolled with any terminal yet.'),
        h('div', { class: 'mut' }, `Chain head: ${app.state.chain.length ? `seq ${app.state.chain[app.state.chain.length - 1].seq}` : 'no ops yet'}; live short codes: ${snaps.length}`),
      ),
      btn('Scan a share request', () => scanRelease(), 'primary', 'scan-release'),
      btn('Enroll with a terminal', () => enroll(), '', 'enroll'),
      btn('Short codes I issued', () => snapshotList(), '', 'snapshots'),
      btn('Export my ops as text', exportOps, '', 'export-ops'),
      btn('Reshare and wrap keys', reshareScreen, '', 'reshare'),
      btn('Recovery and a new phone', () => recovery(), '', 'recovery'),
      btn('Show backup words', showBackup, '', 'show-backup'),
      btn('About and fingerprint', about, '', 'about'),
      btn('Lock', lock, '', 'lock'),
    );
  }

  function about() {
    show(
      title('About this page'),
      h('div', { class: 'card' },
        h('div', { class: 'mut' }, 'Fingerprint of this file (the terminal prints the same value)'),
        h('div', { class: 'big fp', id: 'fingerprint' }, app.fingerprint),
        h('p', { class: 'mut' }, 'To recompute it: replace the 64 hex characters of the fingerprint inside the file with 64 zeros and run sha256sum on the result.'),
        h('p', { class: 'mut' }, `Secure context: ${globalThis.isSecureContext ? 'yes' : 'no'}. Content-Security-Policy: no network access of any kind.`),
      ),
      back(),
    );
  }

  // ---------------------------------------------------------------- scanning

  // Shared scan screen. onDoc(bytes) gets each completed document.
  function scan({ heading, hint, onDoc, expect = null, onBack = home }) {
    const video = h('video', { id: 'cam', muted: true, playsinline: true });
    const bar = h('progress', { id: 'progress', max: '100', value: '0' });
    const status = h('p', { class: 'mut', id: 'scan-status' }, 'Starting the camera...');
    const paste = h('textarea', { id: 'paste', placeholder: 'base64 of the document (no camera? paste it here)', spellcheck: 'false' });
    const col = new DocumentCollector();
    let cam = null;
    let finished = false;
    const done = (bytes) => {
      if (finished) return;
      finished = true;
      cam && cam.stop();
      onDoc(bytes);
    };
    show(
      title(heading, hint), video, bar, status,
      h('details', {}, h('summary', {}, 'No camera? Paste instead'), paste, btn('Use pasted text', () => {
        try {
          done(Uint8Array.from(atob(paste.value.trim()), (c) => c.charCodeAt(0)));
        } catch {
          status.textContent = 'That is not valid base64.';
        }
      }, '', 'paste-go')),
      btn('Back', onBack, '', 'back'),
    );
    app.cleanup.push(() => {
      finished = true;
      cam && cam.stop();
    });
    startCamera(video, (frame) => {
      const doc = col.push(frame);
      const p = col.progress();
      bar.value = p.total ? Math.round((100 * p.have) / p.total) : 0;
      status.textContent = p.total ? `Reading: ${p.have} of ${p.total} parts${col.errors ? `, ${col.errors} damaged` : ''}` : 'Point the camera at the terminal QR code.';
      if (doc) {
        let kind = -1;
        try {
          kind = decodeFrame(doc).kind;
        } catch {
          /* not one of ours */
        }
        if (!expect || expect.includes(kind)) done(doc);
        else {
          col.reset();
          status.textContent = 'Ignored a QR code that is not what this step needs.';
        }
      }
    }).then((c) => {
      cam = c;
      if (finished) c.stop();
      else status.textContent = 'Point the camera at the terminal QR code.';
    }).catch((e) => {
      status.textContent = `Camera unavailable: ${e.message}`;
      status.className = 'err';
    });
  }

  function refusalScreen(text, checks) {
    show(
      title('Refused'),
      h('div', { class: 'refusal', id: 'refusal' }, text),
      h('div', { class: 'card', id: 'checks' }, (checks || []).map((c) => h('div', { class: `check ${c.ok ? (c.warn ? 'warn' : 'ok') : 'bad'}` }, c.text))),
      btn('Back', home, 'primary', 'back'),
    );
  }

  // ---------------------------------------------------------------- share release

  function scanRelease() {
    scan({
      heading: 'Scan the share request',
      hint: 'Open the unlock screen on the terminal and point the camera at its QR code.',
      expect: [KIND.CHALLENGE, KIND.RECOVERY_NOTICE],
      onDoc: (bytes) => {
        let kind;
        try {
          kind = decodeFrame(bytes).kind;
        } catch (e) {
          return refusalScreen(`Not a SSHub document: ${e.message}`, []);
        }
        if (kind === KIND.RECOVERY_NOTICE) return recoveryNotice(bytes);
        const ev = evaluateChallenge(bytes, { now: app.now(), identity: app.identity, ring: app.state.ring, aliases: app.state.aliases });
        if (!ev.ok) return refusalScreen(ev.refusal, ev.checks);
        release(ev);
      },
    });
  }

  function descriptionRows(desc) {
    return h('div', { class: 'rows', id: 'desc' }, desc.rows.flatMap((r) => [
      h('div', { class: 'k' }, r.label),
      h('div', { class: `v${r.danger ? ' danger' : ''}`, 'data-danger': r.danger ? '1' : '0' }, r.value),
    ]));
  }

  function release(ev) {
    const live = liveSnapshot(app.state, ev.hostHex, app.now());
    const optSnap = h('input', { id: 'opt-snapshot', type: 'checkbox', checked: !live, disabled: !!live });
    const optAudio = h('select', { id: 'opt-audio-profile' }, h('option', { value: 'fast' }, 'Sound: fast (about 8 s)'), h('option', { value: 'slow' }, 'Sound: slow, noisy rooms (about 16 s)'));
    const optQr = h('input', { id: 'opt-qr', type: 'checkbox', checked: true });
    const optClip = h('input', { id: 'opt-clip', type: 'checkbox', checked: true });
    const optSound = h('input', { id: 'opt-sound', type: 'checkbox', checked: true });
    const timer = h('div', { class: 'mut', id: 'timer' });
    const msg = h('p', { class: 'err', id: 'release-error' });
    const approveBtn = btn('Approve and release', () => approve(), 'primary', 'approve');
    const tick = () => {
      const left = Math.floor(ev.challenge.expiry - app.now());
      timer.textContent = left > 0 ? `Expires in ${left} s` : 'Expired: ask the terminal for a new request.';
      approveBtn.disabled = left <= 0;
    };
    tick();
    const iv = setInterval(tick, 1000);
    async function approve() {
      if (app.now() >= ev.challenge.expiry) return void tick();
      approveBtn.disabled = true;
      msg.textContent = optSnap.checked ? 'Sealing the short code (this takes a few seconds)...' : 'Approving...';
      try {
        let snap = null;
        if (optSnap.checked) snap = await issueSnapshot(app.state, ev.hostHex, ev.share[0], ev.share.slice(1), app.now());
        const out = approveRelease(ev, { identity: app.identity, state: app.state, now: app.now(), snapshot: snap ? { host_uid: bytesOf(ev.hostHex), blob: snap.blob } : null });
        if (snap) recordSnapshot(app.state, ev.hostHex, snap.record);
        await save();
        send(ev, out, snap, { qr: optQr.checked, clip: optClip.checked, sound: optSound.checked, profile: optAudio.value });
      } catch (e) {
        msg.textContent = e.message;
        approveBtn.disabled = false;
      }
    }
    show(
      title('Share request'),
      h('div', { class: 'headline', id: 'headline' }, ev.description.headline),
      descriptionRows(ev.description),
      h('div', { class: 'card' },
        h('div', { class: 'mut' }, 'Binding words (the terminal shows the same four)'),
        h('div', { class: 'sas', id: 'binding-words' }, ev.bindingWords),
        h('div', { class: 'mut', id: 'binding-hash' }, ev.bindingHash)),
      h('div', { class: 'card', id: 'checks' }, ev.checks.map((c) => h('div', { class: `check ${c.warn ? 'warn' : 'ok'}` }, c.text))),
      timer,
      h('div', { class: 'card' },
        h('label', { class: 'inline' }, optQr, 'Show a QR code'),
        h('label', { class: 'inline' }, optClip, 'Copy to the clipboard'),
        h('label', { class: 'inline' }, optSound, 'Play a sound'), optAudio,
        h('label', { class: 'inline' }, optSnap, live ? `A short code for this credential is already live until ${new Date(live.expires_at * 1000).toISOString().slice(0, 16)}Z; a second one is refused.` : 'Also issue a short code (repeat access for 7 days)')),
      msg, approveBtn,
      btn('Refuse', home, '', 'refuse'),
    );
    app.cleanup.push(() => clearInterval(iv), () => ev.share.fill(0));
  }

  function send(ev, out, snap, opts) {
    const stops = [];
    const nodes = [title('Approved', `Sent to ${ev.description.requester ? safeText(ev.requester) : 'the terminal'}. The terminal takes the first channel that arrives.`)];
    if (opts.qr) {
      const canvas = h('canvas', { class: 'qr', id: 'reverse-qr' });
      nodes.push(h('h2', {}, 'QR code'), h('p', { class: 'mut' }, 'Hold it in front of the laptop camera.'), canvas);
      stops.push(loopQr(canvas, reverseQrFrames(out.full)).stop);
    }
    if (opts.clip) {
      const text = toBase64(out.full);
      const status = h('p', { class: 'mut', id: 'clip-status' }, 'Copying...');
      nodes.push(h('h2', {}, 'Clipboard'), status, btn('Copy again', () => copy(), '', 'copy'));
      const copy = async () => {
        try {
          await navigator.clipboard.writeText(text);
          status.textContent = 'Copied (base64, encrypted for one use).';
        } catch {
          status.textContent = 'The browser refused the clipboard; tap Copy again.';
        }
      };
      copy();
    }
    if (opts.sound) {
      const status = h('p', { class: 'mut', id: 'audio-status' }, 'Tap Play.');
      const play = async () => {
        status.textContent = 'Playing...';
        try {
          await playSamples(responseBurst(out.short, opts.profile));
          status.textContent = 'Played. Tap again to repeat.';
        } catch (e) {
          status.textContent = `Sound failed: ${e.message}`;
        }
      };
      nodes.push(h('h2', {}, `Sound (${opts.profile})`), status, btn('Play sound', play, '', 'play'));
      play();
    }
    nodes.push(
      h('details', { id: 'manual' }, h('summary', {}, 'Type it by hand (24 words)'),
        h('p', { class: 'mut' }, 'Enter the index and the words on the terminal. Anyone who sees them has this share: look around first.'),
        h('div', { class: 'big', id: 'manual-index' }, `Index ${out.manual.index}`), wordsBox(out.manual.words, 'manual-words')),
    );
    if (snap) {
      nodes.push(
        h('h2', {}, 'Short code'),
        h('div', { class: 'big', id: 'snapshot-code' }, snap.code),
        h('p', { class: 'mut' }, `Valid until ${new Date(snap.record.expires_at * 1000).toISOString().slice(0, 16)}Z, once per credential. Argon2id ${snap.record.profile.mem_kib / 1024} MiB x ${snap.record.profile.iters}${snap.record.weak ? ' (below the usual floor: this phone is slow)' : ''}. The sealed blob went with the reply above.`));
    }
    nodes.push(btn('Done', home, 'primary', 'send-done'));
    show(...nodes);
    app.cleanup.push(() => ev.share.fill(0), ...stops);
  }

  // ---------------------------------------------------------------- short codes

  function snapshotList() {
    const rows = Object.entries(app.state.snapshots).map(([host, s]) => {
      const alive = liveSnapshot(app.state, host, app.now());
      return h('div', { class: 'card', 'data-host': host },
        h('div', { class: 'v' }, host), h('div', { class: 'mut' }, `${alive ? 'live' : 'expired'}; issued ${new Date(s.issued_at * 1000).toISOString().slice(0, 16)}Z, expires ${new Date(s.expires_at * 1000).toISOString().slice(0, 16)}Z; Argon2id ${s.profile.mem_kib / 1024} MiB x ${s.profile.iters}${s.weak ? ' (weak)' : ''}`),
        btn('Declare it dead (blob never arrived or the credential was reissued)', async () => { invalidateSnapshot(app.state, host); await save(); snapshotList(); }, '', `kill-${host}`));
    });
    show(title('Short codes I issued', 'One live code per credential. The code itself is never stored here.'), rows.length ? rows : h('p', { class: 'mut', id: 'no-snapshots' }, 'None.'), back());
  }

  // ---------------------------------------------------------------- enrollment

  function enroll() {
    const nameIn = h('input', { id: 'device-name', value: 'my phone', maxlength: '40' });
    show(
      title('Enroll with a terminal', 'On the terminal start phone enrollment; it shows a QR code. This phone and the terminal then each commit to their keys, reveal them, and show four words that must be identical.'),
      h('label', {}, 'Name this phone will have in the ring', nameIn),
      btn('Scan the terminal', () => {
        const name = nameIn.value.trim();
        if (!name || new TextEncoder().encode(name).length > 64 || /[\u0000-\u001f\u007f]/.test(name)) return void (nameIn.className = 'err');
        enrollStep1(name);
      }, 'primary', 'enroll-scan'),
      back(),
    );
  }

  function enrollStep1(name) {
    scan({
      heading: 'Enrollment 1 of 4: terminal commitment',
      hint: 'The terminal shows a short QR code.',
      expect: [KIND.ENROLL_COMMIT],
      onBack: enroll,
      onDoc: (bytes) => {
        let m;
        try {
          m = decodeEnroll(bytes);
        } catch (e) {
          return refusalScreen(`Not an enrollment document: ${e.message}`, []);
        }
        if (m.kind !== 'commit' || m.role !== ROLE_EXISTING) return refusalScreen('Expected the terminal commitment first.', []);
        const ephSec = randomBytes(32);
        const me = {
          ed25519_pub: app.identity.edPub, x25519_pub: app.identity.xPub, mlkem_pub: app.identity.mlkemPub,
          eph: x25519Pub(ephSec), nonce: randomBytes(32), name, attended: true,
        };
        enrollShow(`Enrollment 2 of 4: show this phone's commitment`, 'Hold this to the terminal camera until it confirms, then tap Next.', encodeEnrollCommit(ROLE_JOINING, commitOf(ROLE_JOINING, me)), () => enrollStep3(me, m.commit));
      },
    });
  }

  function enrollShow(heading, hint, doc, next, extra = []) {
    const canvas = h('canvas', { class: 'qr', id: 'enroll-qr' });
    show(title(heading, hint), canvas, ...extra, next ? btn('Next', next, 'primary', 'enroll-next') : null, btn('Cancel', home, '', 'back'));
    app.cleanup.push(loopQr(canvas, reverseQrFrames(doc)).stop);
  }

  function enrollStep3(me, theirCommit) {
    scan({
      heading: 'Enrollment 3 of 4: terminal keys',
      hint: 'The terminal now reveals its keys.',
      expect: [KIND.ENROLL_REVEAL],
      onBack: enroll,
      onDoc: (bytes) => {
        let m;
        try {
          m = decodeEnroll(bytes);
        } catch (e) {
          return refusalScreen(`Not an enrollment document: ${e.message}`, []);
        }
        if (m.kind !== 'reveal' || m.role !== ROLE_EXISTING) return refusalScreen('Expected the terminal reveal.', []);
        const r = m.reveal;
        if (hexOf(commitOf(ROLE_EXISTING, r)) !== hexOf(theirCommit)) return refusalScreen('The terminal reveal does not open the commitment it showed first. Enrollment stopped: someone is interfering.', []);
        if (r.mlkem_pub.length !== 1184 || hexOf(r.ed25519_pub) === hexOf(me.ed25519_pub)) return refusalScreen('The terminal presented unusable keys.', []);
        const th = transcriptHash(me, r);
        const words = sasWords(th);
        enrollShow(
          'Enrollment 4 of 4: compare the words',
          'The terminal shows four words too. They must be identical. Different words mean someone sits between you: tap Differ.',
          encodeEnrollReveal(ROLE_JOINING, me),
          null,
          [
            h('div', { class: 'sas', id: 'sas-words' }, words),
            h('div', { class: 'mut' }, `Terminal calls itself: ${safeText(r.name)} [${deviceIdOf(r.ed25519_pub)}]`),
            btn('The words match', () => enrollConfirm(me, r, th, true), 'primary', 'sas-match'),
            btn('They differ', () => enrollConfirm(me, r, th, false), 'danger', 'sas-differ'),
          ],
        );
      },
    });
  }

  function enrollConfirm(me, r, th, ok) {
    const sig = edSign(app.identity.edSeed, confirmMessage(ROLE_JOINING, ok, th));
    if (!ok) {
      return enrollShow('Enrollment declined', 'Show this to the terminal so it stops too.', encodeEnrollConfirm(ROLE_JOINING, false, sig), home);
    }
    enrollShow('Show this phone confirmation', 'Hold it to the terminal camera, then tap Next to read the terminal confirmation.', encodeEnrollConfirm(ROLE_JOINING, true, sig), () =>
      scan({
        heading: 'Terminal confirmation',
        hint: 'The terminal shows a last QR code once it agrees too.',
        expect: [KIND.ENROLL_CONFIRM],
        onBack: enroll,
        onDoc: async (bytes) => {
          try {
            const m = decodeEnroll(bytes);
            if (m.kind !== 'confirm' || m.role !== ROLE_EXISTING) throw new Error('not the terminal confirmation');
            if (!edVerify(r.ed25519_pub, confirmMessage(ROLE_EXISTING, m.ok, th), m.sig)) throw new Error('the terminal confirmation signature is bad');
            if (!m.ok) throw new Error('the terminal declined');
          } catch (e) {
            return refusalScreen(`Enrollment failed: ${e.message}`, []);
          }
          const id = deviceIdOf(r.ed25519_pub);
          app.state.ring[id] = { name: r.name, ed25519_pub: hexOf(r.ed25519_pub), enrolled_at: Math.floor(app.now()) };
          app.state.enrolled = { device_id: id, name: me.name, at: Math.floor(app.now()) };
          await save();
          show(title('Enrolled', `This phone is now known to ${safeText(r.name)} [${id}] as ${safeText(me.name)}.`), btn('Done', home, 'primary', 'enroll-done'));
        },
      }));
  }

  // ---------------------------------------------------------------- recovery

  function recovery() {
    show(
      title('Recovery and a new phone'),
      h('div', { class: 'card' }, h('h2', {}, 'I still have the 24 words'),
        h('p', { class: 'mut' }, 'Restoring them on a new phone gives the SAME device: no recovery is needed, nothing to revoke. Use "Restore from 24 words" on the lock screen of the new phone, then wipe this one.')),
      h('div', { class: 'card' }, h('h2', {}, 'I lost the phone and the words'),
        h('p', { class: 'mut' }, 'On a terminal start phone recovery. It needs two other attended devices or the paper phrase, a third factor, and waits seven days on every device screen. Afterwards enroll the new phone here as usual and the ring reshares to it.')),
      h('div', { class: 'card' }, h('h2', {}, 'A recovery is running and I did not start it'),
        h('p', { class: 'mut' }, 'A phone that can still sign proves the recovery is an attack. Scan the notice from a terminal and cancel it; no other factor is needed.'),
        btn('Scan a recovery notice', () => scan({ heading: 'Scan the recovery notice', hint: 'A terminal shows it on its Sync screen.', expect: [KIND.RECOVERY_NOTICE], onDoc: recoveryNotice }), 'primary', 'scan-notice')),
      back(),
    );
  }

  // The notice screen takes any document of kind 3: a phone recovery (its own flow) or a Red
  // operation that needs this phone's co-signature. Everything shown comes from the op bytes.
  function recoveryNotice(bytes) {
    let n;
    let op;
    try {
      n = decodeRecoveryNotice(bytes);
      op = parseOp(n.op_canonical);
    } catch (e) {
      return refusalScreen(`Notice is malformed: ${e.message}`, []);
    }
    if (op.kind === 'PhoneReenroll') return reenrollNotice(n, op);
    if (isRedOp(op)) return redNotice(n, op);
    return refusalScreen(`This notice carries a ${op.kind} operation, which needs nothing from this phone.`, []);
  }

  const aliasesNow = () => ({ ...app.state.aliases, ...Object.fromEntries(Object.entries(app.state.ring).map(([i, d]) => [i, d.name])) });

  // Shows ops the terminal must ingest: a QR code when it is small enough, always the paste text
  // (base64 of a kind 5 frame) for terminals without a camera, and the clipboard.
  function opsOut({ heading, hint, ops, textId = 'ops-b64' }) {
    const frame = encodePhoneOps(ops);
    const text = toBase64(frame);
    const nodes = [title(heading, hint)];
    const stops = [];
    if (frame.length <= MAX_QR_DOC_BYTES) {
      const canvas = h('canvas', { class: 'qr', id: 'reverse-qr' });
      nodes.push(canvas);
      stops.push(loopQr(canvas, reverseQrFrames(frame)).stop);
    } else nodes.push(h('p', { class: 'mut', id: 'ops-too-big' }, `${frame.length} bytes: too long for a QR code, use the text.`));
    const status = h('p', { class: 'mut', id: 'ops-status' }, `${ops.length} op(s), ${frame.length} bytes as text.`);
    nodes.push(
      h('textarea', { id: textId, readonly: true }, text), status,
      btn('Copy the text', async () => {
        try {
          await navigator.clipboard.writeText(text);
          status.textContent = 'Copied. Paste it into the terminal.';
        } catch {
          status.textContent = 'The browser refused the clipboard: select the text and copy it by hand.';
        }
      }, '', 'ops-copy'),
      btn('Done', home, 'primary', 'send-done'),
    );
    show(...nodes);
    app.cleanup.push(...stops);
    navigator.clipboard && navigator.clipboard.writeText(text).catch(() => {});
  }

  function exportOps() {
    const have = app.state.chain.length;
    show(
      title('Export my ops', 'The newest signed entries of this phone chain (a terminal takes at most 8 per delivery), as text for a terminal without a camera (paste it into the terminal) or as a QR code.'),
      h('p', { class: 'mut', id: 'export-count' }, have ? `${have} op(s) in the chain.` : 'This phone has not written anything yet.'),
      have ? btn(`The newest ${Math.min(have, COURIER_TAIL)}`, () => opsOut({ heading: 'Phone ops', hint: 'Newest entries of the chain.', ops: courierOps(app.state) }), 'primary', 'export-tail') : null,
      back(),
    );
  }

  // ---------------------------------------------------------------- reshare (C14)

  function reshareScreen() {
    const msg = h('p', { class: 'err', id: 'reshare-error' });
    show(
      title('Reshare and wrap keys', `Wrap key generation ${app.identity.wrapEpoch}. After a holder is revoked, every remaining holder rotates its wrap keys first; then the ring reshares and this phone adopts its new share here.`),
      msg,
      btn('Rotate wrap keys', async () => {
        try {
          const next = identityOfSecret(app.identity.secret, app.identity.wrapEpoch + 1);
          appendWrapKeyRotate(app.state, app.identity, next, app.now(), null);
          app.identity = next;
          await save();
          opsOut({ heading: 'Wrap keys rotated', hint: 'Deliver this to a terminal before the reshare is started: contributors wrap to the new keys. Older generations stay readable on this phone (they derive from the same 24 words).', ops: courierOps(app.state, 1) });
        } catch (e) {
          msg.textContent = e.message;
        }
      }, '', 'rotate'),
      btn('Adopt a reshare: load the epoch ops', () => scan({ heading: 'Load the reshare ops', hint: 'The terminal shows or copies the reshare ops addressed to this phone (QR code, or paste the text below).', expect: [KIND.PHONE_OPS], onBack: reshareScreen, onDoc: reshareLoad }), 'primary', 'reshare-load'),
      back(),
    );
  }

  function reshareLoad(bytes) {
    let ops;
    let epochs;
    try {
      ops = decodePhoneOps(bytes);
      epochs = ops.map((o) => parseOp(o.canonical)).filter((o) => o.kind === 'ShareReshareBatch').map((o) => BigInt(o.fields.epoch));
    } catch (e) {
      return refusalScreen(`Not a list of reshare ops: ${e.message}`, []);
    }
    if (!epochs.length) return refusalScreen('These ops carry no reshare batch.', []);
    const epoch = epochs.reduce((a, b) => (a > b ? a : b));
    const existing = ownStorageKey(app.identity, app.state.chain, epoch);
    const plan = planReshare(app.identity, ops, epoch, { ring: app.state.ring, existingStorage: existing });
    if (!plan.ok) return refusalScreen(plan.error, []);
    const msg = h('p', { class: 'err', id: 'reshare-error' });
    show(
      title('Adopt the reshare'),
      h('div', { class: 'card', id: 'reshare-summary' },
        h('div', {}, `Epoch ${epoch}, ${plan.holders.length} holders, this phone is holder number ${plan.myIndex}.`),
        h('div', {}, `Credentials ready: ${plan.items.length}; waiting for a second contributor: ${plan.skipped.length}.`),
        h('div', { class: 'mut' }, `Contributors: ${plan.contributors.map((c) => `${safeText(aliasesNow()[c.id] || c.id)} [${c.id}] (${c.verified ? 'signature verified' : 'signature not checked'}${c.session ? '' : ', no session key for this phone'})`).join('; ')}`),
        h('div', { class: 'mut' }, existing ? 'A storage key of this epoch was already published.' : 'A new storage key is published with the new shares.')),
      msg,
      btn('Publish the new shares', async () => {
        try {
          const out = publishReshare(app.state, app.identity, plan, app.now());
          await save();
          opsOut({ heading: 'Shares adopted', hint: 'Deliver these ops to a terminal. The ring marks a credential adopted only after a real unlock with the new sharing.', ops: out });
        } catch (e) {
          msg.textContent = e.message;
        }
      }, 'primary', 'reshare-publish'),
      btn('Cancel', reshareScreen, '', 'back'),
    );
  }

  function redNotice(n, op) {
    const desc = describeOp(op, aliasesNow());
    const known = app.state.ring[op.device_id];
    const sigOk = known ? edVerify(bytesOf(known.ed25519_pub), n.op_canonical, n.op_signature) : null;
    if (sigOk === false) return refusalScreen(`The signature of ${op.device_id} does not verify over this operation.`, []);
    const age = app.now() - Number(op.timestamp);
    if (age > RED_CEREMONY_TTL_SECS) return refusalScreen(`This operation is ${Math.floor(age / 86400)} days old; a Red ceremony lapses after 3 days. Ask the terminal to propose it again.`, []);
    if (age < -TIMESTAMP_SKEW_GRACE_SECS) return refusalScreen('This operation is dated in the future.', []);
    const own = op.device_id === app.identity.deviceId;
    // A trust certificate is authored by the phone itself: the phone's own signature is the proof.
    const blocked = op.kind === 'CertIssue' && !own ? 'A trust certificate must be issued by the phone itself. This one was written by another device: do not approve it.' : null;
    const msg = h('p', { class: 'err', id: 'red-error' });
    show(
      title('Red operation: needs this phone'),
      h('div', { class: 'refusal', id: 'red-banner' }, `${op.kind} proposed by ${safeText(aliasesNow()[op.device_id] || 'a device this phone has not met')} [${op.device_id}]. It takes effect only with three proofs: this phone, a second attended device and a third factor. Your signature here is the phone's proof.`),
      h('div', { class: 'headline', id: 'headline' }, desc.headline),
      descriptionRows(desc),
      h('p', { class: 'mut' }, sigOk ? 'The signature of the proposer verifies.' : 'This phone does not know the proposer, so its signature is not checked here.'),
      h('p', { class: 'mut', id: 'red-hash' }, `Operation hash ${opHashHex(n.op_canonical)}`),
      blocked ? h('p', { class: 'err', id: 'red-blocked' }, blocked) : null,
      msg,
      blocked ? null : btn('Approve: sign this operation with the phone', async () => {
        try {
          appendRedPhoneSign(app.state, app.identity, opHashHex(n.op_canonical), app.now(), { lamport: op.lamport, epoch: op.epoch });
          await save();
          opsOut({ heading: 'Signed', hint: 'Carry this to a terminal: it holds the phone proof for the operation above. Show the QR code or paste the text.', ops: courierOps(app.state) });
        } catch (e) {
          msg.textContent = e.message;
        }
      }, 'danger', 'red-approve'),
      btn('Refuse', home, '', 'refuse'),
    );
  }

  function reenrollNotice(n, op) {
    const desc = describeOp(op, aliasesNow());
    const known = app.state.ring[op.device_id];
    const sigOk = known ? edVerify(bytesOf(known.ed25519_pub), n.op_canonical, n.op_signature) : null;
    if (sigOk === false) return refusalScreen(`The signature of ${op.device_id} does not verify over this notice.`, []);
    const ends = Number(op.fields.initiated_at) + RECOVERY_TIMELOCK_SECS;
    const msg = h('p', { class: 'err', id: 'cancel-error' });
    show(
      title('Phone recovery in progress'),
      h('div', { class: 'headline', id: 'headline' }, desc.headline),
      descriptionRows(desc),
      h('p', { class: 'mut', id: 'timelock' }, `The ring accepts the new phone after ${new Date(ends * 1000).toISOString().slice(0, 16)}Z unless it is cancelled.`),
      h('p', { class: 'mut' }, sigOk ? 'The signature verifies.' : 'This phone does not know the signing device, so the signature is not checked here.'),
      msg,
      btn('Cancel this recovery: I still have this phone', async () => {
        try {
          appendReenrollCancel(app.state, app.identity, opHashHex(n.op_canonical), app.now(), { lamport: op.lamport, epoch: op.epoch });
          await save();
          opsOut({ heading: 'Cancellation signed', hint: 'Show the QR code to any terminal, or paste the text into it. It carries the cancellation and the head of this phone chain.', ops: courierOps(app.state), textId: 'cancel-b64' });
        } catch (e) {
          msg.textContent = e.message;
        }
      }, 'danger', 'cancel-recovery'),
      btn('Leave it (I started this)', home, '', 'back'),
    );
  }

  await boot();
  return app;
}
