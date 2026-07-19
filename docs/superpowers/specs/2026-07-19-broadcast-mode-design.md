# Broadcast mode: run a command on multiple hosts (#3)

## Problem

Fleet chores (check disk, restart an agent, read a version) require connecting
host by host. A broadcast run over a group or tag selection turns that into one
action: pick targets, type one command, run it non-interactively on all of them
concurrently, and read an aggregated result. The app has no async runtime, so
this rides the existing thread + mpsc worker pattern (`ping`, `sftp`, `os-detect`).

## Decisions (locked with user)

- **Target selection:** pick **one group OR one tag from a menu** ("Broadcast
  to:"). No per-host multi-select in v1; the dry-run preview offers `[e] edit
  targets` to deselect individual hosts before running.
- **Barrier before running:** command prompt -> **dry-run preview** listing the
  full target set + the command -> `[y]` confirm / `[e]` edit targets / `[N]`
  cancel. Protects against an accidental fleet-wide `reboot`/`rm`.
- **Not a modal — a background job + live docked widget.** On `[y]`, the preview
  overlay **slides with a short animation** (~250 ms, ease-out) from the center
  into a corner of the dashboard body and becomes a live **Broadcast panel**. The
  run executes in the background; the user can switch tabs and come back.
- **Focus/zoom integration (#18).** The Broadcast panel joins the panel
  focus-ring as `PanelId::Broadcast`. `Alt`+arrows focus it; `z` / `Alt+Enter`
  zoom it full-screen to read per-host output (scroll, failures first).
- **Completion -> countdown -> auto-dismiss.** When every host is done, a
  countdown bar (default 5 s) runs along the bottom of the panel; when it elapses
  the panel slides/fades out. **Focusing or zooming the panel pauses the
  countdown**, so output being read is never yanked away.
- **Transport:** `ssh` subprocess with `BatchMode`, reusing the exact
  arg-splicing from `src/osinfo/detect.rs::SshProbeRunner` — take
  `ssh_argv_for_entry`, splice `ConnectTimeout` + `BatchMode`, append the remote
  command as the final argv, capture stdout+stderr+exit. Inherits `~/.ssh/config`,
  ProxyJump, agent. **No PTY.**
- **Auth:** key/agent (`BatchMode=yes`) **and stored-password hosts** — the
  password/passphrase is resolved per host at target-pick time (`resolve_pending_secret`,
  the same path connect/detect use) and answered via `SSH_ASKPASS` + `PendingSecret`
  with `BatchMode=no`. Hosts with neither a key/agent nor a stored secret fail fast.
  (Stored-password was pulled forward from Phase 2 — real fleets are password-based,
  so key/agent-only made the feature unusable in practice.)
- **Concurrency:** bounded worker pool, default **8** concurrent; the rest queue.
  A single number, easy to make configurable later.
- **Audit (lightweight):** each host result is written via the existing
  `store.log_auth_event(via = "broadcast", status, note = "<cmd> (exit N)")`, so
  runs show up in the Audit tab as one line per host. **Full stdout/stderr is
  ephemeral** (live panel only) — no new tables in v1. A full-output audit
  drill-down is explicitly out of scope.
- **Cancel:** since the job is backgrounded, a cancel key (`x`) on the focused/
  zoomed Broadcast panel aborts pending tasks and kills in-flight `ssh`
  children. Auto-dismiss handles the finished case.

## Non-goals (v1)

- Grouping hosts by identical output / diff view.
- A saved command library (we remember only the **last** command for convenience).
- Per-host timeout overrides beyond the global connect + run timeouts.
- More than one broadcast run at a time (v1 runs one; starting a new one while
  one is live is disallowed with a notice).

## UX flow

```
hosts tab
  b
   -> "Broadcast to:"  (menu: groups + tags)
        pick #prod
   -> cmd> sudo systemctl restart nginx
   -> preview overlay:
        Run `sudo systemctl restart nginx` on 6 hosts (#prod):
          web-prod, web-staging, cache-1, cache-2, db-01, lb-1
        [y] confirm   [e] edit targets   [N] cancel
   -> y
        (preview slides to bottom-right corner, becomes Broadcast panel)

Broadcast panel (live, docked, background):
  ┌─ cast: systemctl restart nginx · #prod · 4/6 ─┐
  │ ⏳ web-prod   running                          │
  │ ✓  cache-1    exit 0                           │
  │ ✗  db-01      exit 255  ssh: connect refused   │
  │ …                                              │
  └─ Alt+↑↓ focus · z zoom · x cancel ────────────┘

  all done -> ▁▁▁▁▁ countdown 5s ▁▁▁▁▁ -> slides out
  (per-host summary lines already in Audit tab)
```

## Architecture

### New module: `src/broadcast/`

`mod.rs` owns the worker pool and pure types. No UI, no `App` dependency —
testable in isolation like `osinfo::detect`.

```rust
pub struct BroadcastTask {
    pub host_id: i64,
    pub host_name: String,
    pub argv: Vec<String>,   // ssh_argv_for_entry(entry), argv[0] == "ssh"
}

pub enum HostState { Pending, Running, Done { exit: i32 }, Failed { reason: String } }

pub struct HostResult {
    pub host_id: i64,
    pub host_name: String,
    pub state: HostState,
    pub stdout: String,      // captured, live-only (not persisted in v1)
    pub stderr: String,
}

pub enum BroadcastEvent {
    Started { host_id: i64 },
    Finished { host_id: i64, exit: i32, stdout: String, stderr: String },
    Failed { host_id: i64, reason: String },
}

/// Spawn a bounded pool. `runner` is abstracted (like ProbeRunner) so tests
/// inject canned output without spawning ssh. `cancel` kills in-flight work.
pub fn spawn_broadcast(
    tasks: Vec<BroadcastTask>,
    command: String,
    concurrency: usize,
    cancel: Arc<AtomicBool>,
    runner: Arc<dyn CommandRunner>,
) -> Receiver<BroadcastEvent>;

pub trait CommandRunner: Send + Sync {
    /// Run `argv` + remote `command`, return (exit, stdout, stderr) or an error
    /// reason. Honors `cancel` and a per-host run timeout.
    fn run(&self, argv: &[String], command: &str, cancel: &AtomicBool)
        -> Result<(i32, String, String), String>;
}
```

Pool shape: K worker threads pull `BroadcastTask`s from a shared
`Arc<Mutex<mpsc::Receiver<BroadcastTask>>>`; each sends `BroadcastEvent`s back on
one `mpsc::Sender<BroadcastEvent>`. The real `SshCommandRunner` mirrors
`detect::SshProbeRunner` (BatchMode splice, ConnectTimeout, capture), plus a run
timeout that kills the child so one hung host never holds a slot forever.

### App state

```rust
struct BroadcastState {
    target_label: String,        // "#prod" or "group: production"
    command: String,
    results: Vec<HostResult>,    // one per target, updated from events
    rx: Receiver<BroadcastEvent>,
    cancel: Arc<AtomicBool>,
    concurrency: usize,
    phase: BroadcastPhase,       // Running | Settling { done_at: Instant }
    anim: Option<SlideAnim>,     // entry slide; None once settled in place
}
// App: broadcast: Option<BroadcastState>
```

New `AppMode` variants drive the pre-run overlay stages: `BroadcastPickTarget`,
`BroadcastCommand`, `BroadcastPreview`. Once running, the mode returns to Normal
and the panel lives on the dashboard (background job) — it is NOT a mode.

### Event draining

The 50 ms poll loop drains `broadcast.rx` each tick (same as ping/sftp), applying
`BroadcastEvent -> HostResult` via a **pure reducer** (`apply_event`, unit-tested).
`results` stays in a stable per-target order (one row per host, updated in
place); **failures-first is a render-time sort of a view**, not a mutation of
`results`. When all results are terminal, set `phase = Settling { done_at: now }`
and start the countdown; write each host's `log_auth_event`. Focus/zoom of the
panel clears `done_at` back to a paused state; leaving it re-arms with a fresh
`done_at`.

### Rendering

`src/tui/screens/broadcast.rs`:
- pre-run overlay stages (menu / prompt / preview) — modal popups, reuse the
  existing popup + text-input widgets.
- the docked panel — a corner-anchored `Rect` in the dashboard body, drawn over
  the normal panels, but registered in the #18 focus-ring so `Alt`+arrows and
  `z`/`Alt+Enter` reach it. Zoomed view reuses the #18 zoom scaffolding
  (`zoom_window`, scroll, reverse-highlight, failures-first sort).
- the countdown bar — a thin gauge along the panel bottom driven by
  `done_at.elapsed() / DISMISS`.

### Animation

A small **general tween helper** (new; `animation.rs` today is only the startup
splash and is not reusable). `SlideAnim { from: Rect, to: Rect, start: Instant,
dur: Duration }` with an `ease_out` fn; `rect_at(now)` lerps the panel rect.
Driven by the existing per-tick re-render (20 fps over ~250 ms = ~5 frames).
Purely cosmetic — if `dur` elapsed, the panel is simply at `to`.

## Error handling

- Spawn failure / non-zero exit / run timeout -> `HostState::Failed` row with the
  reason; never aborts the whole run.
- Empty target (group/tag with no hosts) -> notice, no panel.
- Starting a broadcast while one is live -> notice ("a broadcast is already
  running"), refuse.
- Cancel -> `cancel` flag set, in-flight children killed, pending tasks skipped;
  remaining rows marked failed ("cancelled").
- ssh binary missing -> first host's spawn error surfaces as its reason (same as
  detect today).

## Testing (offline, no real ssh)

- **Pool** (`spawn_broadcast` + fake `CommandRunner`): concurrency is bounded,
  every task yields exactly one terminal event, cancel stops pending work,
  failures are reported not panicked.
- **Reducer** (`apply_event`): pure `BroadcastEvent -> Vec<HostResult>` — running/
  done/failed transitions update the right row in place; order stays stable.
- **View sort**: the render-time failures-first ordering is a pure fn over
  `&[HostResult]`.
- **Tween** (`rect_at`): endpoints and monotonic interpolation.
- **e2e (TestBackend):** `b` -> menu -> pick -> command -> preview -> confirm ->
  panel renders with rows; a canned runner drives rows to done; countdown appears.
- **Audit:** after a run, `list_auth_events` contains one `via = "broadcast"` row
  per host with the right status + note.

## Phasing

- **Phase 1 (shipped):** menu selection, preview barrier, background pool,
  live docked panel + slide animation + focus/zoom, countdown auto-dismiss,
  lightweight audit lines, cancel, and **key/agent + stored-password auth**
  (SSH_ASKPASS, pulled forward from Phase 2).
- **Phase 2 (deferred):** optionally full-output audit drill-down (new
  `broadcast_runs`/`_results` tables) if ephemeral output proves too limiting;
  configurable concurrency.
