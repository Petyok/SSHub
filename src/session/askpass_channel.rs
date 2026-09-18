//! Private, bounded, nonblocking TUI side of SSH_ASKPASS.
//!
//! A fresh capability is issued to each ssh child, never to another session.
//! Each helper invocation owns a separate stream; replies cannot cross callers.
//! No prompts, capabilities, answers, or serialization errors enter diagnostics.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const SOCKET_ENV: &str = "SSHUB_ASKPASS_SOCKET";
pub const TOKEN_ENV: &str = "SSHUB_ASKPASS_TOKEN";
pub const MODE_ENV: &str = "SSHUB_ASKPASS_CHANNEL";
pub const TIMEOUT: Duration = Duration::from_secs(120);
pub const MAX_ANSWER_BYTES: usize = 4096;
const MAX_FRAME: usize = 32 * 1024;
const MAX_CLIENTS: usize = 8;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRequest {
    token: String,
    prompt: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", content = "answer", deny_unknown_fields)]
enum WireReply {
    Answer(String),
    Cancel,
}

fn refused() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "askpass request refused")
}

fn valid_answer(answer: &str) -> bool {
    answer.len() <= MAX_ANSWER_BYTES && !answer.contains(['\n', '\r', '\0'])
}

/// One modal's request. Dropping this closes the helper connection (failure).
pub struct Request {
    pub prompt: String,
    stream: UnixStream,
    deadline: Instant,
}

impl Request {
    /// Observe timeout and helper disappearance without blocking the event loop.
    pub fn is_live(&mut self) -> bool {
        if Instant::now() >= self.deadline {
            return false;
        }
        let mut byte = [0];
        matches!(self.stream.read(&mut byte), Err(e) if e.kind() == io::ErrorKind::WouldBlock)
    }

    pub fn answer(mut self, answer: &str) -> io::Result<()> {
        if !valid_answer(answer) || !self.is_live() {
            return Err(refused());
        }
        // A bounded reply fits the ordinary Unix socket buffer. If the receiver
        // cannot accept it immediately, close rather than stall the TUI or retry
        // after the modal has closed.
        let mut bytes =
            serde_json::to_vec(&WireReply::Answer(answer.to_owned())).map_err(|_| refused())?;
        bytes.push(b'\n');
        self.stream.write_all(&bytes)
    }
}

struct Client {
    stream: UnixStream,
    bytes: Vec<u8>,
    deadline: Instant,
}

pub struct Channel {
    listener: UnixListener,
    // The directory is 0700 from creation; even before chmod of the socket no
    // other uid can connect. Drop removes the socket and directory together.
    directory: tempfile::TempDir,
    token: String,
    clients: VecDeque<Client>,
    ready: VecDeque<Request>,
    timeout: Duration,
}

impl Channel {
    pub fn new() -> io::Result<Self> {
        let root = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        Self::in_directory(&root, TIMEOUT)
    }

    fn in_directory(root: &Path, timeout: Duration) -> io::Result<Self> {
        let directory = tempfile::Builder::new()
            .prefix("sshub-auth-")
            .tempdir_in(root)?;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
        let path = directory.path().join("socket");
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let mut random = [0u8; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        let mut token = String::with_capacity(64);
        use std::fmt::Write as _;
        for byte in random {
            let _ = write!(token, "{byte:02x}");
        }
        Ok(Self {
            listener,
            directory,
            token,
            clients: VecDeque::new(),
            ready: VecDeque::new(),
            timeout,
        })
    }

    pub fn env(&self, exe: &Path) -> Vec<(String, String)> {
        vec![
            ("SSH_ASKPASS".into(), exe.to_string_lossy().into_owned()),
            ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
            // Nonempty DISPLAY also supports OpenSSH versions checking it.
            ("DISPLAY".into(), "sshub:0".into()),
            (MODE_ENV.into(), "1".into()),
            (
                SOCKET_ENV.into(),
                self.directory
                    .path()
                    .join("socket")
                    .to_string_lossy()
                    .into_owned(),
            ),
            (TOKEN_ENV.into(), self.token.clone()),
        ]
    }

    /// Work per tick is bounded, including unauthenticated/partial clients.
    pub fn poll(&mut self) {
        for _ in 0..MAX_CLIENTS {
            let Ok((stream, _)) = self.listener.accept() else {
                break;
            };
            if self.clients.len() + self.ready.len() >= MAX_CLIENTS {
                continue;
            }
            if stream.set_nonblocking(true).is_ok() {
                self.clients.push_back(Client {
                    stream,
                    bytes: Vec::new(),
                    deadline: Instant::now() + self.timeout,
                });
            }
        }
        let count = self.clients.len();
        for _ in 0..count {
            let Some(mut client) = self.clients.pop_front() else {
                break;
            };
            if Instant::now() >= client.deadline {
                continue;
            }
            let mut buffer = [0u8; 4096];
            let mut closed = false;
            for _ in 0..(MAX_FRAME / buffer.len() + 1) {
                match client.stream.read(&mut buffer) {
                    Ok(0) => {
                        closed = true;
                        break;
                    }
                    Ok(n) => {
                        client.bytes.extend_from_slice(&buffer[..n]);
                        if client.bytes.len() > MAX_FRAME || client.bytes.contains(&b'\n') {
                            break;
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        closed = true;
                        break;
                    }
                }
            }
            if closed || client.bytes.len() > MAX_FRAME {
                continue;
            }
            if let Some(end) = client.bytes.iter().position(|b| *b == b'\n') {
                if end + 1 != client.bytes.len() {
                    continue;
                }
                let Ok(wire) = serde_json::from_slice::<WireRequest>(&client.bytes[..end]) else {
                    continue;
                };
                if wire.token != self.token || wire.prompt.is_empty() {
                    continue;
                }
                self.ready.push_back(Request {
                    prompt: wire.prompt,
                    stream: client.stream,
                    deadline: client.deadline,
                });
            } else {
                self.clients.push_back(client);
            }
        }
        self.ready.retain_mut(Request::is_live);
    }

    pub fn next_request(&mut self) -> Option<Request> {
        self.poll();
        self.ready.pop_front()
    }
}

/// Blocking helper side. Any incomplete/malformed/late reply is an error, never
/// an empty or guessed answer. The helper process owns the overall read timeout.
pub fn request_answer(
    path: &Path,
    token: &str,
    prompt: &str,
    timeout: Duration,
) -> io::Result<String> {
    if token.is_empty() || prompt.is_empty() {
        return Err(refused());
    }
    let mut bytes = serde_json::to_vec(&WireRequest {
        token: token.into(),
        prompt: prompt.into(),
    })
    .map_err(|_| refused())?;
    bytes.push(b'\n');
    if bytes.len() > MAX_FRAME {
        return Err(refused());
    }
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(&bytes)?;
    let deadline = Instant::now() + timeout;
    let mut reply = Vec::new();
    let mut buffer = [0u8; 1024];
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(refused)?;
        stream.set_read_timeout(Some(remaining))?;
        let n = stream.read(&mut buffer)?;
        if n == 0 {
            return Err(refused());
        }
        reply.extend_from_slice(&buffer[..n]);
        if reply.len() > MAX_FRAME {
            return Err(refused());
        }
        if let Some(end) = reply.iter().position(|byte| *byte == b'\n') {
            if end + 1 != reply.len() {
                return Err(refused());
            }
            return match serde_json::from_slice::<WireReply>(&reply[..end]) {
                Ok(WireReply::Answer(answer)) if valid_answer(&answer) => Ok(answer),
                _ => Err(refused()),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn client(
        channel: &Channel,
        token: String,
        prompt: &str,
    ) -> thread::JoinHandle<io::Result<String>> {
        let path = channel.directory.path().join("socket");
        let prompt = prompt.to_owned();
        thread::spawn(move || request_answer(&path, &token, &prompt, Duration::from_secs(2)))
    }

    fn next(channel: &mut Channel) -> Request {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(request) = channel.next_request() {
                return request;
            }
            assert!(Instant::now() < deadline, "no request arrived");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn replies_are_bound_to_their_request_and_queue_does_not_answer_early() {
        let root = tempfile::tempdir().unwrap();
        let mut channel = Channel::in_directory(root.path(), Duration::from_secs(2)).unwrap();
        let first = client(&channel, channel.token.clone(), "password:");
        let request = next(&mut channel);
        let second = client(&channel, channel.token.clone(), "Verification code:");
        let queued = next(&mut channel);
        assert!(!first.is_finished());
        assert!(!second.is_finished());
        request.answer("first-answer").unwrap();
        assert_eq!(first.join().unwrap().unwrap(), "first-answer");
        assert!(!second.is_finished());
        queued.answer("second-answer").unwrap();
        assert_eq!(second.join().unwrap().unwrap(), "second-answer");
    }

    #[test]
    fn bad_token_and_missing_token_never_reach_ui() {
        let root = tempfile::tempdir().unwrap();
        let mut channel = Channel::in_directory(root.path(), Duration::from_secs(1)).unwrap();
        let bad = client(&channel, "wrong-token".into(), "password:");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !bad.is_finished() {
            assert!(channel.next_request().is_none());
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(bad.join().unwrap().is_err());
        assert!(request_answer(&root.path().join("missing"), "", "password:", TIMEOUT).is_err());
    }

    #[test]
    fn cancellation_timeout_and_channel_drop_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let mut channel = Channel::in_directory(root.path(), Duration::from_millis(30)).unwrap();
        let cancelled = client(&channel, channel.token.clone(), "password:");
        drop(next(&mut channel));
        assert!(cancelled.join().unwrap().is_err());
        let timed_out = client(&channel, channel.token.clone(), "password:");
        let request = next(&mut channel);
        thread::sleep(Duration::from_millis(40));
        assert!(request.answer("late").is_err());
        assert!(timed_out.join().unwrap().is_err());
        let path = channel.directory.path().join("socket");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        drop(channel);
        assert!(!path.exists());
        assert!(request_answer(&path, "stale", "password:", TIMEOUT).is_err());
    }

    #[test]
    fn auth_cross_session_token_and_queued_deadline_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let mut first = Channel::in_directory(root.path(), Duration::from_secs(2)).unwrap();
        let second = Channel::in_directory(root.path(), Duration::from_secs(2)).unwrap();
        let stale = client(&first, second.token.clone(), "Password:");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !stale.is_finished() {
            assert!(first.next_request().is_none());
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(stale.join().unwrap().is_err());
        let active = client(&first, first.token.clone(), "Password:");
        let request = next(&mut first);
        let waiting = client(&first, first.token.clone(), "Verification code:");
        let deadline = Instant::now() + Duration::from_secs(2);
        while first.ready.is_empty() {
            first.poll();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        // Expire the queued request's original deadline without sleeping or
        // granting a fresh timeout when it reaches the front of the queue.
        first.ready.front_mut().unwrap().deadline = Instant::now();
        assert!(first.next_request().is_none());
        assert!(waiting.join().unwrap().is_err());
        request.answer("only-active").unwrap();
        assert_eq!(active.join().unwrap().unwrap(), "only-active");
    }

    #[test]
    fn auth_helper_process_entry() {
        if std::env::var_os("SSHUB_TEST_HELPER_PROCESS").is_some() {
            assert!(super::super::askpass::maybe_run_askpass());
            std::process::exit(0);
        }
    }

    fn helper_process(channel: &Channel) -> std::process::Child {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "session::askpass_channel::tests::auth_helper_process_entry",
                "--nocapture",
            ])
            .env("SSHUB_TEST_HELPER_PROCESS", "1")
            .env_remove("SSHUB_ASKPASS_FILE")
            .envs(channel.env(Path::new("unused")))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        command.spawn().unwrap()
    }

    #[test]
    fn auth_helper_process_cancellation_timeout_and_closed_socket_exit_nonzero() {
        let root = tempfile::tempdir().unwrap();
        for outcome in ["cancel", "timeout", "closed", "malformed"] {
            let mut channel = Channel::in_directory(root.path(), Duration::from_secs(2)).unwrap();
            if outcome == "closed" {
                let mut command = std::process::Command::new(std::env::current_exe().unwrap());
                command
                    .args([
                        "--exact",
                        "session::askpass_channel::tests::auth_helper_process_entry",
                    ])
                    .env("SSHUB_TEST_HELPER_PROCESS", "1")
                    .env_remove("SSHUB_ASKPASS_FILE")
                    .envs(channel.env(Path::new("unused")));
                drop(channel);
                assert!(!command.output().unwrap().status.success());
                continue;
            }
            let child = helper_process(&channel);
            let mut request = next(&mut channel);
            match outcome {
                "timeout" => {
                    request.deadline = Instant::now();
                    assert!(request.answer("must-not-be-printed").is_err());
                }
                "malformed" => {
                    request.stream.write_all(b"not-json\n").unwrap();
                    drop(request);
                }
                _ => drop(request),
            }
            let output = child.wait_with_output().unwrap();
            assert!(
                !output.status.success(),
                "{outcome} helper must exit nonzero"
            );
            assert!(!String::from_utf8_lossy(&output.stdout).contains("must-not-be-printed"));
            assert!(!String::from_utf8_lossy(&output.stderr).contains("must-not-be-printed"));
        }
    }
}
