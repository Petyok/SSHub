//! Local terminal compatibility probes; no host configuration or credentials used.
use sshub::session::parser::ParserState;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

struct NoHosts;
impl sshub::ssh::HostResolver for NoHosts {
    fn list_hosts(&self) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }
    fn resolve_host(&self, name: &str) -> anyhow::Result<sshub::ssh::SshHost> {
        Ok(sshub::ssh::SshHost::new(name))
    }
}

fn benchmark() {
    struct Ignore;
    impl vte::Perform for Ignore {}
    let data = vec![b'x'; 8 * 1024 * 1024];
    let mut old = vte::Parser::new();
    let start = Instant::now();
    for byte in &data {
        old.advance(&mut Ignore, std::slice::from_ref(byte));
    }
    std::hint::black_box(&old);
    println!(
        "observer_bytewise_8mib_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    let mut bulk = vte::Parser::new();
    let start = Instant::now();
    let consumed = bulk.advance_until_terminated(&mut Ignore, &data);
    assert_eq!(consumed, data.len());
    std::hint::black_box(&bulk);
    println!(
        "observer_bulk_8mib_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    let repeat = b"X\x1b[65535b".repeat(4096 / 9);
    for (name, data) in [("plain", vec![b'x'; 4096]), ("rep", repeat)] {
        let mut parser = ParserState::new(24, 80);
        let start = Instant::now();
        parser.process(&data);
        std::hint::black_box(parser.screen());
        println!(
            "{name}_4kib_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
}

fn btop_smoke(output: &str) -> anyhow::Result<()> {
    use ratatui::{backend::TestBackend, Terminal};
    use sshub::{
        app::{App, AppDeps, AppMode},
        session::{Session, SessionConfig, SessionPhase},
    };
    let config = tempfile::tempdir()?;
    let session_config = SessionConfig {
        argv: vec![
            "env".into(),
            format!("XDG_CONFIG_HOME={}", config.path().display()),
            "btop".into(),
            "--force-utf".into(),
            "--no-tty".into(),
            "--update".into(),
            "100".into(),
        ],
        display_name: "btop local smoke".into(),
        meta: Default::default(),
        pending_secret: None,
        key_push_identity: None,
        host_name: "local-test".into(),
    };
    let mut session = Session::spawn_with_merged_stderr(session_config, 42, 140, None)?;
    session.phase = SessionPhase::Running {
        started_at: Instant::now(),
    };
    let mut app = App::new_with_deps(
        Default::default(),
        AppDeps {
            resolver: Box::new(NoHosts),
            metadata: Arc::new(sshub::metadata::MetadataDb::default()),
            store: Arc::new(sshub::store::LauncherStore::open_in_memory()?),
            password_store: Box::new(sshub::credentials::NoopPasswordStore),
        },
    );
    app.sessions.push(session);
    app.active_session = Some(0);
    app.mode = AppMode::Session;
    app.terminal_area = ratatui::layout::Rect::new(0, 0, 140, 42);
    let mut terminal = Terminal::new(TestBackend::new(140, 42))?;
    for _ in 0..150 {
        app.sessions[0].drain();
        terminal.draw(|frame| sshub::tui::render(frame, &app))?;
        std::thread::sleep(Duration::from_millis(40));
    }
    let screen = app.sessions[0].parser.screen();
    let text = screen.contents();
    anyhow::ensure!(
        text.contains("cpu") || text.contains("CPU"),
        "btop did not render CPU panel: {text}"
    );
    let cells: Vec<_> = terminal.backend().buffer().content.iter().map(|c|
        serde_json::json!({"symbol": c.symbol(), "fg": format!("{:?}", c.fg), "bg": format!("{:?}", c.bg), "modifier": format!("{:?}", c.modifier)})
    ).collect();
    std::fs::write(
        output,
        serde_json::to_vec_pretty(
            &serde_json::json!({"width": 140, "height": 42, "cells": cells}),
        )?,
    )?;
    println!("btop_smoke=PASS output={output}");
    app.sessions[0].write(b"q")?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("btop") => btop_smoke(args.get(2).map(String::as_str).unwrap_or("btop-frame.json")),
        _ => {
            benchmark();
            Ok(())
        }
    }
}
