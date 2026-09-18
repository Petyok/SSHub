//! Behavioral tests for cross-profile transfer CLI surface.
//!
//! These exercise the pure plan/args layer (`sshub::cli::transfer`) against
//! the engine contract (`sshub::store::transfer`): confirmation is always
//! explicit, and `group transfer --with-hosts` toggles host expansion.

use sshub::cli::transfer::{needs_confirm, parse_transfer_args, render_plan_text};
use sshub::store::transfer::{PlannedItem, TransferPlan};

fn sample_plan() -> TransferPlan {
    TransferPlan {
        items: vec![
            PlannedItem {
                kind: "host",
                src_name: "web".to_string(),
                dest_name: "web".to_string(),
                renamed: false,
                blocked: None,
            },
            PlannedItem {
                kind: "host",
                src_name: "db".to_string(),
                dest_name: "db (copy 1)".to_string(),
                renamed: true,
                blocked: None,
            },
            PlannedItem {
                kind: "host",
                src_name: "live".to_string(),
                dest_name: "live".to_string(),
                renamed: false,
                blocked: Some("tunnel running".to_string()),
            },
        ],
        dest_profile: "other".to_string(),
    }
}

#[test]
fn transfer_cli_plan_confirm_requires_explicit_yes() {
    // A runnable plan must demand explicit confirmation (no silent apply).
    assert!(needs_confirm(&sample_plan()));
    // Even an empty plan demands it: the confirm gate is unconditional.
    let empty = TransferPlan {
        items: Vec::new(),
        dest_profile: "other".to_string(),
    };
    assert!(needs_confirm(&empty));
    // A plan with only blocked items demands it too: the gate does not depend
    // on runnability. NOTE: this pins the predicate only — the `cancelled` /
    // exit-1 behavior of `run_transfer` has no unit oracle (it needs live
    // profile stores + interactive stdin; production is frozen).
    let blocked_only = TransferPlan {
        items: vec![PlannedItem {
            kind: "host",
            src_name: "live".to_string(),
            dest_name: "live".to_string(),
            renamed: false,
            blocked: Some("tunnel running".to_string()),
        }],
        dest_profile: "other".to_string(),
    };
    assert!(needs_confirm(&blocked_only));
}

#[test]
fn transfer_cli_group_with_hosts_flag() {
    let with = parse_transfer_args(&[
        "--to".to_string(),
        "other".to_string(),
        "--with-hosts".to_string(),
    ])
    .expect("--with-hosts must parse");
    assert!(with.with_hosts);
    assert_eq!(with.dest_profile, "other");
    assert!(!with.move_mode);
    assert!(!with.auto_yes);

    let without = parse_transfer_args(&["--to".to_string(), "other".to_string()])
        .expect("minimal transfer args must parse");
    assert!(!without.with_hosts);
    // Full flag set: copy flips to move, prompt flips to auto-yes.
    let full = parse_transfer_args(&[
        "--to".to_string(),
        "other".to_string(),
        "--move".to_string(),
        "--yes".to_string(),
    ])
    .expect("full transfer args must parse");
    assert!(full.move_mode);
    assert!(full.auto_yes);
    assert!(!full.with_hosts);
    assert_eq!(full.dest_profile, "other");

    // Destination profile is required.
    assert!(parse_transfer_args(&["--with-hosts".to_string()]).is_err());
}

#[test]
fn transfer_cli_plan_render_marks_renames_and_blocks() {
    let text = render_plan_text(&sample_plan());
    // Golden text: the render is deterministic, so pin it byte-for-byte.
    let expected = "Transfer to profile 'other': 3 item(s)\n  host 'web' -> 'web'\n  host 'db' -> 'db (copy 1)' (renamed)\n  host 'live' blocked: tunnel running\n";
    assert_eq!(text, expected, "plan render golden mismatch:\n{text}");
}

/// Process-wide lock: the CLI transfer path resolves the installation from
/// `HOME`, so hermetic tests must mutate it without racing each other.
static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Point `HOME` at a temp dir (a temp installation with real profile
/// discovery) and restore it on drop. The `SSHUB_*` compat overrides stay
/// unset: they would bypass profiles entirely.
struct IsolatedHome {
    _lock: std::sync::MutexGuard<'static, ()>,
    temp: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
}

impl IsolatedHome {
    fn new() -> Self {
        let lock = HOME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());
        Self {
            _lock: lock,
            temp,
            previous,
        }
    }
}

impl Drop for IsolatedHome {
    fn drop(&mut self) {
        match &self.previous {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }
}

/// Two profiles (`src` active, `dest` target) under an isolated `HOME`, with
/// one labeled tunnel `t1` and one unlabeled tunnel on host `web` in `src`.
/// Returns the home guard (drop last), the source context, and the dest db path.
fn two_profile_ctx() -> (IsolatedHome, sshub::cli::CliContext, std::path::PathBuf) {
    use sshub::store::{LauncherStore, NewHost, NewTunnel, TunnelType};

    let home = IsolatedHome::new();
    let roots = sshub::profile::resolve_roots().unwrap();
    assert!(
        !roots.compat,
        "temp HOME must resolve to profile mode, got compat"
    );
    let mut state = sshub::profile::ProfileState::default();
    sshub::profile::create_profile(&roots, &mut state, "src").unwrap();
    sshub::profile::create_profile(&roots, &mut state, "dest").unwrap();
    state.save(&roots.data_root).unwrap();
    let ssh_config = home.temp.path().join("ssh_config");
    let src_record = state.by_name("src").unwrap().clone();
    let src_paths = sshub::profile::profile_paths(&roots, &src_record, ssh_config.clone());
    let dest_db =
        sshub::profile::profile_paths(&roots, &state.by_name("dest").unwrap().clone(), ssh_config)
            .launcher_db();

    let store = LauncherStore::open(src_paths.launcher_db()).unwrap();
    let host_id = store
        .create_host(&NewHost {
            name: "web".into(),
            address: "web.example.com".into(),
            ..NewHost::launcher("web", "")
        })
        .unwrap()
        .id;
    for tunnel in [
        NewTunnel {
            host_id: Some(host_id),
            tunnel_type: TunnelType::Local,
            local_port: 8080,
            remote_host: "localhost".into(),
            remote_port: 80,
            label: Some("t1".into()),
            auto_connect: false,
        },
        NewTunnel {
            host_id: Some(host_id),
            tunnel_type: TunnelType::Local,
            local_port: 9090,
            remote_host: "localhost".into(),
            remote_port: 90,
            label: None,
            auto_connect: false,
        },
    ] {
        store.create_tunnel(&tunnel).unwrap();
    }
    drop(store);

    let ctx = sshub::cli::CliContext::bootstrap_with(src_paths).unwrap();
    (home, ctx, dest_db)
}

fn dest_tunnels(dest_db: &std::path::Path) -> Vec<sshub::store::Tunnel> {
    sshub::store::LauncherStore::open(dest_db)
        .unwrap()
        .list_tunnels()
        .unwrap()
}

fn tunnel_args(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

#[test]
fn tunnel_transfer_arm_returns_inner_exit_code() {
    // Oracle: the `tunnel transfer` exit code itself — a transfer the engine
    // refuses (blocked host ⇒ empty runnable plan) must surface the inner
    // `1`, not the arm's old hardcoded `0`. No stdin is read on this path.
    let (_home, mut ctx, dest_db) = two_profile_ctx();

    // Fake a running tunnel: a pid file holding our own (live) pid marks the
    // host blocked through the same `tunnel_runtime_state` check production
    // uses, so the plan has zero runnable items.
    let pid_dir = sshub::tunnel::ensure_tunnel_pid_dir(ctx.profile.tunnel_base()).unwrap();
    let tunnel_id = ctx.store.list_tunnels().unwrap()[0].id;
    let pid_path = sshub::tunnel::tunnel_pid_path(&pid_dir, tunnel_id);
    sshub::tunnel::write_tunnel_pid(&pid_path, std::process::id()).unwrap();

    let code = sshub::cli::tunnel::run(
        &mut ctx,
        &tunnel_args(&["transfer", "t1", "--to", "dest", "--yes"]),
    )
    .unwrap();
    assert_eq!(
        code, 1,
        "refused transfer must exit 1 through the tunnel arm"
    );
    assert!(
        dest_tunnels(&dest_db).is_empty(),
        "refused transfer moves nothing"
    );

    // Unblock and transfer for real: the success code passes through too,
    // and the prepared destination receives the tunnel.
    sshub::tunnel::remove_tunnel_pid(&pid_path).unwrap();
    let code = sshub::cli::tunnel::run(
        &mut ctx,
        &tunnel_args(&["transfer", "t1", "--to", "dest", "--yes"]),
    )
    .unwrap();
    assert_eq!(
        code, 0,
        "successful transfer must exit 0 through the tunnel arm"
    );
    let tunnels = dest_tunnels(&dest_db);
    assert_eq!(tunnels.len(), 1);
    assert_eq!(tunnels[0].label.as_deref(), Some("t1"));
}

#[test]
fn tunnel_transfer_accepts_canonical_unlabeled_tokens() {
    // Oracle: the destination tunnel rows — the unlabeled tunnel must arrive
    // whether it is addressed by bare id or by its `:<port>` token.
    for token in ["1", ":9090", "tunnel#1"] {
        let (_home, mut ctx, dest_db) = two_profile_ctx();
        let unlabeled = ctx
            .store
            .list_tunnels()
            .unwrap()
            .into_iter()
            .find(|t| t.label.is_none())
            .expect("unlabeled tunnel");
        let token = match token {
            "1" => unlabeled.id.to_string(),
            ":9090" => ":9090".to_string(),
            _ => format!("tunnel#{}", unlabeled.id),
        };
        let code = sshub::cli::tunnel::run(
            &mut ctx,
            &tunnel_args(&["transfer", &token, "--to", "dest", "--yes"]),
        )
        .unwrap();
        assert_eq!(code, 0, "token '{token}' must transfer");
        let tunnels = dest_tunnels(&dest_db);
        assert_eq!(tunnels.len(), 1, "token '{token}' moves one tunnel");
        assert_eq!(tunnels[0].label, None);
        assert_eq!(tunnels[0].local_port, 9090);
    }
}
