<!-- OPENWIKI:START -->

## OpenWiki

This repository uses OpenWiki for recurring code documentation. Start with `openwiki/quickstart.md`, then follow its links to architecture, workflows, domain concepts, operations, integrations, testing guidance, and source maps.

The scheduled OpenWiki GitHub Actions workflow refreshes the repository wiki. Do not hand-edit generated OpenWiki pages unless explicitly asked; prefer updating source code/docs and letting OpenWiki regenerate.

<!-- OPENWIKI:END -->

## Multi-agent worktrees (shared Cargo target)

Main agent checkout stays in this folder (`ssh-tui/`). Extra agents use git
worktrees so they do not fight over the same working tree.

Rust build output is **shared** across the main checkout and all worktrees to
save disk (~1.2G per full dev+test+release cycle, not one such dir per agent):

```text
sshub-dev/
  .cargo-target/          ← real cargo artifacts (build.target-dir)
  .worktrees/<agent>/     ← isolated checkouts
  ssh-tui/                ← main checkout (this repo)
    .cargo/config.toml    ← untracked, points at ../.cargo-target
```

Each checkout carries its own untracked `.cargo/config.toml` with an absolute
`build.target-dir`. There is **no `target` symlink** — `cargo clean` deletes a
symlink, after which cargo silently builds into a private `./target` and the
shared dir quietly becomes one copy per agent. If you find a `target` symlink,
do not repair it: run `just setup-shared-target`, which migrates it.

```bash
just setup-shared-target          # once per checkout/worktree (writes .cargo/config.toml)
just worktree-add agent-foo       # ../.worktrees/agent-foo on feature/agent-foo (runs the above)
just worktree-rm agent-foo        # remove worktree; sweeps its stale artifacts
just sweep                        # prune shared target back under 4GB (cargo never does)
```

The `vendored` feature (OpenSSL compiled from source) is **on by default**,
because `cargo install sshub` and the release tarballs must build with no system
OpenSSL present. Local work opts out with `--no-default-features` to link the
system libs instead — ~120 MB less target/ per profile and most of a cold
build's wall clock. `just test` already does. Never pass that flag on anything
that ships: `just build`, CI, and the release workflow stay on the default.

Do **not** run two `cargo`/`just test` builds at once against the shared
target — fingerprint races. Serialize builds (one agent compiling at a time).

## Implementation rules

Canonical source: [docs/implementation-flow.md](docs/implementation-flow.md). These are the agent-enforced highlights.

### Workflow

1. Claim the issue on GitHub before coding. Sign every issue/PR comment: `_Written by {Model} ({Platform}) on behalf of the maintainer._`
2. Branch from `development` per § Branch naming. Never from `main`. For epics, use `feature/<epic>` as integration branch and child `poc/*` or stage branches; never bump `Cargo.toml` version on non-development branches.
3. Small, logical commits. Conventional commit titles (`feat:`, `fix:`, `test:`, `docs:`, etc.).
4. Production PRs target `development`; exploratory child PoC/stage PRs may target their epic integration branch. Body includes `Closes #N`, what changed, how tested, and the signature.

### Branch naming

`<prefix>/<slug>`, where the prefix says what the work **is** — not what it is
attached to. A bug fix is `fix/`, however large; docs are `docs/`; `feature/` is
for a `feat:` commit and nothing else.

| Prefix | Justified by a commit of type |
|---|---|
| `feature/` | `feat` |
| `fix/` | `fix` |
| `docs/` | `docs` |
| `chore/` | `chore`, `ci`, `build`, `refactor`, `perf`, `style`, `test` |
| `poc/` | anything — an epic's exploratory child branch has no fixed shape |

Supporting commits of other types are fine: a `fix/` branch may carry `test:`
and `docs:` commits, it just needs at least one `fix:`.

Enforced by `scripts/check-branch-name.sh`, which the `pre-commit` hook calls
for the name shape and CI calls with every commit type on the branch (CI skips
forks — a contributor's branch name is not ours to police). The script's own
cases run as `scripts/check-branch-name.sh --self-test` in the same CI job.

This was prose until a bug fix shipped on a `feature/` branch (#95) and nothing
objected. Rules that only live in a Markdown file are advisory; move what you
can into a check.

### Verify before every push

```bash
just test
cargo fmt
cargo fmt --check
cargo clippy --all-targets
```

All must pass. CI runs the same and fails on any warning.

### Oracle tests

Code that re-implements what an external tool already knows (`ssh -G`,
`ssh-keygen`, a foreign export format) must be tested **against that tool**, not
against a mock or an agent-authored fixture — a mock only proves the code agrees
with itself, which is exactly what an agent produces when it invents logic.
Verify the new test fails without the fix. See [docs/oracle-tests.md](docs/oracle-tests.md).

### Adversarial review

After local green, run an independent adversarial review on the diff (2+ critics for focused changes, 3+ for features). Fix verified blockers/highs before pushing. Verdict must be `SAFE TO COMMIT` or equivalent.

### Review findings discipline

- **Verify every finding against code and test output before fixing.** Do not trust critic summaries blindly. Re-open the code, reproduce the claim, confirm it is real.
- If a finding is wrong, explain why with evidence. Do not apply speculative fixes.
- Separate blockers from nice-to-haves. Do not broaden scope unless the change created a real risk.
- Pre-existing issues found during review: note them, do not fix in the same PR unless they are blockers for the current change.

### Docs and changelog

- `CHANGELOG.md` under `[Unreleased]` for user-visible changes. Name external contributors per entry.
- Update `README.md`, in-app help (`src/tui/screens/help.rs`), and footer hints (`src/tui/mod.rs`) when UX or keybindings change. Undiscoverable features are bugs.
- Update `openwiki/` when architecture or operational behaviour changes.

### Tests

- Use fixtures and `tempfile`. Never touch real `~/.ssh`, keyring, or user config dirs.
- Tests that mutate process-wide env vars must serialize via `config::with_test_config_dir`.
- New overlays/screens need a render smoke test. New key actions need the full keybinds.rs insertion (macro arm, enum, ALL, label, config field, Default, default_for, binds, set).

### CI

Watch GitHub Actions after push (`gh pr checks <n> --watch`). Local green is not enough. Do not mark work complete until every required check passes.
