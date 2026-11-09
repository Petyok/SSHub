//! The phone's terminal side as verbs: thin wrappers over the calls the TUI uses
//! (`sync::phone_enroll`, `sync::wire`, `sync::hub`, `sync::unlock_paths`, `sync::cert`), so a
//! script or a headless machine can enrol a phone, show it a Red op, take its ops in and answer a
//! share challenge. Frames travel as text: base64 of the frame, exactly what the phone page
//! copies and accepts. Plain output is one `key: value` line per item; `--format json` writes one
//! JSON object per line (events) or one object (single results).

use std::io::{BufRead, Write};
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::{json, Value};

use super::sync_phone_ble::BleLink;
use super::theme::sanitize_foreign as plain;
use crate::profile::ProfilePaths;
use crate::store::LauncherStore;
use crate::sync::crypto::DeviceIdentity;
use crate::sync::identity_store::load_or_init_identity;
use crate::sync::phone_enroll::{admit_phone, PhoneEnroll, Step};

pub const COMMANDS: [&str; 7] = [
    "sync-phone-enroll",
    "sync-notice",
    "sync-phone-ops",
    "sync-phone-chain",
    "sync-share-request",
    "sync-cert-request",
    "sync-trust-renew",
];

pub fn is_phone_command(cmd: &str) -> bool {
    COMMANDS.contains(&cmd)
}

const EMPTY_RING: &str = "No sync ring on this profile yet.";

/// A line the verbs print: its plain form and its JSON form.
pub(super) fn emit(
    out: &mut impl Write,
    json_out: bool,
    event: &str,
    text: String,
    mut doc: Value,
) {
    let _ = if json_out {
        doc["event"] = json!(event);
        writeln!(out, "{doc}")
    } else {
        writeln!(out, "{event}: {text}")
    };
    let _ = out.flush();
}

/// The fingerprint of the phone page this build ships, FIRST: comparing it with the phone's
/// About screen is the control against a substituted page.
fn companion_line(out: &mut impl Write, json_out: bool) {
    let fp = crate::sync::companion::COMPANION_FINGERPRINT;
    emit(
        out,
        json_out,
        "companion",
        format!("{fp} ({})", crate::sync::companion::COMPARE_HINT),
        json!({"fingerprint": fp}),
    );
}

pub(super) fn b64(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

/// Frame text from the phone (whitespace ignored), or an error that leaves the session going.
fn unb64(line: &str) -> Result<Vec<u8>> {
    B64.decode(line.split_whitespace().collect::<String>())
        .context("that is not base64: paste the frame as the phone shows it")
}

// ------------------------------------------------------------- enrollment

/// One enrollment of a phone as a text session. `feed` takes the lines the user (or a script)
/// types; the answer to the four words is `y` or `n`, every other line is a phone frame.
pub struct EnrollSession<'a> {
    store: &'a LauncherStore,
    id: &'a DeviceIdentity,
    enroll: PhoneEnroll,
    first: Vec<u8>,
    comparing: bool,
    pub done: Option<bool>,
    json: bool,
    ble: Option<&'a BleLink>,
    /// `[sync.notify]` of this profile, for the mail hint after the phone is admitted.
    notify: Option<crate::config::NotifyConfig>,
    /// `--replace`: the verified phone is the NEW phone of a handover, not an addition.
    replace: bool,
}

impl<'a> EnrollSession<'a> {
    pub fn new(store: &'a LauncherStore, id: &'a DeviceIdentity, name: &str, json: bool) -> Self {
        let (enroll, first) = PhoneEnroll::begin(id, name);
        Self {
            store,
            id,
            enroll,
            first,
            comparing: false,
            done: None,
            json,
            ble: None,
            notify: None,
            replace: false,
        }
    }

    /// A handover: the new phone is verified here, then the live phone is shown a request to hand the ring over.
    pub fn with_replace(mut self) -> Self {
        self.replace = true;
        self
    }

    /// The profile's mail settings, so the hint after admission says whether a recovery is mailed.
    pub fn with_notify(mut self, notify: Option<crate::config::NotifyConfig>) -> Self {
        self.notify = notify;
        self
    }

    /// Also carry the frames over Bluetooth (`--ble`).
    pub fn with_ble(mut self, ble: &'a BleLink) -> Self {
        self.ble = Some(ble);
        self
    }

    /// The page fingerprint, then the first frame for the phone to read.
    pub fn start(&self, out: &mut impl Write) {
        companion_line(out, self.json);
        self.frame(out, &self.first, false);
    }

    /// Print a frame for the phone; with `--ble` it also goes to the connected phone page.
    fn frame(&self, out: &mut impl Write, frame: &[u8], last: bool) {
        let mut doc = json!({"frame": b64(frame)});
        if last {
            doc["final"] = json!(true);
        }
        emit(out, self.json, "frame", b64(frame), doc);
        if let Some(ble) = self.ble {
            ble.send(frame);
        }
    }

    pub fn feed(&mut self, line: &str, out: &mut impl Write) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let step =
            if self.comparing && matches!(line.to_lowercase().as_str(), "y" | "yes" | "n" | "no") {
                self.comparing = false;
                self.enroll
                    .answer(self.id, matches!(line.to_lowercase().as_str(), "y" | "yes"))
            } else {
                unb64(line).and_then(|bytes| self.enroll.scan(self.id, &bytes))
            };
        match step {
            Ok(step) => self.show(step, out),
            Err(e) => emit(
                out,
                self.json,
                "error",
                format!("{e:#}"),
                json!({"error": format!("{e:#}")}),
            ),
        }
    }

    fn show(&mut self, step: Step, out: &mut impl Write) {
        let j = self.json;
        match step {
            Step::Show(frame) => self.frame(out, &frame, false),
            Step::Compare { words, phone_name } => {
                self.comparing = true;
                let name = plain(&phone_name);
                emit(
                    out,
                    j,
                    "words",
                    format!("{words} (the phone calls itself {name}; answer y if the phone shows the same four words, n if not)"),
                    json!({"words": words, "phone_name": phone_name}),
                );
            }
            Step::Wait => emit(
                out,
                j,
                "wait",
                "the phone's confirmation is next".into(),
                json!({}),
            ),
            Step::Verified(v) if self.replace => match handover_offer(self.store, self.id, &v) {
                Ok(offer) => {
                    self.frame(out, &v.frame, true);
                    let req = b64(&offer.request);
                    emit(out, j, "handover", req.clone(), json!({"request": req}));
                    emit(
                        out,
                        j,
                        "handover_words",
                        offer.words.join(" "),
                        json!({"words": offer.words, "name": v.name}),
                    );
                    for l in handover_steps() {
                        emit(out, j, "next", l.clone(), json!({"line": l}));
                    }
                    self.done = Some(true);
                }
                Err(e) => {
                    emit(
                        out,
                        j,
                        "error",
                        format!("{e:#}"),
                        json!({"error": format!("{e:#}")}),
                    );
                    self.done = Some(false);
                }
            },
            Step::Verified(v) => {
                let admitted = admit_phone(self.store, self.id, &v).and_then(|op| {
                    super::sync_admin::op_report(
                        self.store,
                        &op,
                        format!("{} is enrolled; its admission is proposed", plain(&v.name)),
                        crate::store::now_ts(),
                    )
                });
                match admitted {
                    Ok(r) => {
                        self.frame(out, &v.frame, true);
                        for l in &r.lines {
                            emit(out, j, "admitted", l.clone(), json!({"line": l}));
                        }
                        let (configured, hint) =
                            super::sync_admin::notify_hint(self.notify.as_ref());
                        for l in hint {
                            emit(out, j, "notify_hint", l, json!({"configured": configured}));
                        }
                        self.done = Some(true);
                    }
                    Err(e) => {
                        emit(
                            out,
                            j,
                            "error",
                            format!("{e:#}"),
                            json!({"error": format!("{e:#}")}),
                        );
                        self.done = Some(false);
                    }
                }
            }
            Step::Stopped { why, frame } => {
                if let Some(f) = frame {
                    self.frame(out, &f, true);
                }
                emit(out, j, "stopped", plain(&why), json!({"why": why}));
                self.done = Some(false);
            }
        }
    }
}

/// What the terminal shows for a handover (design 5): the kind 12 request for the LIVE phone to scan and the eight words
/// it will show next to the new phone's name.
pub(crate) struct HandoverOffer {
    pub request: Vec<u8>,
    pub words: Vec<String>,
}

/// Build the offer for the verified new phone. The ring must have a live phone; the new phone is NOT registered here:
/// the handover op the old phone signs adds it, once the ceremony completes.
pub(crate) fn handover_offer(
    store: &LauncherStore,
    id: &DeviceIdentity,
    v: &crate::sync::phone_enroll::Verified,
) -> Result<HandoverOffer> {
    let old = crate::sync::quorum::phone_device(store)?
        .context("the ring has no phone to hand over from: enroll the phone without --replace")?;
    ensure!(
        v.phone.device_id() != old,
        "that is the phone the ring already has"
    );
    let terminal: [u8; 8] = hex::decode(id.public().device_id())
        .ok()
        .and_then(|b| b.try_into().ok())
        .context("this terminal's device id is not 16 hex characters")?;
    let old_id: [u8; 8] = hex::decode(&old)
        .ok()
        .and_then(|b| b.try_into().ok())
        .context("the phone's device id is not 16 hex characters")?;
    let words = crate::sync::crypto::handover_words(&v.phone, &v.name, &old_id);
    let request = crate::sync::wire::PhoneHandoverRequestWire {
        new_phone: v.phone.clone(),
        name: v.name.clone(),
        terminal,
        op: None,
    }
    .encode();
    Ok(HandoverOffer { request, words })
}

/// Stage 2 of kind 12 (contract 4l b): the frame for the NEW phone once the terminal ingested the `PhoneHandover` op, the
/// request layout plus the op's hash and lamport, and the same eight words the old phone showed.
pub(crate) fn handover_admission(
    op: &crate::sync::oplog::Op,
    old_phone: &str,
    terminal: [u8; 8],
) -> Result<HandoverOffer> {
    let crate::sync::oplog::OpBody::PhoneHandover { new_phone, name } = &op.body else {
        bail!("that is not a phone handover");
    };
    let old_id: [u8; 8] = hex::decode(old_phone)
        .ok()
        .and_then(|b| b.try_into().ok())
        .context("the phone's device id is not 16 hex characters")?;
    let request = crate::sync::wire::PhoneHandoverRequestWire {
        new_phone: new_phone.clone(),
        name: name.clone(),
        terminal,
        op: Some((op.hash(), op.lamport)),
    }
    .encode();
    Ok(HandoverOffer {
        request,
        words: crate::sync::crypto::handover_words(new_phone, name, &old_id),
    })
}

/// One `handover_admission:` frame per handover op still waiting, for the phone it adds; empty when the ring has no phone.
fn admission_lines(store: &LauncherStore, id: &DeviceIdentity) -> Vec<(String, String)> {
    let Ok(Some(old)) = crate::sync::quorum::phone_device(store) else {
        return Vec::new();
    };
    let Some(terminal) = hex::decode(id.public().device_id())
        .ok()
        .and_then(|b| <[u8; 8]>::try_from(b).ok())
    else {
        return Vec::new();
    };
    store
        .sync_pending()
        .unwrap_or_default()
        .iter()
        .filter_map(|op| handover_admission(op, &old, terminal).ok())
        .map(|o| (b64(&o.request), o.words.join(" ")))
        .collect()
}

/// What happens after the request is shown (plain steps for the person at the terminal).
pub(crate) fn handover_steps() -> Vec<String> {
    vec![
        "1. Show the request to the OLD phone (it must still work): it shows the new phone's name and the eight words above; they must match, then it signs the handover.".to_string(),
        "2. Bring the old phone's answer here with `sshub sync-phone-ops`: it then prints `handover_admission:`, a second kind 12 frame for the NEW phone (with the same eight words), which signs its own admission; bring that answer in the same way.".to_string(),
        "3. Approve the handover with `sshub sync-approve <change-id>` on another computer and add the paper phrase with `sshub sync-factor <change-id> --paper`. Nothing changes before all three.".to_string(),
        "4. Then run `sshub sync-heal`: the new phone holds your old saved passwords only after that. Retire the old phone yourself.".to_string(),
    ]
}

/// What a kind 5 list says about the ring's terminals (the `terminals` block, unsigned and informational), in plain
/// words; a plain fallback when the sender did not say.
pub(crate) fn terminals_line(t: Option<&crate::sync::wire::TerminalsBlock>, now: i64) -> String {
    match t {
        Some(t) => format!(
            "This list says the ring has {} terminal{}; the newest change it names is {} old. The phone shows this when you accept one list.",
            t.count,
            if t.count == 1 { "" } else { "s" },
            crate::sync::health::human_left(now - t.newest_op_ts)
        ),
        None => "This list does not say how many terminals the ring has.".to_string(),
    }
}

fn phone_enroll(
    store: &LauncherStore,
    id: &DeviceIdentity,
    input: impl BufRead,
    out: &mut impl Write,
    json_out: bool,
    notify: Option<crate::config::NotifyConfig>,
    replace: bool,
) -> Result<i32> {
    let mut s = EnrollSession::new(store, id, &crate::sync::runtime::device_name(), json_out)
        .with_notify(notify);
    if replace {
        s = s.with_replace();
    }
    s.start(out);
    for line in input.lines() {
        s.feed(&line?, out);
        if let Some(ok) = s.done {
            return Ok(if ok { 0 } else { 1 });
        }
    }
    bail!("the input ended before the phone was verified")
}

// ----------------------------------------------------------------- notice

/// The kind 3 text for the Red op whose id starts with `prefix`.
pub fn notice_text(store: &LauncherStore, prefix: &str) -> Result<(String, String)> {
    let op = crate::sync::hub::op_by_hash_prefix(store, prefix)?;
    ensure!(
        crate::sync::ceremony::is_gated(store, &op)?,
        "that change is not a big change: the phone has nothing to approve"
    );
    let hash: String = op.hash().iter().map(|b| format!("{b:02x}")).collect();
    Ok((hash, b64(&crate::sync::hub::op_notice(&op).encode())))
}

// -------------------------------------------------------------- phone ops

/// Ingest the phone's kind 5 text; what applied, and the ceremony of every Red op it touches.
pub fn phone_ops(
    store: &LauncherStore,
    text: &str,
) -> Result<(crate::sync::protocol::RoundSummary, Vec<String>)> {
    let summary = crate::sync::unlock_paths::phone_ops_from_text(store, text)?;
    let mut ceremony = Vec::new();
    let now = crate::store::now_ts();
    for op in store.sync_pending()? {
        if crate::sync::ceremony::is_gated(store, &op)? {
            let st = crate::sync::ceremony::red_status(store, &op, now)?;
            ceremony.push(format!(
                "{}: {}",
                super::sync_cmd::op_id(&op.hash()),
                if st.complete { "complete" } else { "waiting" }
            ));
        }
    }
    Ok((summary, ceremony))
}

/// Why ops of a pasted list were not applied, one entry per reason the ingest distinguishes:
/// `chain-gap` (per author: first refused seq), `already-known`, `invalid` (the first reasons).
pub fn skipped_ops(
    store: &LauncherStore,
    s: &crate::sync::protocol::RoundSummary,
) -> Result<Vec<Value>> {
    let heads = store.sync_chain_heads()?;
    let mut out = Vec::new();
    for (dev, first, count) in &s.gaps {
        let have = heads.iter().find(|h| &h.0 == dev).map_or(0, |h| h.1);
        let note = if have == 0 {
            "this ring has none of that device's earlier changes; export the device's full history"
                .to_string()
        } else {
            format!(
                "this ring has that device's chain only up to seq {have}; export its full chain"
            )
        };
        out.push(json!({"reason": "chain-gap", "device": dev, "first_seq": first, "count": count, "have_seq": have, "note": note}));
    }
    if s.duplicates > 0 {
        out.push(json!({"reason": "already-known", "count": s.duplicates, "note": "this ring already has these changes"}));
    }
    let gap_ops: usize = s.gaps.iter().map(|g| g.2).sum();
    if s.rejected > gap_ops {
        out.push(json!({"reason": "invalid", "count": s.rejected - gap_ops, "details": s.reject_reasons.iter().filter(|r| !r.starts_with("gap:")).collect::<Vec<_>>(), "note": "these changes were refused"}));
    }
    Ok(out)
}

fn skipped_line(k: &Value) -> String {
    let reason = k["reason"].as_str().unwrap_or("");
    let mut line = format!("skipped {} {reason}", k["count"]);
    if let Some(f) = k["first_seq"].as_u64() {
        line.push_str(&format!(" (first seq {f})"));
    }
    if let Some(d) = k["details"].as_array().filter(|d| !d.is_empty()) {
        let d: Vec<&str> = d.iter().filter_map(Value::as_str).collect();
        line.push_str(&format!(": {}", d.join("; ")));
    }
    format!("{line}: {}", k["note"].as_str().unwrap_or(""))
}

// ------------------------------------------------------------ share reply

/// The challenge for `host` as text, plus what is needed to open the phone's reply.
pub struct ShareChallenge {
    pub text: String,
    /// The same challenge as raw bytes (what Bluetooth carries).
    pub frame: Vec<u8>,
    challenge: crate::sync::quorum::Challenge,
    eph: zeroize::Zeroizing<[u8; 32]>,
    phone_ed25519: [u8; 32],
}

impl ShareChallenge {
    pub fn new(
        store: &LauncherStore,
        id: &DeviceIdentity,
        host: &crate::store::HostUid,
    ) -> Result<Self> {
        let phone = crate::sync::quorum::phone_device(store)?.context("the ring has no phone")?;
        let phone_ed25519 = store
            .sync_devices()?
            .into_iter()
            .find(|d| d.id == phone)
            .map(|d| d.public.ed25519_pub)
            .context("the phone is not in the ring")?;
        let (challenge, eph) = crate::sync::quorum::make_challenge(
            store,
            id,
            host,
            &phone,
            crate::store::now_ts(),
            crate::sync::limits::CHALLENGE_EXPIRY_SECS,
        )?;
        Ok(Self {
            frame: challenge.to_wire().encode(),
            text: b64(&challenge.to_wire().encode()),
            challenge,
            eph,
            phone_ed25519,
        })
    }

    /// Open the phone's kind 2 text. The share is what matters: its courier ops and snapshot are
    /// taken in on the way, and a failure of that is returned as a warning, not an error (the
    /// share was received and is valid; the TUI reports it the same way).
    pub fn open(
        &self,
        store: &LauncherStore,
        reply: &str,
    ) -> Result<(crate::sync::crypto::Share, Option<String>)> {
        let r = crate::sync::unlock_paths::open_phone_reply_clipboard(
            reply,
            &self.challenge,
            &self.eph,
            &self.phone_ed25519,
        )?;
        let warning = r.ext.as_ref().and_then(|ext| {
            crate::sync::unlock_paths::apply_reply_ext(store, ext)
                .err()
                .map(|e| format!("reply accepted; the phone's own changes were not kept: {e:#}"))
        });
        Ok((r.share, warning))
    }
}

/// A share the phone already gave, as the unlocker's second share.
struct GivenShare(crate::sync::crypto::Share);

impl crate::sync::quorum::ShareSource for GivenShare {
    fn master_phrase(&self) -> Option<zeroize::Zeroizing<String>> {
        None
    }

    fn second_share(
        &self,
        _host: &crate::store::HostUid,
        _epoch: u64,
        _mine: &crate::sync::crypto::Share,
    ) -> std::result::Result<crate::sync::crypto::Share, crate::credentials::UnlockError> {
        Ok(self.0.clone())
    }
}

/// One line from `input`, or `None` after `wait`.
fn line_within(input: impl BufRead + Send + 'static, wait: Duration) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut input = input;
        let mut line = String::new();
        let _ = input.read_line(&mut line);
        let _ = tx.send(line);
    });
    rx.recv_timeout(wait).ok().filter(|l| !l.trim().is_empty())
}

// -------------------------------------------------------------- arguments

struct Args {
    json: bool,
    wait: u64,
    days: u32,
    budget: u32,
    group: Option<String>,
    hosts: Vec<String>,
    pos: Vec<String>,
    /// Carry the frames over Bluetooth too (feature `bluetooth`).
    ble: bool,
    /// `sync-phone-enroll --replace`: hand the ring over to a new phone.
    replace: bool,
}

fn parse(args: &[String]) -> std::result::Result<Args, String> {
    let mut a = Args {
        json: false,
        wait: crate::sync::limits::CHALLENGE_EXPIRY_SECS as u64,
        days: 30,
        budget: 10,
        group: None,
        hosts: Vec::new(),
        pos: Vec::new(),
        ble: false,
        replace: false,
    };
    let mut i = 0;
    let value = |i: usize, flag: &str| {
        args.get(i + 1)
            .filter(|v| !v.starts_with("--"))
            .cloned()
            .ok_or_else(|| format!("{flag} requires a value"))
    };
    while i < args.len() {
        let f = args[i].as_str();
        match f {
            "--format" => {
                a.json = match value(i, f)?.as_str() {
                    "plain" => false,
                    "json" => true,
                    other => return Err(format!("unknown format '{other}'")),
                };
                i += 2;
            }
            "--wait" => {
                a.wait = value(i, f)?
                    .parse::<u64>()
                    .map_err(|_| "--wait takes seconds".to_string())?
                    .min(crate::sync::limits::CHALLENGE_EXPIRY_SECS as u64);
                i += 2;
            }
            "--days" => {
                a.days = value(i, f)?
                    .parse()
                    .map_err(|_| "--days takes a number".to_string())?;
                i += 2;
            }
            "--budget" => {
                a.budget = value(i, f)?
                    .parse()
                    .map_err(|_| "--budget takes a number".to_string())?;
                i += 2;
            }
            "--group" => {
                a.group = Some(value(i, f)?);
                i += 2;
            }
            "--replace" => {
                a.replace = true;
                i += 1;
            }
            "--ble" => {
                a.ble = true;
                i += 1;
            }
            "--host" => {
                a.hosts.push(value(i, f)?);
                i += 2;
            }
            _ if f.starts_with('-') => return Err(format!("unknown option '{f}'")),
            _ => {
                a.pos.push(f.to_string());
                i += 1;
            }
        }
    }
    Ok(a)
}

// -------------------------------------------------------------------- run

pub fn run(
    cmd: &str,
    args: &[String],
    resolve: impl FnOnce() -> Result<ProfilePaths>,
) -> Result<i32> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help(cmd);
        return Ok(0);
    }
    let a = match parse(args) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("sshub: {msg}\n       run `sshub {cmd} --help` for usage");
            return Ok(2);
        }
    };
    let want = |n: usize, what: &str| -> Option<i32> {
        (a.pos.len() != n).then(|| {
            eprintln!("sshub: {cmd} needs {what}\n       run `sshub {cmd} --help` for usage");
            2
        })
    };
    let bad = match cmd {
        "sync-phone-enroll" | "sync-phone-ops" => want(0, "no argument"),
        "sync-notice" => want(1, "one change id"),
        "sync-phone-chain" => want(1, "one phone device id"),
        "sync-share-request" => want(1, "one host name"),
        "sync-cert-request" => want(0, "no argument (use --group or --host)"),
        "sync-trust-renew" if a.pos.len() > 1 => want(1, "at most one trust session id"),
        _ if a.replace && cmd != "sync-phone-enroll" => {
            eprintln!("sshub: --replace belongs to sync-phone-enroll\n       run `sshub {cmd} --help` for usage");
            Some(2)
        }
        _ => None,
    };
    if let Some(code) = bad {
        return Ok(code);
    }
    let paths = resolve()?;
    match execute(cmd, &a, &paths) {
        Ok(code) => Ok(code),
        Err(e) => {
            eprintln!("sshub: {cmd}: {e:#}");
            Ok(1)
        }
    }
}

fn execute(cmd: &str, a: &Args, paths: &ProfilePaths) -> Result<i32> {
    let store = LauncherStore::open(paths.launcher_db()).context("open the launcher database")?;
    ensure!(!store.sync_devices()?.is_empty(), "{EMPTY_RING}");
    let secrets = crate::credentials::open_for_profile(paths);
    let id = load_or_init_identity(paths, secrets.as_ref())?;
    let mut out = std::io::stdout();
    let notify = crate::config::load_config_at(&paths.config_file)
        .ok()
        .map(|c| c.sync.notify);
    // Every verb that talks to the page prints its fingerprint first; the enrollment session
    // does it itself so that its output starts with it.
    if matches!(
        cmd,
        "sync-notice" | "sync-phone-chain" | "sync-share-request" | "sync-cert-request"
    ) || (cmd == "sync-trust-renew" && !a.pos.is_empty())
    {
        companion_line(&mut out, a.json);
    }
    match cmd {
        "sync-phone-enroll" if a.ble => {
            super::sync_phone_ble::enroll(&store, &id, &mut out, a.json, notify, a.replace)
        }
        "sync-phone-enroll" => phone_enroll(
            &store,
            &id,
            std::io::stdin().lock(),
            &mut out,
            a.json,
            notify,
            a.replace,
        ),
        "sync-notice" => {
            let (op, text) = notice_text(&store, &a.pos[0])?;
            emit(
                &mut out,
                a.json,
                "notice",
                text.clone(),
                json!({"op": op, "notice": text}),
            );
            if a.ble {
                return super::sync_phone_ble::push(&unb64(&text)?, a.json, a.wait, &mut out);
            }
            Ok(0)
        }
        "sync-phone-chain" => {
            let me = id.public().device_id();
            let wire = crate::sync::hub::phone_chain_wire(&store, &a.pos[0], &me)?;
            let text = b64(&wire.encode());
            emit(
                &mut out,
                a.json,
                "chain",
                text.clone(),
                json!({
                    "chain": text, "sender": me, "ops": wire.ops.len(),
                    "terminals": wire.terminals.map(|t| json!({"count": t.count, "newest_op_ts": t.newest_op_ts})),
                }),
            );
            if !a.json {
                emit(
                    &mut out,
                    false,
                    "terminals",
                    terminals_line(wire.terminals.as_ref(), crate::store::now_ts()),
                    json!({}),
                );
            }
            if a.ble {
                return super::sync_phone_ble::push(&wire.encode(), a.json, a.wait, &mut out);
            }
            Ok(0)
        }
        "sync-phone-ops" => {
            let text = if a.ble {
                super::sync_phone_ble::ops_text(a.json, a.wait, &mut out)?
            } else {
                let mut text = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
                text
            };
            phone_ops_cmd(&store, &id, &text, a.json, &mut out)
        }
        "sync-share-request" => share_request(paths, &store, &id, a, &mut out),
        "sync-cert-request" => {
            let me = id.public().device_id();
            let (labels, scopes) = crate::app::sync_trust::scope_choices(&store);
            let scope = pick_scope(&labels, &scopes, a)?;
            let wire = crate::sync::cert::cert_request(
                &store,
                &me,
                scope,
                i64::from(a.days) * 86_400,
                a.budget,
                1,
                crate::store::now_ts(),
            )?;
            let text = b64(&wire.encode());
            emit(
                &mut out,
                a.json,
                "request",
                text.clone(),
                json!({"request": text}),
            );
            if a.ble {
                return super::sync_phone_ble::push(&wire.encode(), a.json, a.wait, &mut out);
            }
            Ok(0)
        }
        "sync-trust-renew" => trust_renew(&store, &id, a, &mut out),
        _ => unreachable!("dispatched by is_phone_command"),
    }
}

/// `sync-trust-renew [<id>] [--days N]`: without an id, the trust sessions that can be renewed; with one, the kind 11
/// request the phone confirms. Built on a terminal that is not the delegate (the evidence comes from the others).
fn trust_renew(
    store: &LauncherStore,
    id: &DeviceIdentity,
    a: &Args,
    out: &mut impl Write,
) -> Result<i32> {
    let now = crate::store::now_ts();
    let devices: Vec<(String, String)> = crate::sync::ceremony::effective_devices(store, None)?
        .into_iter()
        .map(|d| (d.id, d.name))
        .collect();
    let rows = crate::app::sync_trust::trust_rows(store, &devices, now);
    let Some(prefix) = a.pos.first() else {
        for r in &rows {
            let line = format!(
                "{} {}: ends in {}, renewed {} of {} times",
                &hex_of(&r.cert_id)[..8],
                plain(&r.delegate),
                crate::sync::health::human_left(r.expiry - now),
                r.renewals,
                crate::sync::limits::CERT_MAX_RENEWALS
            );
            emit(
                out,
                a.json,
                "session",
                line,
                json!({"id": hex_of(&r.cert_id), "delegate": r.delegate, "expiry": r.expiry, "renewals": r.renewals}),
            );
        }
        return Ok(0);
    };
    let hits: Vec<_> = rows
        .iter()
        .filter(|r| hex_of(&r.cert_id).starts_with(&prefix.to_lowercase()))
        .collect();
    ensure!(
        prefix.len() >= 6,
        "give at least 6 hex digits of the trust session id"
    );
    let [row] = hits.as_slice() else {
        bail!(
            "{prefix} matches {} trust sessions in force (list them with no argument)",
            hits.len()
        );
    };
    ensure!(
        row.delegate_id != id.public().device_id(),
        "this computer is the delegate: renew it from another full computer, whose view is the evidence"
    );
    let wire =
        crate::sync::cert::cert_renew_request(store, row.cert_id, i64::from(a.days) * 86_400, now)?;
    emit(
        out,
        a.json,
        "opened",
        crate::app::sync_trust::opened_line(&wire),
        json!({"opened": wire.opened, "opened_total": wire.opened_total}),
    );
    let text = b64(&wire.encode());
    emit(
        out,
        a.json,
        "request",
        text.clone(),
        json!({"request": text}),
    );
    if a.ble {
        return super::sync_phone_ble::push(&wire.encode(), a.json, a.wait, out);
    }
    Ok(0)
}

/// `sync-phone-ops` after the phone's list was read: ingest it and print what applied, what was skipped, the ceremony of
/// every Red op and, for a pending handover, the second kind 12 frame for the new phone.
fn phone_ops_cmd(
    store: &LauncherStore,
    id: &DeviceIdentity,
    text: &str,
    json: bool,
    out: &mut impl Write,
) -> Result<i32> {
        let (s, ceremony) = phone_ops(store, text)?;
        let skipped = skipped_ops(store, &s)?;
        emit(
            out,
            json,
            "ops",
            format!(
                "{} received, {} applied, {} waiting for approval",
                s.received, s.applied, s.pending_arrived
            ),
            json!({"received": s.received, "applied": s.applied, "pending": s.pending_arrived, "skipped": skipped, "red": ceremony}),
        );
        if !json {
            for k in &skipped {
                emit(out, false, "skipped", skipped_line(k), json!({}));
            }
            for c in &ceremony {
                emit(out, false, "red", c.clone(), json!({}));
            }
        }
        // Contract 4l (b): the second kind 12 frame, for the NEW phone.
        for (frame, words) in admission_lines(store, id) {
            emit(
                out,
                json,
                "handover_admission",
                frame.clone(),
                json!({"frame": frame, "words": words}),
            );
            if !json {
                emit(out, false, "handover_words", words, json!({}));
            }
        }
    Ok(0)
}

fn hex_of(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The trust-session scope from `--group NAME` or one or more `--host NAME`, by the labels the
/// TUI's scope step offers.
fn pick_scope(
    labels: &[String],
    scopes: &[crate::sync::oplog::CertScope],
    a: &Args,
) -> Result<crate::sync::oplog::CertScope> {
    use crate::sync::oplog::CertScope;
    match (&a.group, a.hosts.is_empty()) {
        (Some(g), true) => {
            let want = format!("group {g} (");
            let i = labels
                .iter()
                .position(|l| l.starts_with(&want))
                .with_context(|| format!("no group named {g}"))?;
            Ok(scopes[i].clone())
        }
        (None, false) => {
            let mut uids = Vec::new();
            for h in &a.hosts {
                let want = format!("host {h}");
                let i = labels
                    .iter()
                    .position(|l| *l == want)
                    .with_context(|| format!("no host named {h}"))?;
                if let CertScope::Hosts(u) = &scopes[i] {
                    uids.extend(u.iter().copied());
                }
            }
            ensure!(
                uids.len() <= crate::sync::wire::MAX_CERT_REQUEST_HOSTS,
                "a trust session lists at most {} hosts",
                crate::sync::wire::MAX_CERT_REQUEST_HOSTS
            );
            uids.sort_unstable();
            Ok(CertScope::Hosts(uids))
        }
        _ => bail!("give --group NAME or one or more --host NAME (never everything)"),
    }
}

fn share_request(
    paths: &ProfilePaths,
    store: &LauncherStore,
    id: &DeviceIdentity,
    a: &Args,
    out: &mut impl Write,
) -> Result<i32> {
    let ctx = super::context::CliContext::bootstrap_with(paths.clone())?;
    let host = ctx.managed_host_by_name(&a.pos[0])?;
    let uid = store
        .host_uid(host.id)?
        .context("that host has no synced identity yet")?;
    let ch = ShareChallenge::new(store, id, &uid)?;
    let shown = ch.challenge.shown()?;
    emit(
        out,
        a.json,
        "host",
        format!("{} {}", plain(&host.name), shown.host_id),
        json!({"host": host.name, "host_id": shown.host_id, "host_uid": shown.host_uid}),
    );
    emit(
        out,
        a.json,
        "words",
        format!(
            "{} (compare the host id and these words with the phone before approving)",
            shown.words
        ),
        json!({"words": shown.words}),
    );
    emit(
        out,
        a.json,
        "challenge",
        ch.text.clone(),
        json!({"challenge": ch.text, "host": host.name, "host_id": shown.host_id, "host_uid": shown.host_uid, "words": shown.words}),
    );
    let line = if a.ble {
        let ble = BleLink::new(a.json, crate::sync::media::ListenOpts::SHARE_RESPONSE)?;
        ble.announce(out);
        ble.send(&ch.frame);
        super::sync_phone_ble::wait_reply(&ble, Duration::from_secs(a.wait), out)
    } else {
        line_within(
            std::io::BufReader::new(std::io::stdin()),
            Duration::from_secs(a.wait),
        )
    }
    .with_context(|| format!("no reply from the phone within {} s", a.wait))?;
    let (share, warning) = ch.open(store, &line)?;
    if let Some(w) = warning {
        emit(out, a.json, "warning", plain(&w), json!({"warning": w}));
    }
    let unlocker = super::unlock::unlocker_with(&ctx, Box::new(GivenShare(share)))?;
    let key = crate::credentials::host_key(host.id);
    crate::credentials::ensure_unlocked(&[crate::credentials::SecretRef::Key(key)], &unlocker)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    emit(
        out,
        a.json,
        "opened",
        format!(
            "{} (the credential is open in this process only)",
            plain(&host.name)
        ),
        json!({"host": host.name, "opened": true}),
    );
    Ok(0)
}

// ------------------------------------------------------------------- help

pub fn print_help(cmd: &str) {
    let (synopsis, text) = match cmd {
        "sync-phone-enroll" => ("sync-phone-enroll [--replace] [--ble] [--format plain|json]", "With --replace the verified phone is the NEW phone of a handover: the ring keeps working with the old phone, and the terminal prints the kind 12 request for the OLD phone (`handover:`), the eight words to compare (`handover_words:`) and the next steps (`next:`) instead of adding the phone. Enroll a phone from this terminal, as text. Prints the terminal's first frame (`frame:`); read each of the phone's frames (kind 6, 7, 8, base64) from stdin, one per line, and print the next frame to show. After the phone's reveal it prints the four words (`words:`): answer `y` or `n` on stdin. On a verified phone it registers it and proposes its admission (a Red change) and prints the last frame for the phone. Needs a ring. Exit 0 verified, 1 stopped or input ended. With --ble (a build with the `bluetooth` feature) the frames also travel over Bluetooth: it prints `bluetooth: pick SSHub-XXXX in the phone app's device list`, sends each frame it prints to the phone app and takes the phone's frames from Bluetooth or stdin; the four words are still answered on stdin."),
        "sync-notice" => ("sync-notice <op-id> [--wait <seconds>] [--ble] [--format plain|json]", "Print the kind 3 notice (base64) of a Red op, for the phone to read: the op as signed, so the phone can show what it is asked to approve. <op-id> is a unique prefix (4+ hex characters) of the op's hash. With --ble (a build with the `bluetooth` feature) the notice is also handed to the phone app over Bluetooth: it prints `bluetooth: pick SSHub-XXXX in the phone app's device list` and waits up to --wait seconds until the phone has the frame (`sent:`)."),
        "sync-phone-chain" => ("sync-phone-chain <phone-device-id> [--wait <seconds>] [--ble] [--format plain|json]", "Print the kind 5 list (base64) of the phone's own newest ops (at most 8) that this terminal holds, for a RESTORED phone to take its chain back. The list names this terminal's device id as its sender (unsigned: it only lets the phone tell two lists apart). The phone adopts its chain only from two lists naming the same newest op that come from two DIFFERENT terminals, so run this on two terminals. Scanning one list twice is refused. <phone-device-id> is the 16 hex characters on the phone's home screen. With --ble (a build with the `bluetooth` feature) the list is also handed to the phone app over Bluetooth (`sent:`)."),
        "sync-phone-ops" => ("sync-phone-ops [--wait <seconds>] [--ble] [--format plain|json]", "Take in the ops the phone copied (kind 5, base64) from stdin, chain them into the log and print what applied, why the others were skipped (`skipped` in JSON: `chain-gap` with the first refused seq when the list starts past what the ring knows, `already-known`, `invalid`) and the ceremony of every pending Red op. A malformed text changes nothing. With --ble (a build with the `bluetooth` feature) the ops frame is received from the phone app over Bluetooth instead of stdin (`bluetooth: pick SSHub-XXXX in the phone app's device list`, up to --wait seconds)."),
        "sync-share-request" => ("sync-share-request <host> [--wait <seconds>] [--ble] [--format plain|json]", "Ask the phone for its share of <host>'s credential: prints the host name with its host id (32 hex digits in groups of four, as the phone draws it) and the challenge's four binding words (`host:` and `words:` lines; in JSON the `host_id`, `host_uid` and `words` fields, also on the challenge line), which you compare with the phone before approving; then prints the kind 1 challenge (base64), then reads the phone's kind 2 reply from stdin (up to --wait seconds; default and maximum 120, the challenge's life), opens it, takes in its courier ops and opens the credential with this device's share. The credential is open in this process only: run it to prove the path and to take in the phone's ops; `host connect` does its own unlock. With --ble (a build with the `bluetooth` feature) the challenge is also sent to the phone app over Bluetooth (`bluetooth: pick SSHub-XXXX in the phone app's device list`) and the reply is taken from Bluetooth or stdin, whichever comes first."),
        "sync-trust-renew" => ("sync-trust-renew [<id>] [--days N] [--ble] [--format plain|json]", "Renew a trust session without a new full approval: no id lists the sessions in force (id, delegate, end, renewals used of 2); with the first 6 or more hex digits of an id it prints the kind 11 request (base64) for the phone, which shows what the other terminals saw of the delegate and asks you to confirm. At most the first term (--days, default 30), at most 2 renewals, at most 180 days in all. Run it on a computer that is not the delegate. The phone's answer goes in with sync-phone-ops."),
        "sync-cert-request" => ("sync-cert-request (--group NAME | --host NAME...) [--days N] [--budget N] [--ble] [--format plain|json]", "Print the kind 9 trust-session request (base64) for the phone to approve: a group or listed hosts (at most 64, never everything), a term in days (default 30) and a budget of new hosts a day (default 10). The phone's answer goes in with sync-phone-ops. The kind 10 extension is TUI-only. With --ble (a build with the `bluetooth` feature) the request is also handed to the phone app over Bluetooth: it prints `bluetooth: pick SSHub-XXXX in the phone app's device list` and waits up to --wait seconds until the phone has the frame (`sent:`)."),
        _ => return,
    };
    println!(
        "sshub {cmd} - {}",
        text.split(". ").next().unwrap_or("").trim_end_matches('.')
    );
    println!();
    println!("USAGE:");
    println!("    sshub [--profile NAME] {synopsis}");
    println!();
    println!("{text}");
}

#[cfg(test)]
mod tests;
