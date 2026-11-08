//! Test connect wiring (issue #79): build the steps for a chain, run them on a
//! worker, and fold the events into [`TestRun`] for the map and the hop editor.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use super::network_map::MapSel;
use super::*;
use crate::map::{hops, parse_raw_hop, Chain, HopKey, HopKind};
use crate::ssh::test_connect::{
    spawn_test, test_runner, SecretSource, StepState, TestEvent, TestStep,
};

/// Who asked for the run, so only that screen shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestOwner {
    Map(MapSel),
    Editor,
}

pub struct TestRun {
    pub owner: TestOwner,
    /// What is being tested, for titles.
    pub title: String,
    pub steps: Vec<TestStep>,
    pub states: Vec<StepState>,
    rx: Receiver<TestEvent>,
    cancel: Arc<AtomicBool>,
    pub done: bool,
}

impl Drop for TestRun {
    /// Leaving the map, closing the form or quitting stops ssh and its `-W`
    /// helpers instead of letting them run to the step timeout.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        // The worker is detached: it kills the ssh process group when it sees
        // the flag (within its poll interval) and then sends `Done`. Wait for
        // that, briefly, so no ssh or `-W` helper outlives sshub.
        if !self.done {
            let until = std::time::Instant::now() + std::time::Duration::from_millis(500);
            while let Some(left) = until.checked_duration_since(std::time::Instant::now()) {
                match self.rx.recv_timeout(left) {
                    Ok(TestEvent::Done) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        }
    }
}

impl TestRun {
    pub fn running(&self) -> bool {
        !self.done
    }

    /// First failed step.
    pub fn failure(&self) -> Option<(usize, &str, &str)> {
        self.states.iter().enumerate().find_map(|(i, s)| match s {
            StepState::Failed { reason, detail } => Some((i, reason.as_str(), detail.as_str())),
            _ => None,
        })
    }

    /// A step was skipped because its hop is not a valid hop.
    pub fn refused(&self) -> bool {
        self.states.iter().any(|s| {
            matches!(s, StepState::Skipped { reason } if reason.contains("invalid") || reason.contains("not a valid"))
        })
    }

    /// Title line: progress while running, outcome after.
    pub fn headline(&self) -> String {
        let total = self.steps.len();
        if let Some((i, reason, _)) = self.failure() {
            return format!("failed at hop {} of {total} ({reason})", i + 1);
        }
        if self.done {
            if self.states.contains(&StepState::Cancelled) {
                return "cancelled".into();
            }
            // A step that was skipped was not tested: that is not "ok".
            if self.refused() {
                return "refused: invalid hop".into();
            }
            let ok = self
                .states
                .iter()
                .filter(|s| matches!(s, StepState::Ok { .. }))
                .count();
            if ok < total {
                return format!("{ok} of {total} steps tested, the rest not tested");
            }
            return format!("all {total} steps ok");
        }
        let at = self
            .states
            .iter()
            .position(|s| matches!(s, StepState::Running | StepState::Pending))
            .unwrap_or(total.saturating_sub(1));
        format!("step {} of {total}", at + 1)
    }
}

/// The target a chain test ends on.
pub(crate) enum TestTarget {
    /// A host of [`App::hosts`].
    Host(usize),
    /// The host form being edited (not necessarily saved).
    Form,
    /// The last hop is the target: no extra step.
    None,
}

/// One hop of the chain under test: a name/raw token, or a ProxyCommand hop
/// (shown, skipped, and not part of the `-J` prefix).
pub(crate) type TestHop = Option<String>;

const PROXY_SKIP: &str = "proxy cmd, not tested";

impl App {
    /// The hops, final target and title for the map cursor's path; sets a
    /// notice and returns `None` when there is nothing to build.
    fn map_chain_for_cursor(&mut self) -> Option<(MapSel, Vec<TestHop>, TestTarget, String)> {
        let v = self.map_view.as_ref()?;
        let sel = v.selection()?;
        let map = &v.map;
        let hop_name = |k: &HopKey| -> TestHop {
            match k {
                HopKey::Saved(h) => Some(map.hosts[*h].name.clone()),
                HopKey::Raw(r) => Some(r.label()),
                HopKey::Invalid(t) => Some(t.clone()),
                HopKey::ProxyCmd(_) => None,
            }
        };
        let (hops, target, title) = match sel {
            MapSel::Hop(n) => {
                let hops: Vec<TestHop> = map
                    .node_path(n)
                    .into_iter()
                    .map(|p| hop_name(&map.nodes[p].key))
                    .collect();
                (hops, TestTarget::None, map.key_label(&map.nodes[n].key))
            }
            MapSel::Host(h) => match &map.chains[h] {
                Chain::Hops(c) => {
                    let hops = c
                        .iter()
                        .map(|hop| match &hop.kind {
                            HopKind::ProxyCommand(_) => None,
                            // The token as written, so `user@` and `:port`
                            // reach ssh exactly as a real connect sends them.
                            _ => Some(hop.token.clone()),
                        })
                        .collect();
                    (hops, TestTarget::Host(h), map.hosts[h].name.clone())
                }
                Chain::Cycle(_) => {
                    self.host_notice = Some("jump loop: nothing to test, fix the ProxyJump".into());
                    return None;
                }
            },
            _ => {
                self.host_notice = Some("put the cursor on a hop or host to use its path".into());
                return None;
            }
        };
        Some((sel, hops, target, title))
    }

    /// `t` on the map: test the path of what the cursor is on.
    pub(crate) fn map_start_test(&mut self) {
        if self.test_run.as_ref().is_some_and(TestRun::running) {
            self.host_notice = Some("a test is already running (Esc cancels it)".into());
            return;
        }
        let Some((sel, hops, target, title)) = self.map_chain_for_cursor() else {
            return;
        };
        self.start_test(TestOwner::Map(sel), title, &hops, target);
    }

    /// `y` on the map: the real command for the cursor's path, every word
    /// shell-quoted (hop and host names come from imports and can hold anything).
    pub(crate) fn map_command_line(&mut self) -> Option<String> {
        let (_, hops, target, _) = self.map_chain_for_cursor()?;
        let steps = self.test_steps(&hops, target);
        let last = steps.last()?;
        if let Some(why) = &last.skip {
            self.host_notice = Some(format!("no command: {why}"));
            return None;
        }
        Some(
            last.argv
                .iter()
                .map(|w| crate::session_log::shell_quote(w))
                .collect::<Vec<_>>()
                .join(" "),
        )
    }

    /// `t` in the hop editor: test the chain as it is in the form, ending on
    /// the host being edited.
    pub(crate) fn editor_start_test(&mut self) {
        if self.test_run.as_ref().is_some_and(TestRun::running) {
            return;
        }
        let Some(form) = self.host_form.as_ref() else {
            return;
        };
        let own = form.proxy_jump.trim();
        let tokens = if own.is_empty() {
            // The inherited chain is what a connect would use.
            let primary = self
                .groups
                .iter()
                .find(|g| form.group_ids.contains(&g.id))
                .map(|g| g.id);
            let selected: Vec<i64> = form.group_ids.iter().copied().collect();
            self.store
                .inherited_for_groups(primary, &selected)
                .ok()
                .and_then(|r| r.proxy_jump)
                .map(|j| hops::split(&j))
                .unwrap_or_default()
        } else {
            hops::split(own)
        };
        let title = if form.name.trim().is_empty() {
            "this host".to_string()
        } else {
            form.name.trim().to_string()
        };
        let hops: Vec<TestHop> = tokens.into_iter().map(Some).collect();
        self.start_test(TestOwner::Editor, title, &hops, TestTarget::Form);
    }

    fn test_steps(&self, hops: &[TestHop], target: TestTarget) -> Vec<TestStep> {
        let mut steps: Vec<TestStep> = Vec::new();
        for i in 0..hops.len() {
            steps.push(self.hop_test_step(hops, i));
        }
        match target {
            TestTarget::None => {}
            TestTarget::Host(h) => {
                let prefix = self.test_prefix(hops, hops.len());
                steps.push(self.entry_test_step(h, prefix));
            }
            TestTarget::Form => {
                let prefix = self.test_prefix(hops, hops.len());
                steps.push(self.form_test_step(prefix));
            }
        }
        // `ssh -J bastion-eu,jump-2 db-2 true`: names as written, for panels.
        for (i, step) in steps.iter_mut().enumerate() {
            // The hops before this step keep their own stored secrets, fetched
            // when the step runs; the step's own secret is its own.
            let before = hops_before(&hops[..i.min(hops.len())]);
            if step.skip.is_none() && !before.is_empty() {
                let (store, entries, passwords) = (
                    Arc::clone(&self.store),
                    self.hosts.clone(),
                    Arc::clone(&self.password_store),
                );
                step.hops = Some(Arc::new(move || {
                    hop_secrets(&store, &entries, passwords.as_ref(), Some(&before))
                }));
            }
            let via: Vec<&str> = hops[..i.min(hops.len())]
                .iter()
                .flatten()
                .map(String::as_str)
                .collect();
            step.shown = if via.is_empty() {
                format!("ssh {} true", step.label)
            } else {
                format!("ssh -J {} {} true", via.join(","), step.label)
            };
        }
        steps
    }

    fn start_test(
        &mut self,
        owner: TestOwner,
        title: String,
        hops: &[TestHop],
        target: TestTarget,
    ) {
        let steps = self.test_steps(hops, target);
        if steps.is_empty() {
            self.host_notice = Some("nothing to test".into());
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let rx = spawn_test(steps.clone(), Arc::new(test_runner()), Arc::clone(&cancel));
        self.test_run = Some(TestRun {
            owner,
            title,
            states: vec![StepState::Pending; steps.len()],
            steps,
            rx,
            cancel,
            done: false,
        });
    }

    /// Esc: stop a running test, or clear a finished one. `true` when it did.
    pub(crate) fn cancel_or_clear_test(&mut self, owner_is_editor: bool) -> bool {
        let Some(run) = self.test_run.as_mut() else {
            return false;
        };
        if (run.owner == TestOwner::Editor) != owner_is_editor {
            return false;
        }
        if run.running() {
            run.cancel.store(true, Ordering::Relaxed);
        } else {
            self.test_run = None;
        }
        true
    }

    /// Fold worker events into the run; called every tick.
    pub fn poll_test_run(&mut self) {
        // The form this test belongs to is gone: stop it.
        if self.host_form.is_none()
            && self
                .test_run
                .as_ref()
                .is_some_and(|r| r.owner == TestOwner::Editor)
        {
            self.test_run = None;
            return;
        }
        let Some(run) = self.test_run.as_mut() else {
            return;
        };
        let mut log: Vec<crate::ssh::probe::SshLogEntry> = Vec::new();
        loop {
            let ev = match run.rx.try_recv() {
                Ok(ev) => ev,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                // The worker is gone without a `Done`: do not wait for it.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if !run.done {
                        for st in &mut run.states {
                            if matches!(st, StepState::Pending | StepState::Running) {
                                *st = StepState::Cancelled;
                            }
                        }
                    }
                    run.done = true;
                    break;
                }
            };
            match ev {
                TestEvent::Step { index, state } => {
                    // A failed step leaves its command and reason in the ssh
                    // log of that host, where `l` on the map shows it.
                    if let (StepState::Failed { reason, detail }, Some(step)) =
                        (&state, run.steps.get(index))
                    {
                        let host = step.host.clone().unwrap_or_else(|| step.label.clone());
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_secs() as i64);
                        let entry = |line: String, level| crate::ssh::probe::SshLogEntry {
                            host_name: host.clone(),
                            line,
                            level,
                            timestamp: now,
                        };
                        log.push(entry(
                            format!("$ {}", step.cmd),
                            crate::ssh::probe::LogLevel::Info,
                        ));
                        log.push(entry(
                            format!("test connect: {reason} {detail}")
                                .trim_end()
                                .to_string(),
                            crate::ssh::probe::LogLevel::Error,
                        ));
                    }
                    if let Some(slot) = run.states.get_mut(index) {
                        *slot = state;
                    }
                }
                TestEvent::Done => run.done = true,
            }
        }
        for e in log {
            self.push_ssh_log(e);
        }
    }

    /// `-J` string for the first `n` hops (ProxyCommand hops are not in it),
    /// expanded the way connect expands it. `None` when there is none.
    fn test_prefix(&self, hops: &[TestHop], n: usize) -> Result<Option<String>, String> {
        let names: Vec<String> = hops[..n].iter().flatten().cloned().collect();
        // ssh would run that hop's ProxyCommand to get through it: not tested.
        if names.iter().any(|t| self.token_has_proxy_command(t)) {
            return Err(PROXY_SKIP.to_string());
        }
        if names.is_empty() {
            return Ok(None);
        }
        self.store
            .expand_jump(&hops::join(&names))
            .map(Some)
            .map_err(|e| format!("{e:#}"))
    }

    /// The saved host a hop token names (`name`, `user@name`, `name:port`).
    fn token_host(&self, token: &str) -> Option<usize> {
        let by_name = |n: &str| self.hosts.iter().position(|h| h.name() == n);
        by_name(token).or_else(|| {
            parse_raw_hop(token)
                .filter(|r| !r.ipv6)
                .and_then(|r| by_name(&r.host))
        })
    }

    fn token_has_proxy_command(&self, token: &str) -> bool {
        self.token_host(token)
            .is_some_and(|i| entry_has_proxy_command(&self.hosts[i]))
    }

    fn hop_test_step(&self, hops: &[TestHop], i: usize) -> TestStep {
        let Some(token) = hops[i].as_deref() else {
            return skipped("proxy cmd", PROXY_SKIP);
        };
        let prefix = match self.test_prefix(hops, i) {
            Ok(p) => p,
            Err(e) => return skipped(token, &e),
        };
        if let Some(idx) = self.token_host(token) {
            return self.entry_test_step(idx, Ok(prefix));
        }
        match parse_raw_hop(token) {
            Some(raw) => {
                let mut host = SshHost::new(raw.host.clone());
                host.hostname = Some(raw.host);
                host.user = raw.user;
                host.port = raw.port;
                host.proxy_jump = prefix;
                finish(token, None, crate::ssh::build_ssh_argv(&host), None)
            }
            None => skipped(token, "invalid hop, not tested"),
        }
    }

    /// Step for a saved host reached through `prefix`.
    fn entry_test_step(&self, idx: usize, prefix: Result<Option<String>, String>) -> TestStep {
        let entry = &self.hosts[idx];
        let name = entry.name().to_string();
        let prefix = match prefix {
            Ok(p) => p,
            Err(e) => return skipped(&name, &e),
        };
        if entry_has_proxy_command(entry) {
            return skipped(&name, PROXY_SKIP);
        }
        let alias = |prefix: &Option<String>| {
            let mut argv = vec!["ssh".to_string()];
            if let Some(j) = prefix {
                argv.push("-J".into());
                argv.push(j.clone());
            }
            argv.push(crate::ssh::safe_ssh_target(name.clone()));
            argv
        };
        match entry {
            HostEntry::Legacy { .. } => finish(&name, Some(name.clone()), alias(&prefix), None),
            HostEntry::Managed(m) => {
                let resolved = self
                    .store
                    .resolve_connection(m)
                    .unwrap_or_else(|_| crate::store::ResolvedConnection::from_stored(m));
                let secret: SecretSource = {
                    let entry = entry.clone();
                    let identity = resolved.identity.clone();
                    let store = Arc::clone(&self.password_store);
                    Arc::new(move || {
                        resolve_pending_secret(&entry, identity.as_ref(), store.as_ref()).0
                    })
                };
                let argv = if m.source == HostSource::SshConfig {
                    alias(&prefix)
                } else {
                    let mut host = resolved_to_ssh_host(m, &resolved);
                    host.proxy_jump = prefix;
                    host.remote_command = None;
                    crate::ssh::build_ssh_argv(&host)
                };
                finish(&name, Some(name.clone()), argv, Some(secret))
            }
        }
    }

    /// The last step of an editor test: the host as the form has it.
    fn form_test_step(&self, prefix: Result<Option<String>, String>) -> TestStep {
        let Some(form) = self.host_form.as_ref() else {
            return skipped("this host", "no form");
        };
        let name = if form.name.trim().is_empty() {
            "this host".to_string()
        } else {
            form.name.trim().to_string()
        };
        let prefix = match prefix {
            Ok(p) => p,
            Err(e) => return skipped(&name, &e),
        };
        let identity = form
            .identity_index
            .checked_sub(1)
            .and_then(|i| self.identities.get(i));
        let mut host = SshHost::new(name.clone());
        host.hostname = Some(form.address.trim().to_string());
        host.user =
            optional_field(&form.username).or_else(|| identity.and_then(|i| i.username.clone()));
        host.port = form.port.trim().parse().ok();
        host.identity_file = identity
            .and_then(|i| i.private_key.as_ref())
            .map(|p| p.to_string_lossy().into_owned());
        host.proxy_jump = prefix;
        if host.hostname.as_deref().unwrap_or("").is_empty() {
            return skipped(&name, "no address yet");
        }
        finish(&name, None, crate::ssh::build_ssh_argv(&host), None)
    }
}

/// The saved-host hops of a chain as one ProxyJump string.
fn hops_before(hops: &[TestHop]) -> String {
    let names: Vec<String> = hops.iter().flatten().cloned().collect();
    hops::join(&names)
}

/// The host's ssh_config sets a ProxyCommand (presence only).
fn entry_has_proxy_command(entry: &HostEntry) -> bool {
    match entry {
        HostEntry::Legacy { host, .. } => host.has_proxy_command,
        HostEntry::Managed(m) => m
            .ssh_config_hash
            .as_deref()
            .is_some_and(crate::ssh::ssh_config_hash_has_proxy_command),
    }
}

fn skipped(label: &str, reason: &str) -> TestStep {
    TestStep {
        label: label.to_string(),
        host: None,
        argv: Vec::new(),
        secret: None,
        hops: None,
        cmd: String::new(),
        shown: String::new(),
        skip: Some(reason.to_string()),
    }
}

fn finish(
    label: &str,
    host: Option<String>,
    argv: Vec<String>,
    secret: Option<SecretSource>,
) -> TestStep {
    TestStep {
        label: label.to_string(),
        host,
        cmd: format!("{} true", argv.join(" ")),
        shown: String::new(),
        argv,
        secret,
        hops: None,
        skip: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{host, test_app};
    use crate::store::NewHost;

    #[test]
    fn a_hop_with_proxy_command_stops_every_step_that_goes_through_it() {
        let mut edge = host("edge");
        edge.has_proxy_command = true;
        let mut app = test_app(vec![("edge", edge), ("tail", host("tail"))]);
        app.reload_hosts().unwrap();
        let hops: Vec<TestHop> = vec![Some("edge".into()), Some("tail".into())];

        // The prefix of any step after `edge` must not become `ssh -J edge`.
        assert_eq!(app.test_prefix(&hops, 1), Err(PROXY_SKIP.to_string()));
        let steps = [
            app.hop_test_step(&hops, 0),
            app.hop_test_step(&hops, 1),
            app.entry_test_step(1, app.test_prefix(&hops, 2)),
        ];
        for (i, step) in steps.iter().enumerate() {
            assert_eq!(step.skip.as_deref(), Some(PROXY_SKIP), "step {i}");
            assert!(step.argv.is_empty(), "step {i} must not run ssh");
        }
    }

    #[test]
    fn hop_tokens_keep_their_user_and_port_in_the_prefix() {
        let mut app = test_app(vec![]);
        let mut j = NewHost::launcher("jump", "10.0.0.2");
        j.username = Some("ops".into());
        app.store().create_host(&j).unwrap();
        app.reload_hosts().unwrap();
        let hops: Vec<TestHop> = vec![Some("root@jump:2200".into()), Some("x".into())];
        // What a real connect sends: the override wins over the stored values.
        assert_eq!(
            app.test_prefix(&hops, 1),
            Ok(Some("root@10.0.0.2:2200".to_string()))
        );
        // And the step for that hop finds the saved host behind the token.
        assert!(app.token_host("root@jump:2200").is_some());
    }

    struct CountingStore(Arc<std::sync::atomic::AtomicUsize>);

    impl crate::credentials::PasswordStore for CountingStore {
        fn get(&self, _: &str) -> anyhow::Result<Option<String>> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(Some("pw".into()))
        }
        fn set(&self, _: &str, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn delete(&self, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn building_steps_never_reads_the_keyring_only_running_one_does() {
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut app = crate::app::tests::test_app_with_store(
            vec![],
            Box::new(CountingStore(Arc::clone(&reads))),
        );
        let mut j = NewHost::launcher("jump", "10.0.0.2");
        j.has_password = true;
        app.store().create_host(&j).unwrap();
        app.reload_hosts().unwrap();
        let idx = app.hosts.iter().position(|h| h.name() == "jump").unwrap();
        // `y` builds the steps and prints the command: no secret is needed.
        let steps = app.test_steps(&[], TestTarget::Host(idx));
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        let fetch = steps[0]
            .secret
            .as_ref()
            .expect("a managed host has a source");
        // The worker asks right before the step runs.
        assert!(fetch().is_some());
        assert!(reads.load(Ordering::Relaxed) > 0);
    }

    fn run_with(owner: TestOwner, rx: Receiver<TestEvent>, cancel: Arc<AtomicBool>) -> TestRun {
        TestRun {
            owner,
            title: String::new(),
            steps: vec![skipped("a", "x"), skipped("b", "x")],
            states: vec![StepState::Running, StepState::Pending],
            rx,
            cancel,
            done: false,
        }
    }

    #[test]
    fn a_dead_worker_ends_the_run_and_dropping_a_run_cancels_it() {
        let mut app = test_app(vec![]);
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        app.test_run = Some(run_with(
            TestOwner::Map(MapSel::Hop(0)),
            rx,
            Arc::clone(&cancel),
        ));
        drop(tx);
        app.poll_test_run();
        let run = app.test_run.as_ref().unwrap();
        assert!(run.done && !run.running());
        assert_eq!(run.states, vec![StepState::Cancelled; 2]);
        assert!(!cancel.load(Ordering::Relaxed));
        // Esc clears it, which cancels the (already finished) worker flag.
        app.test_run = None;
        assert!(cancel.load(Ordering::Relaxed));

        // Closing the form that owns a running test stops the test.
        let (_tx, rx) = std::sync::mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        app.test_run = Some(run_with(TestOwner::Editor, rx, Arc::clone(&cancel)));
        assert!(app.host_form.is_none());
        app.poll_test_run();
        assert!(app.test_run.is_none());
        assert!(cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn a_group_form_with_an_invalid_default_jump_is_not_saved() {
        let mut app = test_app(vec![]);
        app.enter_group_form(None).unwrap();
        let form = app.group_form.as_mut().unwrap();
        form.name = "prod".into();
        form.default_proxy_jump = "-oProxyCommand=id".into();
        app.save_group_form().unwrap();
        assert!(app.group_form.is_some(), "the form stays open");
        assert!(app
            .group_notice
            .as_deref()
            .is_some_and(|n| n.contains("ProxyJump")));
        assert!(app
            .store()
            .list_groups()
            .unwrap()
            .iter()
            .all(|g| g.name != "prod"));
        app.group_form.as_mut().unwrap().default_proxy_jump = "bastion-eu".into();
        app.save_group_form().unwrap();
        assert!(app
            .store()
            .list_groups()
            .unwrap()
            .iter()
            .any(|g| g.name == "prod"));
    }

    /// Run `argv` (an ssh command line) for real, bounded and non-interactive.
    fn run_ssh(argv: &[String]) {
        let mut cmd = std::process::Command::new(&argv[0]);
        cmd.args([
            "-F",
            "/dev/null",
            "-o",
            "ConnectTimeout=2",
            "-o",
            "BatchMode=yes",
        ])
        .args(&argv[1..])
        .arg("true")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
        let _ = cmd.status();
    }

    /// OpenSSH runs the `-J` hop helper through a shell and does not quote the
    /// user: `a$IFS-oProxyCommand=touch\$IFS/x@h` becomes an injected option.
    /// No path that builds an ssh argv may let such a hop through.
    #[test]
    fn a_shell_injecting_hop_user_never_reaches_ssh_through_any_builder() {
        if !std::process::Command::new("ssh").arg("-V").output().is_ok() {
            crate::test_support::tool_missing("ssh");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("pwned");
        let user = format!("a$IFS-oProxyCommand=touch\\$IFS{}", marker.display());
        let hostile = format!("{user}@127.0.0.1:1");

        // Control: raw ssh with this -J does run it, so the test is not blind.
        run_ssh(&[
            "ssh".into(),
            "-J".into(),
            hostile.clone(),
            "t@127.0.0.1".into(),
        ]);
        if !marker.exists() {
            eprintln!("note: this ssh does not run the injection; refusal asserts still apply");
        }
        let _ = std::fs::remove_file(&marker);

        let mut app = crate::app::tests::test_app(vec![]);
        let group = app
            .store()
            .create_group(&crate::store::NewHostGroup {
                name: "g".into(),
                default_proxy_jump: Some(hostile.clone()),
                ..Default::default()
            })
            .unwrap();
        let mut own = NewHost::launcher("own", "10.0.0.1");
        own.proxy_jump = Some(hostile.clone());
        let mut grouped = NewHost::launcher("grouped", "10.0.0.2");
        grouped.group_id = Some(group.id);
        let mut hop = NewHost::launcher("hop", "10.0.0.3");
        hop.username = Some(user.clone());
        let mut via_hop = NewHost::launcher("via-hop", "10.0.0.4");
        via_hop.proxy_jump = Some("hop".into());
        let mut hosts = Vec::new();
        for nh in [own, grouped, hop, via_hop] {
            hosts.push(app.store().create_host(&nh).unwrap());
        }
        app.reload_hosts().unwrap();

        for h in [&hosts[0], &hosts[1], &hosts[3]] {
            // Connect, tunnel, broadcast, SFTP and push key all start here.
            let refused = match app.store().resolve_connection(h) {
                Err(_) => true,
                Ok(r) => {
                    if r.jump_error.is_none() {
                        run_ssh(&crate::app::resolved_session_argv(h, &r));
                    }
                    r.jump_error.is_some()
                }
            };
            assert!(refused, "{} was not refused", h.name);
            let entry = HostEntry::from_managed(h.clone());
            assert!(app.connect_host_entry(entry).is_ok());
            assert!(app.sessions.is_empty(), "{}: a session was spawned", h.name);
        }
        // The display fallback keeps the jump out of argv or flags it.
        for h in &hosts {
            let r = crate::store::ResolvedConnection::from_stored(h);
            if r.jump_error.is_none() {
                run_ssh(&crate::app::resolved_session_argv(h, &r));
            }
        }

        // The map and the hop editor: a hostile token is a skipped step, never
        // an ssh command; a stored hop with a hostile user is refused too.
        for hops in [
            vec![Some(hostile.clone())],
            vec![Some("hop".to_string())],
            vec![Some("own".to_string()), Some(hostile.clone())],
        ] {
            for step in app.test_steps(&hops, TestTarget::None) {
                if step.skip.is_none() {
                    run_ssh(&step.argv);
                }
            }
            let via = app.test_steps(&hops, TestTarget::Host(0));
            for step in via.iter().filter(|s| s.skip.is_none()) {
                run_ssh(&step.argv);
            }
        }
        assert!(!marker.exists(), "an injected -oProxyCommand ran");
    }

    #[test]
    fn skipped_steps_are_never_reported_as_all_ok() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut run = run_with(TestOwner::Editor, rx, Arc::new(AtomicBool::new(false)));
        run.done = true;
        run.states = vec![
            StepState::Skipped {
                reason: "invalid hop, not tested".into(),
            },
            StepState::Skipped {
                reason: "invalid hop, not tested".into(),
            },
        ];
        assert_eq!(run.headline(), "refused: invalid hop");
        run.states = vec![
            StepState::Ok { ms: 1 },
            StepState::Skipped {
                reason: PROXY_SKIP.into(),
            },
        ];
        assert_eq!(run.headline(), "1 of 2 steps tested, the rest not tested");
        run.states = vec![StepState::Ok { ms: 1 }, StepState::Ok { ms: 2 }];
        assert_eq!(run.headline(), "all 2 steps ok");
    }
}
