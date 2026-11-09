//! Deliver a stored password/passphrase to ssh via `SSH_ASKPASS` instead of
//! typing it into the PTY.
//!
//! With `SSH_ASKPASS_REQUIRE=force` (OpenSSH ≥ 8.4) ssh calls the askpass
//! helper for both passphrase and password prompts even on a tty, so the
//! "Enter passphrase for key …" / "…'s password:" line never appears on the
//! screen. The helper is this same binary re-executed in askpass mode (see
//! [`maybe_run_askpass`]); the destination's secret is handed over through a
//! private `0600` file that is removed when the session ends. Jump hops'
//! secrets are never written: the file holds references into the credential
//! store, which the helper reads when that hop's prompt arrives. The plaintext
//! of a synced credential is the one thing no file may hold: the staging
//! process keeps it and serves it over a private socket (see
//! [`AskpassSecret::staged`]).

use std::io::Write;

use crate::credentials::{SecretRef, StoreLocator};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::auth_prompt::PromptClass;
use super::PendingSecret;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Path to hand ssh as its `SSH_ASKPASS` helper: this binary, re-executed.
///
/// Not simply `current_exe()`, which reads `/proc/self/exe` and reports
/// `<path> (deleted)` once the file has been replaced. An upgrade never writes
/// into the running binary: `npm install -g`, `cargo install`, a package manager
/// and `just install` all unlink the old inode and create a new one. ssh then
/// fails to exec that literal path, no stored secret is delivered, and what the
/// user sees is `Permission denied (publickey)`, which points at the server
/// rather than at the upgrade.
///
/// So when the path is gone, strip the marker and take the same path again: after
/// an in-place upgrade a new binary is sitting right there.
pub fn helper_exe() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    if exe.exists() {
        return Ok(exe);
    }
    if let Some(replaced) = strip_deleted_marker(&exe) {
        return Ok(replaced);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!(
            "the sshub binary is no longer at {}, which happens when it is \
             upgraded while running; restart SSHub to restore password and \
             passphrase auth",
            exe.display()
        ),
    ))
}

/// `/proc/self/exe` for a replaced binary reads as `<path> (deleted)`. Returns
/// that path when it exists again, which is the normal outcome of an upgrade.
fn strip_deleted_marker(exe: &Path) -> Option<PathBuf> {
    let text = exe.to_str()?;
    let base = PathBuf::from(text.strip_suffix(" (deleted)")?);
    base.exists().then_some(base)
}

/// A secret staged for `SSH_ASKPASS`: a short-lived owner-only file, except
/// that the plaintext of a synced credential never touches a disk. It stays in
/// this process and is served to the helper over a private socket.
pub struct AskpassSecret {
    path: PathBuf,
    server: Option<SecretServer>,
}

/// Where staged files and sockets go: `$XDG_RUNTIME_DIR` (tmpfs on Linux) or
/// the system temp dir.
fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

impl AskpassSecret {
    /// Write `secret` to a fresh `0600` file under `$XDG_RUNTIME_DIR` (or the
    /// system temp dir).
    pub fn new(secret: &PendingSecret) -> std::io::Result<Self> {
        Self::staged(Some(secret), &[])
    }

    /// Stage the destination's `secret` (if it has one) and references to the
    /// jump hops' secrets. The first line is the destination's; a `C<TAB>p|k`
    /// line says whether it is a password or a key passphrase; an `S` line
    /// names the credential store; every other line is one hop reference
    /// (`H<TAB>class<TAB>user<TAB>host<TAB>key<TAB>via<TAB>ref…`). A local
    /// hop secret is never written: the helper reads it from the store when
    /// that hop's prompt arrives. A synced credential open in this process is
    /// never written either, hop or destination: the file holds a handle
    /// (`V<TAB>d` for the destination, `s:<id>` in a hop line) and the helper
    /// asks this process for the value over a socket in a private directory,
    /// with a one-time token from its environment. The helper answers a hop's
    /// prompt from the line that names that hop, and the destination's prompt
    /// with the first line, only for a prompt of its class: never the other
    /// way round.
    pub fn staged(
        secret: Option<&PendingSecret>,
        hops: &[super::hop_auth::HopSecret],
    ) -> std::io::Result<Self> {
        Self::staged_in(&runtime_dir(), secret, hops)
    }

    fn staged_in(
        dir: &Path,
        secret: Option<&PendingSecret>,
        hops: &[super::hop_auth::HopSecret],
    ) -> std::io::Result<Self> {
        use zeroize::Zeroizing;
        let value = secret.map_or("", PendingSecret::value);
        if value.contains(['\n', '\r']) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a secret with a line break cannot be staged",
            ));
        }
        // Values this process keeps for the helper to ask for, by handle.
        let mut held: Vec<(String, Zeroizing<String>)> = Vec::new();
        let dest_synced = !value.is_empty() && crate::credentials::is_open_synced_value(value);
        if dest_synced {
            held.push(("d".into(), Zeroizing::new(value.to_string())));
        }
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("sshub-askpass-{}-{}", std::process::id(), n));

        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&path)?;
        // The file now exists and is ours: any failure below removes it.
        let mut guard = Self { path, server: None };
        // ssh strips the trailing newline from the askpass output.
        writeln!(f, "{}", if dest_synced { "" } else { value })?;
        match secret {
            Some(PendingSecret::Password(_)) => writeln!(f, "C\tp")?,
            Some(PendingSecret::Passphrase(_)) => writeln!(f, "C\tk")?,
            None => {}
        }
        if dest_synced {
            writeln!(f, "V\td")?;
        }
        // One `S` line names the credential store the hop references live in.
        // The helper opens exactly that store (same profile prefix, same
        // credentials file), so no profile env has to reach it.
        let store = hops.iter().find_map(|h| h.store.as_ref());
        let store_line = store.and_then(store_line);
        if let Some(line) = &store_line {
            writeln!(f, "{line}")?;
        }
        for hop in hops {
            // A hop whose secrets are all held here needs no store. Any
            // other, without the store or with another one than the `S` line
            // names, could not be read by the helper: leave it out.
            let only_staged = hop.refs.iter().all(|r| matches!(r, SecretRef::Staged(_)));
            if !only_staged && (store_line.is_none() || hop.store.as_ref() != store) {
                continue;
            }
            let class = match hop.class {
                PromptClass::Passphrase => "k",
                _ => "p",
            };
            let (user, key) = (
                hop.user.as_deref().unwrap_or(""),
                hop.key_path.as_deref().unwrap_or(""),
            );
            let mut mine: Vec<(String, Zeroizing<String>)> = Vec::new();
            let refs: Vec<String> = hop
                .refs
                .iter()
                .map(|r| match r {
                    SecretRef::Key(k) => format!("k:{k}"),
                    SecretRef::Lookup(s) => format!("l:{s}"),
                    SecretRef::Handle(id) => format!("s:{id}"),
                    // An opened synced credential: kept here, never written.
                    SecretRef::Staged(v) => {
                        let id = format!("s{}", held.len() + mine.len());
                        mine.push((id.clone(), Zeroizing::new(v.value().to_string())));
                        format!("s:{id}")
                    }
                })
                .collect();
            // The format cannot hold a tab or a line break. Such a hop is left
            // out (its prompt then goes unanswered), never mis-parsed into a
            // shifted field with another reference. A held value must also be
            // one line, since the helper hands it to ssh as one.
            if [user, hop.host.as_str(), hop.via.as_str(), key]
                .into_iter()
                .chain(refs.iter().map(String::as_str))
                .any(|field| field.contains(['\t', '\n', '\r']))
                || mine.iter().any(|(_, v)| v.contains(['\n', '\r']))
                || refs.is_empty()
            {
                continue;
            }
            held.extend(mine);
            writeln!(
                f,
                "H\t{class}\t{user}\t{}\t{key}\t{}\t{}",
                hop.host,
                hop.via,
                refs.join("\t")
            )?;
        }
        if !held.is_empty() {
            // No server, no staging: a synced secret never falls back to a file.
            guard.server = Some(SecretServer::start(dir, held)?);
        }
        Ok(guard)
    }

    /// Where the secrets are staged.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether this staging holds a synced credential, which lives only as
    /// long as this process does.
    pub fn holds_synced(&self) -> bool {
        self.server.is_some()
    }

    /// Environment for the ssh child so it consults this helper.
    pub fn env(&self, exe: &Path) -> Vec<(String, String)> {
        let mut env = vec![
            ("SSH_ASKPASS".into(), exe.to_string_lossy().into_owned()),
            ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
            (
                ASKPASS_FILE_ENV.into(),
                self.path.to_string_lossy().into_owned(),
            ),
            (OWNER_ENV.into(), std::process::id().to_string()),
        ];
        if let Some(server) = &self.server {
            env.push((
                VALUES_SOCK_ENV.into(),
                server.socket().to_string_lossy().into_owned(),
            ));
            env.push((VALUES_TOKEN_ENV.into(), server.token().to_string()));
        }
        env
    }

    /// Keep the file after this guard is gone, for an ssh that outlives the
    /// process that staged it: a detached tunnel (#144). Set [`READ_ONCE_ENV`]
    /// on that ssh so the helper removes the file once it has been read, and
    /// [`remove_outlived`] it when the tunnel stops without ever asking.
    /// Callers must refuse a guard that [`holds_synced`](Self::holds_synced):
    /// its values die with this process.
    pub fn outlive(mut self) -> PathBuf {
        std::mem::take(&mut self.path)
    }
}

impl Drop for AskpassSecret {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Names the socket and token of the process holding synced secrets for the
/// helper (this process, for the lifetime of the staging guard).
const VALUES_SOCK_ENV: &str = "SSHUB_ASKPASS_VALUES";
const VALUES_TOKEN_ENV: &str = "SSHUB_ASKPASS_VALUES_TOKEN";

/// Serves held plaintexts, by handle, to the helper: a socket in a `0700`
/// directory, a random token, and nothing on disk but the socket inode.
struct SecretServer {
    directory: tempfile::TempDir,
    token: String,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SecretServer {
    fn start(
        root: &Path,
        values: Vec<(String, zeroize::Zeroizing<String>)>,
    ) -> std::io::Result<Self> {
        use std::io::Read;
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;
        let directory = tempfile::Builder::new()
            .prefix("sshub-askpass-")
            .tempdir_in(root)?;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
        let socket = directory.path().join("s");
        let listener = UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let mut random = [0u8; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        let token: String = random.iter().map(|b| format!("{b:02x}")).collect();

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (stop_flag, expect) = (stop.clone(), token.clone());
        let thread = std::thread::spawn(move || {
            let values: std::collections::HashMap<_, _> = values.into_iter().collect();
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((conn, _)) => serve_held(conn, &expect, &values),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            directory,
            token,
            stop,
            thread: Some(thread),
        })
    }

    fn socket(&self) -> PathBuf {
        self.directory.path().join("s")
    }

    fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for SecretServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // `directory` removes the socket and itself.
    }
}

/// One request: `<token>\t<handle>\n`; the answer is the value and `\n`.
fn serve_held(
    mut conn: std::os::unix::net::UnixStream,
    token: &str,
    values: &std::collections::HashMap<String, zeroize::Zeroizing<String>>,
) {
    use std::io::Read;
    let timeout = Some(std::time::Duration::from_secs(2));
    let _ = conn.set_nonblocking(false);
    let _ = conn.set_read_timeout(timeout);
    let _ = conn.set_write_timeout(timeout);
    let mut request = Vec::new();
    let mut byte = [0u8; 1];
    while request.len() < 256 {
        match conn.read(&mut byte) {
            Ok(1) if byte[0] != b'\n' => request.push(byte[0]),
            _ => break,
        }
    }
    let request = String::from_utf8_lossy(&request);
    let Some((got, id)) = request.split_once('\t') else {
        return;
    };
    if got != token {
        return;
    }
    if let Some(value) = values.get(id) {
        let _ = writeln!(conn, "{}", value.as_str());
    }
}

/// Ask the process that staged the secrets for the value behind `id`.
fn fetch_held(id: &str) -> Option<String> {
    let sock = std::env::var_os(VALUES_SOCK_ENV)?;
    let token = std::env::var(VALUES_TOKEN_ENV).ok()?;
    fetch_held_from(Path::new(&sock), &token, id)
}

fn fetch_held_from(sock: &Path, token: &str, id: &str) -> Option<String> {
    use std::io::Read;
    let timeout = Some(std::time::Duration::from_secs(2));
    let mut conn = std::os::unix::net::UnixStream::connect(sock).ok()?;
    conn.set_read_timeout(timeout).ok()?;
    conn.set_write_timeout(timeout).ok()?;
    writeln!(conn, "{token}\t{id}").ok()?;
    let mut answer = String::new();
    conn.read_to_string(&mut answer).ok()?;
    let value = answer.strip_suffix('\n').unwrap_or(&answer);
    (!value.is_empty() && !value.contains(['\n', '\r'])).then(|| value.to_string())
}

/// Remove a file handed out by [`AskpassSecret::outlive`]. Refuses any path
/// that is not a staged secret, since the caller read it back from disk.
pub fn remove_outlived(path: &Path) {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("sshub-askpass-"))
    {
        let _ = std::fs::remove_file(path);
    }
}

/// Tells the helper to delete the staged file after its first read.
pub const READ_ONCE_ENV: &str = "SSHUB_ASKPASS_READ_ONCE";

const ASKPASS_FILE_ENV: &str = "SSHUB_ASKPASS_FILE";
/// The sshub process that staged the secret: where the ancestry walk stops.
const OWNER_ENV: &str = "SSHUB_ASKPASS_OWNER";

/// Whether the ssh that ran this helper is a ProxyJump/ProxyCommand hop: a
/// process named `ssh` sits between it and the sshub that staged the secret.
/// The hop inherits the destination's environment, so without this check it
/// would be handed the destination's password (#141). Only wrappers such as
/// `script` or `sh -c` sit above the destination ssh itself.
fn asked_by_jump_hop(owner: Option<u32>) -> bool {
    // Where the process tree cannot be read, assume a hop: the destination's
    // secret must not go to an ssh we could not identify. (Platforms with no
    // reader at all keep the old behaviour, or nothing could ever be answered.)
    const UNREADABLE: bool = cfg!(any(target_os = "linux", target_os = "macos"));
    let Some((mut pid, _)) = parent_and_name(std::os::unix::process::parent_id()) else {
        return UNREADABLE;
    };
    for _ in 0..16 {
        if pid <= 1 || Some(pid) == owner {
            return false;
        }
        let Some((parent, name)) = parent_and_name(pid) else {
            return UNREADABLE;
        };
        if name == "ssh" {
            return true;
        }
        pid = parent;
    }
    UNREADABLE
}

/// Parent pid and command name of `pid`.
#[cfg(target_os = "linux")]
fn parent_and_name(pid: u32) -> Option<(u32, String)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `pid (comm) state ppid …`; comm may itself contain `) `.
    let (head, rest) = stat.rsplit_once(") ")?;
    let name = head.split_once(" (")?.1;
    let parent = rest.split_whitespace().nth(1)?.parse().ok()?;
    Some((parent, name.to_owned()))
}

/// Parent pid and command name of `pid`.
#[cfg(target_os = "macos")]
fn parent_and_name(pid: u32) -> Option<(u32, String)> {
    // SAFETY: an all-zero proc_bsdinfo is a valid value of this plain C struct.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is a writable buffer of exactly `size` bytes.
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    let name: Vec<u8> = info
        .pbi_comm
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    Some((info.pbi_ppid, String::from_utf8_lossy(&name).into_owned()))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn parent_and_name(_pid: u32) -> Option<(u32, String)> {
    None
}

/// The prompt ssh passed as the helper's first argument.
fn prompt_arg() -> Option<String> {
    // A test binary's own first argument is a libtest flag.
    #[cfg(test)]
    if let Ok(prompt) = std::env::var("SSHUB_TEST_PROMPT") {
        return Some(prompt);
    }
    std::env::args().nth(1)
}

fn print_answer(value: &str) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

/// What a staged file holds: the destination's secret (first line), its class
/// (`C<TAB>p|k` line), a handle for a destination secret held by the staging
/// process (`V<TAB>id`) and the hop secrets after it.
struct Staged<'a> {
    destination: &'a str,
    /// The class of the destination's secret, when the stager knew it.
    class: Option<PromptClass>,
    /// Set when the destination's secret is not in the file but held by the
    /// staging process.
    destination_handle: Option<&'a str>,
    hops: Vec<super::hop_auth::HopSecret>,
}

/// The `S<TAB>keyring<TAB>prefix` / `S<TAB>file<TAB>prefix<TAB>path` line for
/// `store`; `None` when a field cannot be held by the format.
fn store_line(store: &StoreLocator) -> Option<String> {
    let (kind, prefix, path) = match store {
        StoreLocator::Keyring { prefix } => ("keyring", prefix.as_str(), None),
        StoreLocator::File { path, prefix } => ("file", prefix.as_str(), Some(path.to_str()?)),
    };
    let bad = |s: &str| s.contains(['\t', '\n', '\r']);
    if bad(prefix) || path.is_some_and(bad) {
        return None;
    }
    Some(match path {
        Some(path) => format!("S\t{kind}\t{prefix}\t{path}"),
        None => format!("S\t{kind}\t{prefix}"),
    })
}

fn parse_store(line: &str) -> Option<StoreLocator> {
    let mut f = line.splitn(4, '\t');
    (f.next()? == "S").then_some(())?;
    let kind = f.next()?;
    let prefix = f.next()?.to_string();
    match kind {
        "keyring" => Some(StoreLocator::Keyring { prefix }),
        "file" => Some(StoreLocator::File {
            path: PathBuf::from(f.next()?),
            prefix,
        }),
        _ => None,
    }
}

/// One `H<TAB>class<TAB>user<TAB>host<TAB>key<TAB>via<TAB>ref[<TAB>ref…]`
/// line, each ref `k:<store key>` or `l:<secret lookup spec>`. The hop's
/// `store` is filled in by [`parse_staged`].
fn parse_hop(line: &str) -> Option<super::hop_auth::HopSecret> {
    let mut f = line.splitn(7, '\t');
    (f.next()? == "H").then_some(())?;
    let class = match f.next()? {
        "k" => PromptClass::Passphrase,
        _ => PromptClass::Password,
    };
    let opt = |s: &str| (!s.is_empty()).then(|| s.to_string());
    let (user, host, key_path, via) = (
        opt(f.next()?),
        f.next()?.to_string(),
        opt(f.next()?),
        f.next()?.to_string(),
    );
    let refs: Vec<SecretRef> = f
        .next()?
        .split('\t')
        .filter_map(|r| match r.split_at_checked(2)? {
            ("k:", key) => Some(SecretRef::Key(key.to_string())),
            ("l:", spec) => Some(SecretRef::Lookup(spec.to_string())),
            ("s:", id) => Some(SecretRef::Handle(id.to_string())),
            _ => None,
        })
        .collect();
    Some(super::hop_auth::HopSecret {
        class,
        user,
        host,
        key_path,
        via,
        refs,
        store: None,
    })
}

fn parse_staged(text: &str) -> Staged<'_> {
    let mut lines = text.lines();
    let destination = lines.next().unwrap_or("");
    let mut class = None;
    let mut destination_handle = None;
    let mut store = None;
    let mut hops = Vec::new();
    for line in lines {
        match line {
            "C\tp" => class = Some(PromptClass::Password),
            "C\tk" => class = Some(PromptClass::Passphrase),
            _ if line.starts_with("V\t") => destination_handle = line.strip_prefix("V\t"),
            _ if line.starts_with("S\t") => store = parse_store(line),
            _ => hops.extend(parse_hop(line)),
        }
    }
    for hop in &mut hops {
        hop.store = store.clone();
    }
    Staged {
        destination,
        class,
        destination_handle,
        hops,
    }
}

/// What `hop` references in its credential store, read now. Fails closed:
/// no store, no reference that resolves, or a value ssh could not take as one
/// line, and the hop's prompt is left to the user.
fn hop_value(hop: &super::hop_auth::HopSecret) -> Option<String> {
    // A staged value needs no store; a store-less hop reads only those.
    let store = hop.store.as_ref().map_or_else(
        || {
            Box::new(crate::credentials::NoopPasswordStore)
                as Box<dyn crate::credentials::PasswordStore>
        },
        StoreLocator::open,
    );
    hop.refs
        .iter()
        .find_map(|r| match r {
            // Held by the staging process: asked for now, never read from a file.
            SecretRef::Handle(id) => fetch_held(id),
            _ => r.read(store.as_ref()),
        })
        .filter(|v| !v.is_empty() && !v.contains(['\n', '\r']))
}

/// Which staged secret the helper just handed to ssh.
enum Answered {
    Destination,
    /// The n-th hop line, counted as [`parse_staged`] counts them.
    Hop(usize),
}

/// The staged text without the answered secret; `None` when no secret is left.
fn without_answered(text: &str, answered: &Answered) -> Option<String> {
    let mut out = String::new();
    let mut hop_no = 0;
    let mut secret_left = false;
    for (i, line) in text.lines().enumerate() {
        if i == 0 {
            if matches!(answered, Answered::Destination) {
                out.push('\n');
                continue;
            }
            secret_left |= !line.is_empty();
        } else if parse_hop(line).is_some() {
            let n = hop_no;
            hop_no += 1;
            if matches!(answered, Answered::Hop(h) if *h == n) {
                continue;
            }
            secret_left = true;
        }
        out.push_str(line);
        out.push('\n');
    }
    secret_left.then_some(out)
}

/// A detached tunnel's file lives as long as ssh may still ask, but no longer
/// than the secrets in it: each answered secret is wiped, and the file goes
/// with the last one. Fails closed: if it cannot be rewritten, it is removed.
fn consume(file: &Path, answered: &Answered) {
    let rest = std::fs::read_to_string(file)
        .ok()
        .and_then(|text| without_answered(&text, answered));
    if !rest.is_some_and(|rest| std::fs::write(file, rest).is_ok()) {
        let _ = std::fs::remove_file(file);
    }
}

/// If this process was launched by ssh as its `SSH_ASKPASS` helper, print the
/// staged secret and return `true` (the caller should exit immediately). Set
/// only on the ssh child's environment, so the main TUI process never sees it.
pub fn maybe_run_askpass() -> bool {
    let Some(file) = std::env::var_os(ASKPASS_FILE_ENV) else {
        if std::env::var_os(super::askpass_channel::MODE_ENV).is_none()
            && std::env::var_os(super::askpass_channel::SOCKET_ENV).is_none()
            && std::env::var_os(super::askpass_channel::TOKEN_ENV).is_none()
        {
            return false;
        }
        let answer = (|| {
            let path = std::env::var_os(super::askpass_channel::SOCKET_ENV)?;
            let token = std::env::var(super::askpass_channel::TOKEN_ENV).ok()?;
            let prompt = std::env::args().nth(1)?;
            super::askpass_channel::request_answer(
                Path::new(&path),
                &token,
                &prompt,
                super::askpass_channel::asking_ssh_pid(),
                super::askpass_channel::TIMEOUT,
            )
            .ok()
        })();
        let Some(answer) = answer else {
            std::process::exit(1)
        };
        let mut out = std::io::stdout().lock();
        if writeln!(out, "{answer}")
            .and_then(|()| out.flush())
            .is_err()
        {
            std::process::exit(1);
        }
        return true;
    };
    let owner = std::env::var(OWNER_ENV)
        .ok()
        .and_then(|pid| pid.parse().ok());
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let staged = parse_staged(&text);
    let read_once = std::env::var_os(READ_ONCE_ENV).is_some();
    let prompt = prompt_arg();
    if asked_by_jump_hop(owner) {
        // A hop: its own secret if the prompt names a hop we hold one for,
        // never the destination's.
        let found = prompt
            .as_deref()
            .and_then(|prompt| {
                // The helper's parent is the ssh that is asking.
                let pid = Some(std::os::unix::process::parent_id());
                super::hop_auth::find_secret(&staged.hops, prompt, pid)
            })
            .and_then(|hop| {
                let n = staged.hops.iter().position(|h| std::ptr::eq(h, hop))?;
                // The store is read only now, when this hop asks.
                Some((hop_value(hop)?, n))
            });
        let Some((value, n)) = found else {
            std::process::exit(1)
        };
        print_answer(&value);
        if read_once {
            consume(Path::new(&file), &Answered::Hop(n));
        }
        return true;
    }
    // The destination's secret answers a prompt of its own class and nothing
    // else: not a 2FA code, a host-key question, or a password prompt when
    // what was staged is a key passphrase.
    let class = prompt.as_deref().map(PromptClass::classify);
    let answers = matches!(class, Some(PromptClass::Password | PromptClass::Passphrase))
        && staged.class.is_none_or(|staged| Some(staged) == class);
    let destination = if staged.destination.is_empty() {
        staged.destination_handle.and_then(fetch_held)
    } else {
        Some(staged.destination.to_string())
    };
    let Some(destination) = destination.filter(|_| answers) else {
        // Hop-only staging, or a prompt that is the user's: nothing to say.
        std::process::exit(1);
    };
    print_answer(&destination);
    if read_once {
        consume(Path::new(&file), &Answered::Destination);
    }
    true
}

#[cfg(test)]
mod tests {
    /// `output()` that retries ETXTBSY: a stand-in written by this test can still be open for writing in a child another test
    /// thread forked a moment ago, until that child execs (close-on-exec). The file is never touched again, so a short retry ends.
    trait OutputRetrying {
        fn output_retrying(&mut self) -> std::process::Output;
    }

    impl OutputRetrying for std::process::Command {
        fn output_retrying(&mut self) -> std::process::Output {
            for _ in 0..200 {
                match self.output() {
                    Err(e) if e.raw_os_error() == Some(26) => {
                        std::thread::sleep(std::time::Duration::from_millis(25))
                    }
                    other => return other.unwrap(),
                }
            }
            self.output().unwrap()
        }
    }

    use super::*;

    #[test]
    fn file_helper_entry() {
        if std::env::var_os("SSHUB_TEST_FILE_HELPER").is_some() {
            assert!(maybe_run_askpass());
            std::process::exit(0);
        }
    }

    #[test]
    fn read_once_secret_is_gone_after_the_first_answer() {
        // A detached tunnel's secret outlives sshub (#144); the helper removes
        // it as soon as ssh has read it.
        let dir = tempfile::tempdir().unwrap();
        let guard = AskpassSecret {
            path: dir.path().join("sshub-askpass-test"),
            server: None,
        };
        std::fs::write(&guard.path, "s3cr3t\n").unwrap();
        let path = guard.outlive();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "session::askpass::tests::file_helper_entry",
                "--nocapture",
            ])
            .env("SSHUB_TEST_FILE_HELPER", "1")
            .env(ASKPASS_FILE_ENV, &path)
            .env(READ_ONCE_ENV, "1")
            .env("SSHUB_TEST_PROMPT", "ops@h's password: ")
            // This test plays the ssh; stop the hop walk at its parent.
            .env(OWNER_ENV, std::os::unix::process::parent_id().to_string())
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("s3cr3t"));
        assert!(!path.exists());
    }

    #[test]
    fn staged_secret_is_never_handed_to_a_jump_hop() {
        // ProxyJump (#141): the jump ssh runs under the destination ssh and
        // inherits its askpass environment. Stand-ins named `ssh` (a copy of
        // bash) rebuild both process trees around a real helper process.
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join("ssh");
        std::fs::copy("/bin/bash", &ssh).unwrap();
        // Staged in this test's own dir: another test repoints XDG_RUNTIME_DIR.
        let guard = AskpassSecret {
            path: dir.path().join("secret"),
            server: None,
        };
        std::fs::write(&guard.path, "s3cr3t\n").unwrap();
        let helper = format!(
            "'{}' --exact session::askpass::tests::file_helper_entry --nocapture; echo helper-exit=$?",
            std::env::current_exe().unwrap().display()
        );
        let run = |script: &str| {
            let out = std::process::Command::new(&ssh)
                .args(["-c", script])
                .env("SSHUB_TEST_FILE_HELPER", "1")
                .env("SSHUB_TEST_PROMPT", "ops@10.0.4.91's password: ")
                .envs(guard.env(Path::new("unused")))
                .output_retrying();
            String::from_utf8_lossy(&out.stdout).into_owned()
        };

        // The destination: the helper's `ssh` was started by this process.
        let destination = run(&helper);
        assert!(destination.contains("s3cr3t"), "{destination}");
        assert!(destination.contains("helper-exit=0"), "{destination}");

        // A hop: the helper's `ssh` runs under another `ssh`.
        let hop = run(&format!(
            "'{}' -c \"{}\"; :",
            ssh.display(),
            helper.replace('$', "\\$")
        ));
        assert!(!hop.contains("s3cr3t"), "{hop}");
        assert!(hop.contains("helper-exit=1"), "{hop}");
    }

    #[test]
    fn a_hop_gets_its_own_staged_secret_and_the_destination_its_own() {
        use super::super::auth_prompt::PromptClass;
        use super::super::hop_auth::HopSecret;
        // Same process-tree stand-ins as above: `ssh` is a copy of bash.
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join("ssh");
        std::fs::copy("/bin/bash", &ssh).unwrap();
        // The hops' secrets sit in a file credential store (as the helper will
        // reopen it); only references are staged.
        let creds = dir.path().join("credentials.json");
        let store = StoreLocator::File {
            path: creds.clone(),
            prefix: "profile:7:".into(),
        };
        let backing = store.open();
        backing.set("host:1", "lb-1-pw").unwrap();
        backing.set("host:2", "lb-2-pw").unwrap();
        let hop = |user: &str, host: &str, key: &str| HopSecret {
            user: Some(user.into()),
            host: host.into(),
            via: host.into(),
            key_path: None,
            class: PromptClass::Password,
            refs: vec![SecretRef::Key(key.into())],
            store: Some(store.clone()),
        };
        let path = dir.path().join("secret");
        let guard = AskpassSecret::staged(
            Some(&PendingSecret::Password("target-pw".into())),
            &[
                hop("ops", "10.0.4.11", "host:1"),
                hop("ops", "10.0.4.12", "host:2"),
                // Nothing is stored under this reference.
                hop("ops", "10.0.4.13", "host:404"),
            ],
        )
        .unwrap();
        let staged_text = std::fs::read_to_string(&guard.path).unwrap();
        assert!(
            !staged_text.contains("lb-1-pw") && !staged_text.contains("lb-2-pw"),
            "a hop's secret was written to the staged file: {staged_text}"
        );
        std::fs::copy(guard.path.clone(), &path).unwrap();
        let guard = AskpassSecret { path, server: None };
        let helper = format!(
            "'{}' --exact session::askpass::tests::file_helper_entry --nocapture; echo helper-exit=$?",
            std::env::current_exe().unwrap().display()
        );
        let run = |script: &str, prompt: &str| {
            let out = std::process::Command::new(&ssh)
                .args(["-c", script])
                .env("SSHUB_TEST_FILE_HELPER", "1")
                .env("SSHUB_TEST_PROMPT", prompt)
                .envs(guard.env(Path::new("unused")))
                .output_retrying();
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        // The hop's own ssh, whose last argument is the host it connects to.
        let as_hop_via = |prompt: &str, via: &str| {
            run(
                &format!(
                    "'{}' -c \"{}\" _ {via}; :",
                    ssh.display(),
                    helper.replace('$', "\\$")
                ),
                prompt,
            )
        };
        let as_hop =
            |prompt: &str| as_hop_via(prompt, prompt.split(['@', '\'']).nth(1).unwrap_or("none"));
        // The destination: the first line, whatever the prompt says.
        let dest = run(&helper, "ops@10.0.4.91's password: ");
        assert!(
            dest.contains("target-pw") && dest.contains("helper-exit=0"),
            "{dest}"
        );
        // Each hop: its own line, found by the prompt.
        let one = as_hop("ops@10.0.4.11's password: ");
        assert!(
            one.contains("lb-1-pw") && !one.contains("lb-2-pw") && !one.contains("target-pw"),
            "{one}"
        );
        let two = as_hop("ops@10.0.4.12's password: ");
        assert!(
            two.contains("lb-2-pw") && !two.contains("lb-1-pw") && !two.contains("target-pw"),
            "{two}"
        );
        // A prompt naming lb-2 from the ssh that connects to lb-1 (a hostile
        // hop's text) gets nothing: not lb-2's secret, not lb-1's, not the target's.
        let lie = as_hop_via("ops@10.0.4.12's password: ", "10.0.4.11");
        assert!(
            !lie.contains("-pw") && lie.contains("helper-exit=1"),
            "{lie}"
        );
        // A hop that is not staged gets nothing, and above all not the target's.
        for prompt in [
            "ops@10.9.9.9's password: ",
            // Staged, but its reference resolves to nothing: fail closed.
            "ops@10.0.4.13's password: ",
            "root@10.0.4.11's password: ",
            "Password: ",
        ] {
            let other = as_hop(prompt);
            assert!(
                !other.contains("-pw") && other.contains("helper-exit=1"),
                "{prompt}: {other}"
            );
        }
    }

    #[test]
    fn only_hop_secrets_staged_leaves_the_destination_to_the_user() {
        use super::super::auth_prompt::PromptClass;
        let dir = tempfile::tempdir().unwrap();
        let guard = AskpassSecret::staged(
            None,
            &[super::super::hop_auth::HopSecret {
                user: None,
                host: "h".into(),
                via: "h".into(),
                key_path: None,
                class: PromptClass::Password,
                refs: vec![SecretRef::Key("host:3".into())],
                store: Some(StoreLocator::Keyring {
                    prefix: "profile:2:".into(),
                }),
            }],
        )
        .unwrap();
        let text = std::fs::read_to_string(&guard.path).unwrap();
        let staged = parse_staged(&text);
        assert_eq!(staged.destination, "");
        assert_eq!(staged.class, None);
        assert_eq!(staged.hops.len(), 1);
        assert_eq!(staged.hops[0].refs, [SecretRef::Key("host:3".into())]);
        assert_eq!(
            staged.hops[0].store,
            Some(StoreLocator::Keyring {
                prefix: "profile:2:".into()
            })
        );
        drop(dir);
    }

    #[test]
    fn stages_secret_in_owner_only_file_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_RUNTIME_DIR", dir.path());

        let path;
        {
            let guard = AskpassSecret::new(&PendingSecret::Password("s3cr3t".into())).unwrap();
            let env = guard.env(std::path::Path::new("/usr/bin/sshub"));
            // File path is exposed via the env we hand to ssh.
            let file = env
                .iter()
                .find(|(k, _)| k == ASKPASS_FILE_ENV)
                .map(|(_, v)| v.clone())
                .unwrap();
            path = PathBuf::from(file);
            assert!(env
                .iter()
                .any(|(k, v)| k == "SSH_ASKPASS_REQUIRE" && v == "force"));
            assert!(path.exists());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "s3cr3t\nC\tp\n");

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&path).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o600);
            }
        }
        // Dropped guard removes the file.
        assert!(!path.exists());

        std::env::remove_var("XDG_RUNTIME_DIR");
    }

    #[test]
    fn the_destinations_secret_answers_only_a_prompt_of_its_own_class() {
        // A key passphrase staged for the destination must not become the
        // account password on the password fallback, and a 2FA code or a
        // host-key question gets nothing. `ssh` here is a copy of bash.
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join("ssh");
        std::fs::copy("/bin/bash", &ssh).unwrap();
        let guard =
            AskpassSecret::staged(Some(&PendingSecret::Passphrase("key-phrase".into())), &[])
                .unwrap();
        let helper = format!(
            "'{}' --exact session::askpass::tests::file_helper_entry --nocapture; echo helper-exit=$?",
            std::env::current_exe().unwrap().display()
        );
        let run = |prompt: &str| {
            let out = std::process::Command::new(&ssh)
                .args(["-c", &helper])
                .env("SSHUB_TEST_FILE_HELPER", "1")
                .env("SSHUB_TEST_PROMPT", prompt)
                .envs(guard.env(Path::new("unused")))
                .output_retrying();
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        let ok = run("Enter passphrase for key '/home/u/.ssh/id': ");
        assert!(
            ok.contains("key-phrase") && ok.contains("helper-exit=0"),
            "{ok}"
        );
        for prompt in [
            "ops@10.0.4.91's password: ",
            "Verification code: ",
            "Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
            // A server-sent keyboard-interactive prompt is prefixed `(user@host) `.
            "(ops@10.0.4.91) Enter passphrase for key '/home/u/.ssh/id': ",
        ] {
            let out = run(prompt);
            assert!(
                !out.contains("key-phrase") && out.contains("helper-exit=1"),
                "{prompt}: {out}"
            );
        }
    }

    #[test]
    fn answered_secrets_are_wiped_and_the_file_goes_with_the_last_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sshub-askpass-test");
        std::fs::write(
            &file,
            "dest-pw\nC\tp\nS\tkeyring\t\nH\tp\tops\ta\t\ta\tk:one-ref\nH\tp\tops\tb\t\tb\tk:two-ref\n",
        )
        .unwrap();
        consume(&file, &Answered::Hop(0));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains("one-ref") && text.contains("two-ref") && text.contains("dest-pw"));
        consume(&file, &Answered::Destination);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains("dest-pw") && text.contains("two-ref"));
        // The last secret takes the file with it: nothing stays on disk for the
        // life of a detached tunnel.
        consume(&file, &Answered::Hop(0));
        assert!(!file.exists());

        std::fs::write(&file, "only-dest\nC\tp\n").unwrap();
        consume(&file, &Answered::Destination);
        assert!(!file.exists());

        // References alone (no destination secret) are what a detached tunnel
        // through a key-auth jump stages: the file still goes with the last one.
        std::fs::write(&file, "\nS\tkeyring\t\nH\tp\tops\ta\t\ta\tk:one-ref\n").unwrap();
        consume(&file, &Answered::Hop(0));
        assert!(!file.exists());
    }

    #[test]
    fn a_hop_field_the_format_cannot_hold_is_left_out_not_misparsed() {
        let hop = |key: &str| super::super::hop_auth::HopSecret {
            user: Some("ops".into()),
            host: "h".into(),
            via: "h".into(),
            key_path: Some(key.into()),
            class: PromptClass::Passphrase,
            refs: vec![SecretRef::Key("identity:1".into())],
            store: Some(StoreLocator::Keyring {
                prefix: String::new(),
            }),
        };
        let guard =
            AskpassSecret::staged(None, &[hop("/k\twith-tab"), hop("/k\nline"), hop("/ok")])
                .unwrap();
        let text = std::fs::read_to_string(&guard.path).unwrap();
        let staged = parse_staged(&text);
        assert_eq!(staged.hops.len(), 1);
        assert_eq!(staged.hops[0].key_path.as_deref(), Some("/ok"));
    }

    #[test]
    fn a_hop_without_a_reopenable_store_is_not_staged() {
        let guard = AskpassSecret::staged(
            None,
            &[super::super::hop_auth::HopSecret {
                user: None,
                host: "h".into(),
                via: "h".into(),
                key_path: None,
                class: PromptClass::Password,
                refs: vec![SecretRef::Key("host:1".into())],
                store: None,
            }],
        )
        .unwrap();
        let text = std::fs::read_to_string(&guard.path).unwrap();
        assert!(parse_staged(&text).hops.is_empty(), "{text}");
    }

    /// Every regular file under `root`, contents included.
    fn all_file_text(root: &Path) -> String {
        let mut text = String::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                let kind = entry.file_type().unwrap();
                if kind.is_dir() {
                    stack.push(path);
                } else if kind.is_file() {
                    text.push_str(&String::from_utf8_lossy(&std::fs::read(&path).unwrap()));
                    text.push_str(&path.to_string_lossy());
                }
            }
        }
        text
    }

    fn synced_hop(secret: &str) -> super::super::hop_auth::HopSecret {
        super::super::hop_auth::HopSecret {
            user: Some("ops".into()),
            host: "10.0.4.11".into(),
            via: "lb-1".into(),
            key_path: None,
            class: PromptClass::Password,
            refs: vec![SecretRef::Staged(crate::credentials::StagedValue::new(
                secret.into(),
            ))],
            store: None,
        }
    }

    /// Open `value` in a throwaway gate so the staging treats it as synced.
    fn open_synced(value: &str) -> crate::credentials::SecretGate {
        let gate = crate::credentials::SecretGate::default();
        gate.insert("host:1", zeroize::Zeroizing::new(value.to_string()));
        gate
    }

    #[test]
    fn a_synced_hop_secret_never_reaches_a_file_and_the_helper_gets_it_over_the_socket() {
        let root = tempfile::tempdir().unwrap();
        let guard =
            AskpassSecret::staged_in(root.path(), None, &[synced_hop("hop-s3cr3t")]).unwrap();
        assert!(guard.holds_synced());
        let on_disk = all_file_text(root.path());
        assert!(!on_disk.contains("hop-s3cr3t"), "plaintext reached a file");
        assert!(on_disk.contains("s:s0"), "the file holds only a handle");

        // What the helper does: parse the file, ask for the handle.
        let text = std::fs::read_to_string(guard.path()).unwrap();
        let staged = parse_staged(&text);
        assert!(matches!(
            staged.hops[0].refs.as_slice(),
            [SecretRef::Handle(_)]
        ));
        let env = guard.env(Path::new("/x"));
        let get = |k: &str| {
            env.iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        let (sock, token) = (get(VALUES_SOCK_ENV), get(VALUES_TOKEN_ENV));
        assert_eq!(
            fetch_held_from(Path::new(&sock), &token, "s0").as_deref(),
            Some("hop-s3cr3t")
        );
        // A wrong token or an unknown handle gets nothing.
        assert_eq!(fetch_held_from(Path::new(&sock), "wrong", "s0"), None);
        assert_eq!(fetch_held_from(Path::new(&sock), &token, "s9"), None);
        // The socket directory is private to the owner.
        use std::os::unix::fs::PermissionsExt;
        let dir = Path::new(&sock).parent().unwrap().to_path_buf();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );

        // Dropping the guard removes the socket and its directory.
        drop(guard);
        assert!(!Path::new(&sock).exists() && !dir.exists());
        assert_eq!(all_file_text(root.path()), "", "nothing is left behind");
    }

    #[test]
    fn a_synced_destination_secret_is_held_not_written() {
        let root = tempfile::tempdir().unwrap();
        let _gate = open_synced("dest-s3cr3t");
        let secret = PendingSecret::Password("dest-s3cr3t".into());
        let guard = AskpassSecret::staged_in(root.path(), Some(&secret), &[]).unwrap();
        let text = std::fs::read_to_string(guard.path()).unwrap();
        assert!(!all_file_text(root.path()).contains("dest-s3cr3t"));
        let staged = parse_staged(&text);
        assert_eq!(staged.destination, "");
        assert_eq!(staged.destination_handle, Some("d"));
        assert_eq!(staged.class, Some(PromptClass::Password));
        let env = guard.env(Path::new("/x"));
        let get = |k: &str| {
            env.iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(
            fetch_held_from(
                Path::new(&get(VALUES_SOCK_ENV)),
                &get(VALUES_TOKEN_ENV),
                "d"
            )
            .as_deref(),
            Some("dest-s3cr3t")
        );
    }

    #[test]
    fn a_local_secret_is_staged_in_the_file_as_before_with_no_server() {
        let root = tempfile::tempdir().unwrap();
        let secret = PendingSecret::Password("local-s3cr3t".into());
        let guard = AskpassSecret::staged_in(root.path(), Some(&secret), &[]).unwrap();
        assert!(!guard.holds_synced());
        assert!(std::fs::read_to_string(guard.path())
            .unwrap()
            .starts_with("local-s3cr3t\n"));
        assert!(guard
            .env(Path::new("/x"))
            .iter()
            .all(|(k, _)| k != VALUES_SOCK_ENV));
    }

    #[test]
    fn a_synced_value_ssh_could_not_take_as_one_line_is_not_staged() {
        let root = tempfile::tempdir().unwrap();
        let guard =
            AskpassSecret::staged_in(root.path(), None, &[synced_hop("two\nlines")]).unwrap();
        assert!(!guard.holds_synced(), "nothing to serve, no server");
        assert!(!all_file_text(root.path()).contains("two"));
    }

    #[test]
    fn askpass_mode_off_without_env() {
        std::env::remove_var(ASKPASS_FILE_ENV);
        assert!(!maybe_run_askpass());
    }
}

#[cfg(test)]
mod exe_tests {
    use super::*;

    #[test]
    fn helper_exe_is_the_running_binary() {
        // The test binary exists, so this is the plain path with no marker.
        let exe = helper_exe().unwrap();
        assert!(exe.exists(), "{exe:?}");
        assert_eq!(exe, std::env::current_exe().unwrap());
    }

    #[test]
    fn a_replaced_binary_resolves_to_whatever_took_its_place() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("sshub");
        std::fs::write(&real, b"new binary").unwrap();

        // What /proc/self/exe reads once an upgrade replaced the file.
        let deleted = PathBuf::from(format!("{} (deleted)", real.display()));
        assert_eq!(
            strip_deleted_marker(&deleted).as_deref(),
            Some(real.as_path())
        );

        // Nothing took its place: no path to offer, and the caller must say so
        // rather than handing ssh something that cannot be executed.
        std::fs::remove_file(&real).unwrap();
        assert_eq!(strip_deleted_marker(&deleted), None);
    }

    #[test]
    fn a_path_without_the_marker_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("sshub");
        std::fs::write(&real, b"binary").unwrap();
        assert_eq!(strip_deleted_marker(&real), None);
    }
}
