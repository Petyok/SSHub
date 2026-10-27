use super::*;

/// Look up the stored credential for a host entry and decide whether it's
/// a host password (sent at `password:` prompts) or an identity passphrase
/// (sent at `Enter passphrase for …`). Returns the pending secret and a
/// human-readable diagnostic line for the SSH log.
///
/// `effective_identity` is the resolved identity (host-explicit, else the
/// nearest ancestor group's default): a stored passphrase on a group-level
/// identity must unlock it just like a host-level one.
pub fn resolve_pending_secret(
    entry: &HostEntry,
    effective_identity: Option<&crate::store::Identity>,
    password_store: &dyn crate::credentials::PasswordStore,
) -> (Option<crate::session::PendingSecret>, String) {
    let Some(managed) = entry.managed() else {
        return (
            None,
            "auth: legacy ssh_config host — no stored credential".into(),
        );
    };

    // ssh offers the key before it offers a password, so a passphrase for a
    // keyed identity outranks a remembered host password. The askpass channel
    // only answers a prompt of the matching class, so a `Password` offered at
    // `Enter passphrase for …` is dropped on the floor — v0.17.0 left users
    // typing a passphrase sshub already held. Lookup misses fall through to
    // the host branch; the identity branch below still reports them.
    let keyed_identity = effective_identity
        .or(managed.identity.as_ref())
        .filter(|identity| identity.private_key.is_some());
    if let Some(identity) = keyed_identity {
        let key = crate::credentials::identity_key(identity.id);
        if let Ok(Some(pw)) = password_store.get(&key) {
            return (
                Some(crate::session::PendingSecret::Passphrase(pw)),
                format!("auth: using stored passphrase ({key})"),
            );
        }
    }

    if let Some(spec) = managed.secret_lookup.as_deref() {
        return resolve_secret_lookup(spec, password_store);
    }

    // The remember-me modal saves the secret without touching the host row,
    // so a set flag cannot be required: presence of the secret itself
    // decides. The flag only tunes the diagnostic. Falls through to the
    // identity below when no host secret exists.
    let key = crate::credentials::host_key(managed.id);
    match password_store.get(&key) {
        Ok(Some(pw)) => {
            return (
                Some(crate::session::PendingSecret::Password(pw)),
                if managed.has_password {
                    format!("auth: using stored password ({key})")
                } else {
                    format!("auth: using stored password ({key}) though the row flag is unset")
                },
            );
        }
        Ok(None) if managed.has_password => {
            return (
                None,
                format!(
                    "auth: has_password=true but keyring entry {key} is empty — ssh will prompt"
                ),
            );
        }
        Err(e) if managed.has_password => {
            return (
                None,
                format!("auth: keyring lookup failed for {key}: {e:#} — ssh will prompt"),
            );
        }
        Ok(None) | Err(_) => {}
    }

    if let Some(identity) = effective_identity.or(managed.identity.as_ref()) {
        // Same remember-me gap as the host branch above: the modal saves an
        // identity secret without setting its flag.
        let key = crate::credentials::identity_key(identity.id);
        // A secret on an identity WITH a key unlocks that key (passphrase);
        // on a keyless identity it's a shared login password, letting many
        // hosts reuse one user+password credential.
        let has_key = identity.private_key.is_some();
        match password_store.get(&key) {
            Ok(Some(pw)) => {
                return (
                    Some(if has_key {
                        crate::session::PendingSecret::Passphrase(pw)
                    } else {
                        crate::session::PendingSecret::Password(pw)
                    }),
                    format!(
                        "auth: using stored {} ({key})",
                        if has_key { "passphrase" } else { "password" }
                    ),
                );
            }
            Ok(None) if identity.has_password => {
                return (
                    None,
                    format!(
                        "auth: identity has_password=true but keyring entry {key} is empty — ssh will prompt"
                    ),
                );
            }
            Err(e) if identity.has_password => {
                return (
                    None,
                    format!("auth: keyring lookup failed for {key}: {e:#} — ssh will prompt"),
                );
            }
            Ok(None) | Err(_) => {}
        }
    }

    (
        None,
        "auth: no stored credential — using agent / unlocked key / interactive prompt".into(),
    )
}

/// A host's Secret Service lookup (#142) is authoritative for its password:
/// a failure prompts instead of falling back to the SSHub-stored secret.
fn resolve_secret_lookup(
    spec: &str,
    password_store: &dyn crate::credentials::PasswordStore,
) -> (Option<crate::session::PendingSecret>, String) {
    match password_store.lookup(spec) {
        Ok(pw) => (
            Some(crate::session::PendingSecret::Password(pw)),
            format!("auth: using Secret Service entry {spec}"),
        ),
        Err(e) => (
            None,
            format!("auth: secret lookup {spec} failed: {e:#}; ssh will prompt"),
        ),
    }
}

/// Same as [`resolve_pending_secret`] but for a [`ManagedHost`] row (tunnels).
pub fn resolve_pending_secret_for_managed(
    managed: &crate::store::ManagedHost,
    password_store: &dyn crate::credentials::PasswordStore,
) -> (Option<crate::session::PendingSecret>, String) {
    if let Some(spec) = managed.secret_lookup.as_deref() {
        return resolve_secret_lookup(spec, password_store);
    }

    // Same remember-me gap as the session lookup above: secret presence
    // decides, the row flag only tunes the diagnostic.
    let key = crate::credentials::host_key(managed.id);
    match password_store.get(&key) {
        Ok(Some(pw)) => {
            return (
                Some(crate::session::PendingSecret::Password(pw)),
                format!("auth: using stored password ({key})"),
            );
        }
        Ok(None) if managed.has_password => {
            return (
                None,
                format!(
                    "auth: has_password=true but keyring entry {key} is empty — tunnel cannot prompt"
                ),
            );
        }
        Err(e) if managed.has_password => {
            return (
                None,
                format!("auth: keyring lookup failed for {key}: {e:#}"),
            );
        }
        Ok(None) | Err(_) => {}
    }

    if let Some(identity) = managed.identity.as_ref() {
        let key = crate::credentials::identity_key(identity.id);
        let has_key = identity.private_key.is_some();
        match password_store.get(&key) {
            Ok(Some(pw)) => {
                return (
                    Some(if has_key {
                        crate::session::PendingSecret::Passphrase(pw)
                    } else {
                        crate::session::PendingSecret::Password(pw)
                    }),
                    format!(
                        "auth: using stored {} ({key})",
                        if has_key { "passphrase" } else { "password" }
                    ),
                );
            }
            Ok(None) if identity.has_password => {
                return (
                    None,
                    format!(
                        "auth: identity has_password=true but keyring entry {key} is empty — tunnel cannot prompt"
                    ),
                );
            }
            Err(e) if identity.has_password => {
                return (
                    None,
                    format!("auth: keyring lookup failed for {key}: {e:#}"),
                );
            }
            Ok(None) | Err(_) => {}
        }
    }

    (
        None,
        "auth: no stored credential — tunnel uses agent / unlocked key only (BatchMode)".into(),
    )
}

/// Capture host metadata used by the embedded session header + connect
/// animation, from already-resolved connection values.
pub(crate) fn session_meta_for_resolved(
    m: &ManagedHost,
    resolved: &crate::store::ResolvedConnection,
) -> crate::session::SessionMeta {
    crate::session::SessionMeta {
        user: resolved
            .username
            .clone()
            .or_else(|| resolved.identity.as_ref().and_then(|i| i.username.clone())),
        address: Some(m.address.clone()),
        port: Some(resolved.port),
        identity: resolved
            .identity
            .as_ref()
            .and_then(|i| i.private_key.as_ref())
            .map(|p| p.to_string_lossy().into_owned()),
        proxy_jump: resolved.proxy_jump.clone(),
        host_id: Some(m.id),
    }
}

/// Capture host metadata used by the embedded session header + connect
/// animation.
pub(crate) fn session_meta_for_entry(entry: &HostEntry) -> crate::session::SessionMeta {
    match entry {
        HostEntry::Managed(m) => {
            session_meta_for_resolved(m, &crate::store::ResolvedConnection::from_stored(m))
        }
        HostEntry::Legacy { host, .. } => crate::session::SessionMeta {
            user: host.user.clone(),
            address: host.hostname.clone(),
            port: host.port,
            identity: host.identity_file.clone(),
            proxy_jump: host.proxy_jump.clone(),
            host_id: None,
        },
    }
}

/// Build the bare `mosh` argv for a host entry.
pub fn mosh_argv_for_entry(entry: &HostEntry) -> Vec<String> {
    match entry {
        HostEntry::Managed(m) => {
            let ssh_host = managed_to_ssh_host(m);
            if m.source == HostSource::SshConfig {
                crate::ssh::build_mosh_alias_argv(&ssh_host)
            } else {
                crate::ssh::build_mosh_argv(&ssh_host)
            }
        }
        HostEntry::Legacy { host, .. } => crate::ssh::build_mosh_alias_argv(host),
    }
}

/// CLI connect argv: optional verbose logging, accept-new when secret present.
pub fn prepare_cli_connect_argv(
    mut argv: Vec<String>,
    has_stored_secret: bool,
    verbose: bool,
) -> Vec<String> {
    match argv.first().map(String::as_str) {
        Some("ssh") => {
            if verbose {
                argv.insert(1, "-v".into());
            }
            if has_stored_secret {
                argv.insert(1, "-o".into());
                argv.insert(2, "StrictHostKeyChecking=accept-new".into());
            }
            argv
        }
        Some("mosh") if has_stored_secret => crate::ssh::inject_mosh_ssh_accept_new(argv),
        _ => argv,
    }
}

/// [`prepare_cli_connect_argv`] for an interactive `sshub connect` (and the argv
/// `sshub host resolve` reports for it): ssh also gets `-t` and the startup
/// command keeps the login shell, as in [`prepare_session_connect_argv`].
/// `sshub exec` stays on the plain variant: it drops the startup command and
/// picks `-T` or `-tt` itself.
pub fn prepare_cli_session_argv(
    argv: Vec<String>,
    has_stored_secret: bool,
    verbose: bool,
) -> Vec<String> {
    let mut argv = prepare_cli_connect_argv(argv, has_stored_secret, verbose);
    if argv.first().map(String::as_str) == Some("ssh") {
        argv.insert(1, "-t".into());
        keep_login_shell(&mut argv);
    }
    argv
}

/// Interactive SSH uses explicit host trust, at most three auth attempts, and
/// a remote terminal. ssh asks for a terminal on its own only for a login
/// shell, so a startup command (`remote_command`, or `RemoteCommand` in
/// ssh_config) ran without one: `sudo su root` printed no prompt and the
/// session sat on the connect screen (#171).
pub fn prepare_session_connect_argv(mut argv: Vec<String>, has_stored_secret: bool) -> Vec<String> {
    match argv.first().map(String::as_str) {
        Some("ssh") => {
            argv.splice(
                1..1,
                [
                    "-t",
                    "-o",
                    "StrictHostKeyChecking=ask",
                    "-o",
                    "NumberOfPasswordPrompts=3",
                ]
                .into_iter()
                .chain(crate::session::SSH_DEBUG_ARGS)
                .map(String::from),
            );
            keep_login_shell(&mut argv);
            argv
        }
        Some("mosh") if has_stored_secret => crate::ssh::inject_mosh_ssh_accept_new(argv),
        _ => argv,
    }
}

/// Share this machine's network with the host (#174): add the reverse
/// dynamic forward and its destination limit to an ssh argv, ahead of the
/// destination. `false` (argv untouched) for anything but ssh, mosh has no
/// reverse forward.
pub fn add_share_net_args(argv: &mut Vec<String>, permit_remote_open: &str) -> bool {
    if argv.first().map(String::as_str) != Some("ssh") {
        return false;
    }
    argv.splice(
        1..1,
        crate::session::share_net::ssh_args(permit_remote_open),
    );
    true
}

/// A startup command runs at login and the session goes on in the login
/// shell (#171): `cd /srv` leaves a shell in /srv, `sudo su` a root shell
/// that drops back to yours on exit. ssh runs a remote command instead of the
/// shell, so the shell is started after it. A newline rather than `;` ends the
/// command, so a trailing `&` or `# comment` still parses. A command that
/// starts with `exec` replaces the shell that runs the rest, and the session
/// ends with it.
///
/// Only the trailing `-- <command>` pair `build_ssh_argv` adds for a stored
/// startup command is touched; an ssh_config `RemoteCommand` stays ssh's.
fn keep_login_shell(argv: &mut [String]) {
    if let [.., sep, command] = argv {
        if sep == "--" {
            command.push_str("\nexec \"$SHELL\" -l");
        }
    }
}

/// Build session argv (`ssh` or `mosh`) from the *effective* transport
/// (group inheritance resolved): every connect path must use this, not
/// [`session_argv_for_entry`], so a group-level transport default takes effect.
pub fn resolved_session_argv(
    m: &ManagedHost,
    resolved: &crate::store::ResolvedConnection,
) -> Vec<String> {
    let ssh_host = resolved_to_ssh_host(m, resolved);
    let alias = m.source == HostSource::SshConfig;
    match resolved.transport {
        crate::session_transport::SessionTransport::Ssh => {
            if alias {
                crate::ssh::build_ssh_alias_argv(&ssh_host)
            } else {
                crate::ssh::build_ssh_argv(&ssh_host)
            }
        }
        crate::session_transport::SessionTransport::Mosh => {
            if alias {
                crate::ssh::build_mosh_alias_argv(&ssh_host)
            } else {
                crate::ssh::build_mosh_argv(&ssh_host)
            }
        }
    }
}

/// Build session argv (`ssh` or `mosh`) from the stored transport setting.
/// Prefer [`resolved_session_argv`] on connect paths: this stored-only
/// variant misses a group-level transport default.
pub fn session_argv_for_entry(entry: &HostEntry) -> Vec<String> {
    match entry.session_transport() {
        crate::session_transport::SessionTransport::Ssh => ssh_argv_for_entry(entry),
        crate::session_transport::SessionTransport::Mosh => mosh_argv_for_entry(entry),
    }
}

/// Build the bare `ssh` argv for a host entry (no env / askpass prefix).
///
/// - Launcher-managed hosts: full options via `build_ssh_argv` so we don't
///   require an `~/.ssh/config` alias.
/// - SSH-config-sourced hosts: alias-only argv via `build_ssh_alias_argv` so
///   ssh inherits all options from the user's config.
/// - Legacy entries (ssh_config only, not in launcher DB): alias-only argv.
pub fn ssh_argv_for_entry(entry: &HostEntry) -> Vec<String> {
    match entry {
        HostEntry::Managed(m) => {
            let ssh_host = managed_to_ssh_host(m);
            if m.source == HostSource::SshConfig {
                crate::ssh::build_ssh_alias_argv(&ssh_host)
            } else {
                crate::ssh::build_ssh_argv(&ssh_host)
            }
        }
        HostEntry::Legacy { host, .. } => crate::ssh::build_ssh_alias_argv(host),
    }
}

pub(crate) fn managed_to_ssh_host(m: &ManagedHost) -> SshHost {
    resolved_to_ssh_host(m, &crate::store::ResolvedConnection::from_stored(m))
}

/// Build the [`SshHost`] ssh/mosh argv is derived from, using already-resolved
/// connection values (host-explicit → group chain → global).
pub fn resolved_to_ssh_host(
    m: &ManagedHost,
    resolved: &crate::store::ResolvedConnection,
) -> SshHost {
    let mut host = SshHost::new(&m.name);
    host.hostname = Some(m.address.clone());
    host.port = Some(resolved.port);
    host.user = resolved
        .username
        .clone()
        .or_else(|| resolved.identity.as_ref().and_then(|i| i.username.clone()));
    host.identity_file = resolved
        .identity
        .as_ref()
        .and_then(|i| i.private_key.as_ref())
        .map(|p| p.to_string_lossy().into_owned());
    host.certificate_file = resolved
        .identity
        .as_ref()
        .and_then(|i| i.certificate.as_ref())
        .map(|p| p.to_string_lossy().into_owned());
    host.proxy_jump = resolved.jump_argv.clone();
    host.forward_agent = Some(resolved.forward_agent);
    host.remote_command = m.remote_command.clone();
    host
}

/// Build the [`SshHost`] the native (libssh2) SFTP transport connects with.
///
/// Interactive SSH sessions hand the alias to the system `ssh`, which applies
/// `~/.ssh/config` itself; libssh2 cannot. Launcher rows imported from
/// ssh_config keep only address/port/proxy_jump (`build_import_row`), so for
/// those resolve the alias live through the same `ssh -G` machinery the
/// import used and let the resolved config fill what the store does not
/// manage. SSHub-managed fields always win when present.
pub(crate) fn sftp_ssh_host(
    resolver: &dyn HostResolver,
    store: &crate::store::LauncherStore,
    entry: &HostEntry,
) -> SshHost {
    let mut host = match entry {
        // Resolve group defaults first so SFTP honours the same effective
        // port/identity as a connect. A resolution failure falls back to the
        // stored values rather than refusing the session.
        HostEntry::Managed(m) => match store.resolve_connection(m) {
            Ok(resolved) => resolved_to_ssh_host(m, &resolved),
            Err(_) => {
                // Not resolvable (a ProxyJump that is not valid hops): keep a
                // jump on the host so the "no SFTP over ProxyJump" refusal
                // fires, rather than a direct session past the bastion.
                let mut h = entry.ssh_host();
                h.proxy_jump = Some(m.proxy_jump.clone().unwrap_or_else(|| "(invalid)".into()));
                h
            }
        },
        HostEntry::Legacy { .. } => entry.ssh_host(),
    };
    if let HostEntry::Managed(m) = entry {
        if m.source == HostSource::SshConfig {
            if let Ok(resolved) = resolver.resolve_host(&m.name) {
                host.user = host.user.or(resolved.user);
                host.identity_file = host.identity_file.or(resolved.identity_file);
                host.certificate_file = host.certificate_file.or(resolved.certificate_file);
            }
        }
    }
    host
}

pub(crate) fn optional_path(raw: &str) -> Option<std::path::PathBuf> {
    optional_field(raw).map(std::path::PathBuf::from)
}

/// Copy `secret` to the clipboard and return the notice to show. `what` names it
/// ("password", "passphrase"); the value itself never reaches the message, the
/// audit log or any diagnostic.
pub(crate) fn copy_secret_notice(secret: &str, what: &str) -> String {
    if secret.is_empty() {
        return format!("no {what} stored for this entry");
    }
    match write_osc52(secret) {
        Ok(()) => format!("{what} copied to the clipboard"),
        Err(e) => format!("could not copy the {what}: {e}"),
    }
}

/// Put `text` on the host clipboard via OSC 52. Modern terminals (kitty /
/// iTerm2 / wezterm / Alacritty / foot) interpret the sequence as "put this
/// base64-encoded payload on the system clipboard"; it is invisible to the
/// alternate-screen UI because the host terminal consumes it before it ever
/// lands on a buffer cell. Framing lives in [`crate::osc52`], shared with the
/// session's PTY clipboard relay.
pub(crate) fn write_osc52(text: &str) -> std::io::Result<()> {
    crate::osc52::write_text(text)
}

/// Expand a leading `~` (or `~/`) in a path to the user's home directory.
pub(crate) fn shellexpand_home(path: &str) -> std::path::PathBuf {
    if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(home);
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(home).join(rest);
        }
    }
    std::path::PathBuf::from(path)
}

/// Render `path` for display with the user's home directory collapsed to `~`.
///
/// The inverse of [`shellexpand_home`]. Local paths reach the screen in the SFTP
/// browser, and `/home/<you>/…` is both longer than the pane and nobody else's
/// business — a recording of the browser used to carry the real username straight
/// into the README.
pub(crate) fn contract_home(path: &std::path::Path) -> String {
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => contract_prefix(path, std::path::Path::new(&home)),
        _ => path.display().to_string(),
    }
}

/// [`contract_home`] with the home directory passed in, so it can be tested
/// without touching a process-wide environment variable other tests share.
fn contract_prefix(path: &std::path::Path, home: &std::path::Path) -> String {
    if path == home {
        return "~".to_string();
    }
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

pub(crate) fn os_icon_from_index(index: usize) -> Option<String> {
    match OS_ICON_OPTIONS.get(index) {
        Some(&"(none)") | None => None,
        Some(s) => Some((*s).to_string()),
    }
}

pub(crate) fn os_icon_index_from_option(icon: &Option<String>) -> usize {
    icon.as_deref()
        .and_then(|name| OS_ICON_OPTIONS.iter().position(|opt| *opt == name))
        .unwrap_or(0)
}

pub(crate) fn parse_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn sort_host_indices(hosts: &[HostEntry], indices: &mut [usize], mode: SortMode) {
    indices.sort_by(|&a, &b| compare_hosts(&hosts[a], &hosts[b], mode));
}

pub(crate) fn compare_hosts(a: &HostEntry, b: &HostEntry, mode: SortMode) -> std::cmp::Ordering {
    match mode {
        SortMode::Label => label_cmp(a, b),
        SortMode::LastConnected => match (b.last_connected(), a.last_connected()) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => label_cmp(a, b),
        },
        SortMode::FavoriteFirst => match (a.favorite(), b.favorite()) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => label_cmp(a, b),
        },
        SortMode::GroupThenLabel => group_sort_key(a)
            .cmp(&group_sort_key(b))
            .then_with(|| label_cmp(a, b)),
        SortMode::Manual => a
            .sort_order()
            .cmp(&b.sort_order())
            .then_with(|| a.name().cmp(b.name())),
    }
}

pub(crate) fn label_cmp(a: &HostEntry, b: &HostEntry) -> std::cmp::Ordering {
    a.display_name()
        .to_lowercase()
        .cmp(&b.display_name().to_lowercase())
}

pub(crate) fn group_sort_key(entry: &HostEntry) -> String {
    match entry.managed().and_then(|m| m.group.as_ref()) {
        Some(g) => format!("{:08}_{}", g.sort_order, g.name.to_lowercase()),
        None => format!("z_{UNGROUPED_LABEL}"),
    }
}

pub(crate) fn build_group_sections(
    hosts: &[HostEntry],
    groups: &[HostGroup],
    filtered: &[usize],
) -> Vec<HostGroupSection> {
    let mut sections = Vec::new();

    // Walk the group forest depth-first: each group is followed by its
    // children (in list order), so `sections` reads top-to-bottom exactly as it
    // renders. `depth` drives indentation and subtree collapse. A `visiting`
    // guard defends against a malformed parent cycle in the data.
    let mut visiting = std::collections::HashSet::new();
    build_group_subtree(
        hosts,
        groups,
        filtered,
        None,
        0,
        &mut visiting,
        &mut sections,
    );

    let ungrouped: Vec<usize> = filtered
        .iter()
        .copied()
        .filter(|&idx| hosts[idx].group_ids().is_empty())
        .collect();
    if !ungrouped.is_empty() {
        sections.push(HostGroupSection {
            group: None,
            label: UNGROUPED_LABEL.to_string(),
            host_indices: ungrouped,
            collapsed: false,
            depth: 0,
        });
    }

    sections
}

/// For each section (in DFS order), whether its subtree — the section itself
/// plus its contiguous descendants (depth strictly greater, until depth returns
/// to `<=`) — contains any hosts. Used to keep empty ancestors of a matching
/// nested group while filtering.
pub(crate) fn subtree_has_hosts(sections: &[HostGroupSection]) -> Vec<bool> {
    let n = sections.len();
    let mut out = vec![false; n];
    for i in 0..n {
        if !sections[i].host_indices.is_empty() {
            out[i] = true;
            continue;
        }
        let depth = sections[i].depth;
        let mut j = i + 1;
        while j < n && sections[j].depth > depth {
            if !sections[j].host_indices.is_empty() {
                out[i] = true;
                break;
            }
            j += 1;
        }
    }
    out
}

/// Append the sections for every group whose parent is `parent_id`, then recurse
/// into their children. `groups` is already ordered by (sort_order, name).
fn build_group_subtree(
    hosts: &[HostEntry],
    groups: &[HostGroup],
    filtered: &[usize],
    parent_id: Option<i64>,
    depth: usize,
    visiting: &mut std::collections::HashSet<i64>,
    out: &mut Vec<HostGroupSection>,
) {
    for group in groups.iter().filter(|g| g.parent_id == parent_id) {
        if !visiting.insert(group.id) {
            continue; // cycle guard: already on the current path
        }
        let host_indices: Vec<usize> = filtered
            .iter()
            .copied()
            .filter(|&idx| hosts[idx].group_ids().contains(&group.id))
            .collect();
        // The reserved Favorites group is auto-created and always present; only
        // surface it once it actually has members (an empty section is noise).
        if group.reserved && host_indices.is_empty() {
            visiting.remove(&group.id);
            continue;
        }
        out.push(HostGroupSection {
            group: Some(group.clone()),
            label: group.name.clone(),
            host_indices,
            collapsed: false,
            depth,
        });
        build_group_subtree(
            hosts,
            groups,
            filtered,
            Some(group.id),
            depth + 1,
            visiting,
            out,
        );
        visiting.remove(&group.id);
    }
}

/// Parse a keybinding spec like `"Ctrl+S"`, `"F2"`, `"Alt+Enter"` into a
/// (code, modifiers) pair. Returns `None` for unrecognised specs.
pub(crate) fn parse_keyspec(spec: &str) -> Option<(KeyCode, KeyModifiers)> {
    let parts: Vec<&str> = spec.split('+').map(|p| p.trim()).collect();
    let (key_part, mod_parts) = parts.split_last()?;
    let mut mods = KeyModifiers::empty();
    for m in mod_parts {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
            "alt" | "option" => mods |= KeyModifiers::ALT,
            "shift" => mods |= KeyModifiers::SHIFT,
            _ => return None,
        }
    }
    let key = key_part.trim();
    if key.is_empty() {
        return None;
    }
    let code = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        "esc" | "escape" => KeyCode::Esc,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "backtab" => KeyCode::BackTab,
        lower => {
            // Function key "F1".."F12"?
            if let Some(n) = lower
                .strip_prefix('f')
                .filter(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
                .and_then(|r| r.parse::<u8>().ok())
            {
                KeyCode::F(n)
            } else if lower.chars().count() == 1 {
                // A bare uppercase letter with no explicit modifier (e.g. "Y")
                // means shift+letter; without this it parsed identically to "y"
                // and never matched a real shifted keypress on terminals that
                // report the SHIFT modifier. Guard on `mod_parts.is_empty()` so
                // conventionally-capitalised combos like "Ctrl+S" stay shift-free.
                let orig = key.chars().next().unwrap();
                if orig.is_ascii_uppercase() && mod_parts.is_empty() {
                    mods |= KeyModifiers::SHIFT;
                }
                KeyCode::Char(lower.chars().next().unwrap())
            } else {
                return None;
            }
        }
    };
    Some((code, mods))
}

/// Serialize an incoming key event into a spec string (inverse of
/// [`parse_keyspec`]) for capturing a binding in the UI. Returns `None` for
/// keys that can't be a binding (bare modifiers, unsupported codes).
pub(crate) fn keyevent_to_spec(key: &KeyEvent) -> Option<String> {
    let base = match key.code {
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::BackTab => "BackTab".to_string(),
        KeyCode::F(n) => format!("F{n}"),
        // A *bare* letter (no modifier) must serialize lowercase: a bare
        // uppercase spec means shift+letter (see parse_keyspec), so emitting
        // "G" for an unshifted 'g' would make the captured binding parse back
        // as shift and never match. With a modifier present the letter is
        // uppercased for the conventional display ("Ctrl+S"), which parses
        // unambiguously since the explicit modifier suppresses the shift rule.
        KeyCode::Char(c) if key.modifiers.is_empty() => c.to_ascii_lowercase().to_string(),
        KeyCode::Char(c) => c.to_ascii_uppercase().to_string(),
        _ => return None,
    };
    let mut out = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        out.push_str("Ctrl+");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        out.push_str("Alt+");
    }
    // Shift is only meaningful for keys that aren't already shifted into a
    // distinct char (e.g. Shift+H stays "Shift+H"; '?' has no Shift prefix).
    if key.modifiers.contains(KeyModifiers::SHIFT)
        && !matches!(key.code, KeyCode::Char(c) if !c.is_ascii_alphabetic())
    {
        out.push_str("Shift+");
    }
    out.push_str(&base);
    Some(out)
}

/// Match a parsed spec against an incoming event, comparing char keys
/// case-insensitively (so `Ctrl+S` matches whatever case crossterm reports).
pub(crate) fn keyspec_matches(code: KeyCode, mods: KeyModifiers, key: &KeyEvent) -> bool {
    let code_eq = match (code, key.code) {
        (KeyCode::Char(a), KeyCode::Char(b)) => a.eq_ignore_ascii_case(&b),
        (a, b) => a == b,
    };
    code_eq && key.modifiers == mods
}

pub(crate) fn optional_field(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_prefix_collapses_only_paths_under_that_home() {
        let home = std::path::Path::new("/home/someone");
        assert_eq!(contract_prefix(home, home), "~");
        assert_eq!(
            contract_prefix(&home.join("work/notes.md"), home),
            "~/work/notes.md"
        );
        // Sharing a prefix is not being under it.
        let sibling = std::path::Path::new("/home/someone-else/x");
        assert_eq!(contract_prefix(sibling, home), "/home/someone-else/x");
        // Anything outside home is left alone, remote-looking paths included.
        assert_eq!(
            contract_prefix(std::path::Path::new("/srv/www"), home),
            "/srv/www"
        );
    }
}

#[cfg(test)]
mod sftp_ssh_host_tests {
    use super::*;
    use crate::metadata::HostMetadata;
    use crate::store::{LauncherStore, NewHost, SshConfigHostImport};

    /// A resolver standing in for `ssh -G`: returns one fixed host, or fails.
    struct StubResolver {
        result: Option<SshHost>,
        fail: bool,
    }

    impl HostResolver for StubResolver {
        fn list_hosts(&self) -> Result<Vec<String>> {
            Ok(vec![])
        }

        fn resolve_host(&self, name: &str) -> Result<SshHost> {
            if self.fail {
                anyhow::bail!("no ssh_config entry for {name}");
            }
            Ok(self.result.clone().unwrap())
        }
    }

    fn config_resolved(name: &str) -> SshHost {
        let mut host = SshHost::new(name);
        host.user = Some("deploy".into());
        host.identity_file = Some("/home/u/.ssh/id_deploy".into());
        host.certificate_file = Some("/home/u/.ssh/id_deploy-cert.pub".into());
        // ssh -G echoes a default port; the launcher row must stay authoritative.
        host.port = Some(2222);
        host
    }

    fn ssh_config_entry(store: &LauncherStore) -> HostEntry {
        store
            .upsert_ssh_config_host(&SshConfigHostImport {
                name: "myserver".into(),
                address: "example.com".into(),
                port: 22,
                proxy_jump: None,
                forward_agent: false,
                remote_command: None,
                ssh_config_hash: "h1".into(),
                tags: vec![],
                notes: None,
                environment: None,
                favorite: false,
                last_connected: None,
                session_logging: crate::session_log::SessionLoggingOverride::Inherit,
                ping: crate::session_log::SessionLoggingOverride::Inherit,
                transport: crate::session_transport::SessionTransport::Ssh,
            })
            .unwrap();
        let row = &store
            .list_hosts_filtered(Some(HostSource::SshConfig))
            .unwrap()[0];
        HostEntry::from_managed(row.clone())
    }

    #[test]
    fn imported_host_fills_credentials_from_ssh_config() {
        let store = LauncherStore::open_in_memory().unwrap();
        let entry = ssh_config_entry(&store);
        let resolver = StubResolver {
            result: Some(config_resolved("myserver")),
            fail: false,
        };

        let host = sftp_ssh_host(&resolver, &store, &entry);
        assert_eq!(host.user.as_deref(), Some("deploy"));
        assert_eq!(
            host.identity_file.as_deref(),
            Some("/home/u/.ssh/id_deploy")
        );
        assert_eq!(
            host.certificate_file.as_deref(),
            Some("/home/u/.ssh/id_deploy-cert.pub")
        );
        // Address/port stay owned by the launcher row.
        assert_eq!(host.hostname.as_deref(), Some("example.com"));
        assert_eq!(host.port, Some(22));
    }

    #[test]
    fn managed_username_wins_over_ssh_config() {
        let store = LauncherStore::open_in_memory().unwrap();
        let mut entry = ssh_config_entry(&store);
        {
            let m = match &mut entry {
                HostEntry::Managed(m) => m,
                HostEntry::Legacy { .. } => unreachable!(),
            };
            m.username = Some("root".into());
        }
        let resolver = StubResolver {
            result: Some(config_resolved("myserver")),
            fail: false,
        };

        let host = sftp_ssh_host(&resolver, &store, &entry);
        assert_eq!(host.user.as_deref(), Some("root"));
        // The key the config carries is still filled in — the row had none.
        assert_eq!(
            host.identity_file.as_deref(),
            Some("/home/u/.ssh/id_deploy")
        );
    }

    #[test]
    fn launcher_hosts_never_consult_the_resolver() {
        let store = LauncherStore::open_in_memory().unwrap();
        store
            .create_host(&NewHost::launcher("selfhosted", "10.0.0.9"))
            .unwrap();
        let row = &store
            .list_hosts_filtered(Some(HostSource::Launcher))
            .unwrap()[0];
        let entry = HostEntry::from_managed(row.clone());
        let resolver = StubResolver {
            result: Some(config_resolved("myserver")),
            fail: false,
        };

        let host = sftp_ssh_host(&resolver, &store, &entry);
        // No ssh_config bleed into fully managed hosts.
        assert_eq!(host.user, None);
        assert_eq!(host.identity_file, None);
    }

    #[test]
    fn legacy_entries_pass_through_and_resolution_failure_degrades_gracefully() {
        let legacy = HostEntry::Legacy {
            host: config_resolved("legacy"),
            meta: HostMetadata::new("legacy"),
        };
        let failing = StubResolver {
            result: None,
            fail: true,
        };
        let store = LauncherStore::open_in_memory().unwrap();

        // Legacy hosts already carry resolved credentials; a failing resolver
        // must not damage them (the overlay never runs for Legacy).
        let host = sftp_ssh_host(&failing, &store, &legacy);
        assert_eq!(
            host.identity_file.as_deref(),
            Some("/home/u/.ssh/id_deploy")
        );

        // An imported host with an unreadable config keeps its old behaviour:
        // no creds, no panic.
        let entry = ssh_config_entry(&store);
        let host = sftp_ssh_host(&failing, &store, &entry);
        assert_eq!(host.user, None);
        assert_eq!(host.identity_file, None);
    }
}

/// A jump hop that is a saved host: what ssh will print when it asks that host
/// for a password, and how to find the host's own stored secret.
pub struct HopSpec {
    managed: crate::store::ManagedHost,
    identity: Option<crate::store::Identity>,
    user: Option<String>,
    /// The host as the jump list names it (see [`crate::session::hop_auth::HopCred::via`]).
    via: String,
}

impl HopSpec {
    fn name(&self) -> &str {
        &self.managed.name
    }

    /// The stored secret of `class` for this hop, fetched now. A password is the
    /// hop host's own (its Secret Service lookup, or `host:<id>`); a passphrase
    /// belongs to the identity whose key the hop would use.
    pub fn fetch(
        &self,
        class: crate::session::auth_prompt::PromptClass,
        passwords: &dyn crate::credentials::PasswordStore,
    ) -> Option<crate::session::PendingSecret> {
        use crate::session::auth_prompt::PromptClass;
        use crate::session::PendingSecret;
        match class {
            PromptClass::Password => {
                match resolve_pending_secret_for_managed(&self.managed, passwords).0 {
                    Some(PendingSecret::Password(pw)) => Some(PendingSecret::Password(pw)),
                    _ => None,
                }
            }
            PromptClass::Passphrase => {
                let identity = self.identity.as_ref().filter(|i| i.private_key.is_some())?;
                let key = crate::credentials::identity_key(identity.id);
                passwords
                    .get(&key)
                    .ok()
                    .flatten()
                    .map(PendingSecret::Passphrase)
            }
            PromptClass::HostKey | PromptClass::Generic => None,
        }
    }

    fn key_path(&self) -> Option<String> {
        self.identity
            .as_ref()
            .and_then(|i| i.private_key.as_ref())
            .map(|p| p.to_string_lossy().into_owned())
    }

    /// The same, as the lazy credential an interactive session keeps.
    pub fn lazy(
        self,
        passwords: std::sync::Arc<dyn crate::credentials::PasswordStore>,
    ) -> crate::session::hop_auth::HopCred {
        let cred = crate::session::hop_auth::HopCred::new(
            self.name().to_string(),
            self.user.clone(),
            self.managed.address.clone(),
            self.via.clone(),
            Some(self.managed.id),
            self.identity.as_ref().map(|i| i.id),
            self.key_path(),
            {
                let spec = std::sync::Arc::new(self);
                std::sync::Arc::new(move |class| spec.fetch(class, passwords.as_ref()))
            },
        );
        cred
    }

    /// Where this hop's secrets live, for ssh children that read a staged
    /// file: references only, and only for a class that has a stored secret
    /// right now (checked, never kept). The file holds no secret; the helper
    /// reads the store again when this hop's prompt arrives.
    pub fn secrets(
        &self,
        passwords: &dyn crate::credentials::PasswordStore,
    ) -> Vec<crate::session::hop_auth::HopSecret> {
        let store = passwords.locator();
        use crate::credentials::{host_key, identity_key, SecretRef};
        use crate::session::auth_prompt::PromptClass;
        let password = match self.managed.secret_lookup.as_deref() {
            // Authoritative: a failed lookup prompts, no fallback to SSHub's.
            Some(spec) => vec![SecretRef::Lookup(spec.to_string())],
            None => {
                let mut refs = vec![SecretRef::Key(host_key(self.managed.id))];
                // A keyless identity's secret is a shared login password.
                if let Some(identity) = self.identity.as_ref().filter(|i| i.private_key.is_none()) {
                    refs.push(SecretRef::Key(identity_key(identity.id)));
                }
                refs
            }
        };
        let passphrase: Vec<SecretRef> = self
            .identity
            .as_ref()
            .filter(|i| i.private_key.is_some())
            .map(|i| SecretRef::Key(identity_key(i.id)))
            .into_iter()
            .collect();
        [
            (PromptClass::Password, password),
            (PromptClass::Passphrase, passphrase),
        ]
        .into_iter()
        .filter(|(_, refs)| !refs.is_empty())
        .map(|(class, refs)| crate::session::hop_auth::HopSecret {
            user: self.user.clone(),
            host: self.managed.address.clone(),
            via: self.via.clone(),
            key_path: self.key_path(),
            class,
            refs,
            store: store.clone(),
        })
        // Presence only: the value is read to test it and dropped; a hop with
        // nothing stored (key auth, store error) stays unstaged, so callers'
        // `hops.is_empty()` still means "no hop secret to answer".
        .filter(|hop| {
            hop.refs
                .iter()
                .find_map(|r| r.read(passwords))
                .is_some_and(|v| !v.is_empty() && !v.contains(['\n', '\r']))
        })
        .collect()
    }
}

/// The hops of `jump` (a stored ProxyJump: names, `user@name`, `user@host`)
/// that are saved hosts. Anything else, such as a raw `user@address` hop, has
/// no saved credential and is left out.
pub fn hop_specs(
    store: &crate::store::LauncherStore,
    hosts: &[HostEntry],
    jump: &str,
) -> Vec<HopSpec> {
    let by_name = |n: &str| {
        hosts
            .iter()
            .find_map(|h| h.managed().filter(|m| m.name == n))
    };
    crate::map::hops::split(jump)
        .iter()
        .filter_map(|token| {
            let raw = crate::map::parse_raw_hop(token);
            // What ssh's own process for this hop is given as its host: the
            // host part of a raw `[user@]host[:port]`, else the saved name.
            let via = raw
                .as_ref()
                .map_or_else(|| token.clone(), |r| r.host.clone());
            // By name, or by the host part of the token, or (written as an
            // address) by the one saved host that has that address.
            let managed = by_name(token)
                .or_else(|| {
                    raw.as_ref()
                        .filter(|r| !r.ipv6)
                        .and_then(|r| by_name(&r.host))
                })
                .or_else(|| {
                    let addr = raw.as_ref()?.host.as_str();
                    let mut same = hosts
                        .iter()
                        .filter_map(|h| h.managed())
                        .filter(|m| m.address.eq_ignore_ascii_case(addr));
                    let one = same.next()?;
                    same.next().is_none().then_some(one)
                })?;
            let fields = store.resolve_fields(managed).ok()?;
            let user = raw
                .and_then(|r| r.user)
                .or(fields.username)
                .or_else(|| fields.identity.as_ref().and_then(|i| i.username.clone()));
            Some(HopSpec {
                managed: managed.clone(),
                identity: fields.identity,
                user,
                via,
            })
        })
        .collect()
}

/// The jump hosts' secret references for `jump`, for ssh children that read
/// them from a staged file (tunnels, exec, broadcast, key push). Nothing is
/// read from the credential store here: the helper does that when a hop asks.
pub fn hop_secrets(
    store: &crate::store::LauncherStore,
    hosts: &[HostEntry],
    passwords: &dyn crate::credentials::PasswordStore,
    jump: Option<&str>,
) -> Vec<crate::session::hop_auth::HopSecret> {
    jump.map(|j| {
        hop_specs(store, hosts, j)
            .iter()
            .flat_map(|spec| spec.secrets(passwords))
            .collect()
    })
    .unwrap_or_default()
}
