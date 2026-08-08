# Coverage map

Where the test suite actually goes, and — the part that matters — where it has
never been. Regenerate with `just coverage`.

Snapshot: **v0.13.4**, 2026-08-09, whole suite (`cargo llvm-cov`, 671 unit +
160 integration tests).

```
lines      64.8%   (23881/36826)
functions  72.4%   (2477/3421)
regions    63.2%   (39840/63024)
```

**383 functions in `src/` are never executed by any test, not once.**

The percentage is the boring number. The 383 is the useful one: a function no
test ever enters has never been observed working, and 766 passing tests say
nothing about it. This repo is ~98.5% LLM-written, and an agent will write any
amount of new logic rather than stop and ask — see
[oracle-tests.md](oracle-tests.md). That logic lands here.

## The never-executed list, largest first

| Lines | Function | Why it matters |
|---:|---|---|
| 266 | `handle_mouse` — `src/app/mouse.rs:67` | every mouse interaction in the app |
| 222 | `connect_host_entry` — `src/app/connect.rs:20` | the connect path itself |
| 193 | `render_tunnel_row` — `src/tui/screens/tunnels.rs:144` | |
| 189 | `render_host_form` — `src/tui/screens/host_form.rs:10` | |
| 178 | `render_ping_zoomed` — `src/tui/widgets/right_stack.rs:425` | |
| 175 | `render_tunnel_form` — `src/tui/screens/tunnels.rs:339` | |
| 173 | `push_public_key_to_host` — `src/app/push_key.rs:182` | **writes to a remote `authorized_keys`** |
| 146 | `cmd_edit` — `src/cli/host.rs:341` | |
| 142 | `run_terminal_loop` — `src/lib.rs:253` | the event loop |
| 142 | `cmd_connect` — `src/cli/host.rs:140` | |
| 123 | `poll_keys_and_watcher` — `src/lib.rs:431` | |
| 120 | `render_tunnels` — `src/tui/screens/tunnels.rs:12` | |
| 109 | `render_field_picker` — `src/tui/screens/field_picker.rs:9` | |
| 101 | `render_identity_form` — `src/tui/screens/keychain.rs:46` | |
| 72 | `verify_host_key` — `src/sftp/transport.rs:126` | **TOFU host key check — 0 of 93 regions** |

Whole files at 0%:

| File | Coverage | Uncovered lines |
|---|---:|---:|
| `src/tui/screens/tunnels.rs` | 0% | 466 |
| `src/app/connect.rs` | 0% | 201 |
| `src/tui/screens/host_form.rs` | 0% | 160 |
| `src/tui/screens/keychain.rs` | 0% | 113 |
| `src/tui/screens/push_key_pickers.rs` | 0% | 111 |
| `src/tui/screens/group_form.rs` | 0% | 90 |
| `src/tui/screens/field_picker.rs` | 0% | 88 |
| `src/tui/screens/tag_filter.rs` | 0% | 78 |
| `src/tui/screens/tunnel_reconnect.rs` | 0% | 77 |
| `src/tui/screens/keygen.rs` | 0% | 60 |
| `src/tunnel/audit.rs` | 0% | 55 |
| `src/tui/screens/hosts.rs` | 0% | 51 |
| `src/cli/sftp.rs` | 5% | 255 |
| `src/cli/tunnel.rs` | 6% | 253 |

Note that AGENTS.md § Tests already says *"New overlays/screens need a render
smoke test"*. Ten screens render at 0%. A rule that only lives in Markdown is
advisory; this table is what it looks like after a year of being advisory.

## What to do with it, by group

**Paths that write or defend.** `verify_host_key`, `push_public_key_to_host`,
`connect_host_entry`. These reach the network, a remote `authorized_keys`, or
decide whether to trust a host key. All three have an external oracle available:
a real `sshd` in a container, plus `ssh-keygen -l` for fingerprints. Test them
against it, per [oracle-tests.md](oracle-tests.md). Highest value per hour.

**Screens at 0%.** No oracle exists and none can. Snapshot the rendered frame:
`ratatui::backend::TestBackend` is already used in `tests/e2e/`. A snapshot puts
human review on the *output* rather than the code, which is the right place for
scarce attention, and any silent behaviour change shows up as a diff.

**The event loop.** `run_terminal_loop`, `poll_keys_and_watcher`, `handle_mouse`
— 531 lines nothing has ever run. `handle_mouse` is the largest single
never-executed block in the repo.

No coverage floor in CI, deliberately: a percentage gate is satisfied by tests
that execute code without asserting anything, which is worse than the gap it
hides. Close the gaps above with tests that would fail if the behaviour were
wrong, and the number follows.

## Reading the report yourself

`just coverage` prints the summary and the never-executed list
(`scripts/uncovered-functions.py` over the `cargo llvm-cov` JSON). Two traps
are already handled there, both of which produced confidently wrong numbers
before they were:

1. **The report lists each function once per test binary.** A function covered
   under `--test e2e` still appears with `count: 0` under the binaries that
   never touched it. Counts must be summed per mangled name — otherwise the
   list comes out ~3× too long.
2. **A zero `count` is not proof.** The report also carries degenerate entries
   (a monomorphization in a codegen unit nothing called) sitting on lines the
   suite does execute — `render_palette` shows up this way while being covered
   477/590. The file's line segments are the arbiter.

3. **Single-line spans are not functions.** An entry whose regions collapse onto
   one line is an unused instantiation or a closure folded onto its signature;
   it passes the segment check by accident, because there is only one line to
   look at. `expand_tilde` surfaced that way despite having its own test.

If you extend the script, check it against two functions you already know the
answer for — one covered, one not — before trusting a single number it prints.
