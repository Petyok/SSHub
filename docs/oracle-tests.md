# Oracle tests

sshub is ~98.5% written by LLM agents. Agents produce a bug class humans rarely
do, and it is invisible to the checks we already run.

## The bug class

For a human, writing new logic is expensive, so when a human hits "where do I
get X from?" they stop and go find the real source. For an agent, writing new
logic is free — so it invents a plausible X and moves on. The result compiles,
passes clippy, and passes tests written by the same agent against the same
assumption.

Counting `unwrap()`, `let _ =` or lines-of-code does **not** find these. The
found example below has zero unwraps, zero warnings, and was covered by nine
green unit tests.

## The rule

> Wherever sshub re-implements something an external tool already knows,
> the test must ask the external tool — not a mock, not a fixture an agent wrote.

A mock only ever proves the code agrees with itself. That agreement is exactly
what an agent produces when it invents logic.

Fixtures under `tests/fixtures/` are fine for parsers of *foreign* formats, but
they are agent-authored too: they encode the same assumption as the code. Prefer
a real export from the real tool, and say in a comment where it came from.

## Where the oracles are

| Subsystem | Oracle | Status |
|---|---|---|
| `ssh/resolver.rs` — alias listing | `ssh -F <cfg> -G <alias>` must resolve every listed alias | done — `listed_aliases_round_trip_through_real_ssh` |
| `known_hosts.rs` | `ssh-keygen -F` / `-R` | done — see the `ssh-keygen`-derived assertion in its tests |
| `ssh/export.rs` | round-trip: export, then `ssh -G` the result and compare to the source host | **missing** |
| `import/{putty,mremoteng,termius_csv}.rs` | a real export file produced by that tool | **fixtures are agent-authored** |
| `session/`, `tui/`, `app/` | none exists | read the diff; keep files small |

Where no oracle exists, no amount of test count substitutes for reading the
diff. A large `#[cfg(test)]` module is not evidence — most of sshub's 766 tests
run against in-repo mocks of its own traits.

## Worked example (found by writing the first such test)

`Host "quoted-host"` in `~/.ssh/config`:

- Real OpenSSH strips the quotes and resolves the host normally.
- `collect_host_aliases` listed the alias verbatim, quotes included.
- `resolve_host` then ran `ssh -G '"quoted-host"'`, which fails with
  `hostname contains invalid characters`.

So the host appeared in the UI and could not be connected to. Nine unit tests
covered this parser; none of them asked OpenSSH anything. The differential test
failed on the first run, which is what a useful test looks like.

**When you add such a test, verify it fails without the fix.** A test written
after the fix, never seen red, proves only that it compiles.

## Cost

The differential test must degrade, not fail, when the external tool is absent:

```rust
if Command::new("ssh").arg("-V").output().is_err() {
    eprintln!("skipping: no ssh binary");
    return;
}
```
