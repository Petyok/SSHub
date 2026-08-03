<!-- OPENWIKI:START -->

## OpenWiki

This repository uses OpenWiki for recurring code documentation. Start with `openwiki/quickstart.md`, then follow its links to architecture, workflows, domain concepts, operations, integrations, testing guidance, and source maps.

The scheduled OpenWiki GitHub Actions workflow refreshes the repository wiki. Do not hand-edit generated OpenWiki pages unless explicitly asked; prefer updating source code/docs and letting OpenWiki regenerate.

<!-- OPENWIKI:END -->

## Multi-agent worktrees (shared Cargo target)

Main agent checkout stays in this folder (`ssh-tui/`). Extra agents use git
worktrees so they do not fight over the same working tree.

Rust `target/` is **shared** across the main checkout and all worktrees to
save disk (one ~12G artifact dir, not one per agent):

```text
sshub-dev/
  .cargo-target/          ← real cargo artifacts
  .worktrees/<agent>/     ← isolated checkouts
  ssh-tui/                ← main checkout (this repo)
    target -> ../.cargo-target
```

```bash
just setup-shared-target          # one-time / repair symlink on main checkout
just worktree-add agent-foo       # ../.worktrees/agent-foo on feature/agent-foo
just worktree-rm agent-foo        # remove worktree; keeps shared target
```

Do **not** run two `cargo`/`just test` builds at once against the shared
target — fingerprint races. Serialize builds (one agent compiling at a time).

## Implementation rules

Canonical source: [docs/implementation-flow.md](docs/implementation-flow.md). These are the agent-enforced highlights.

### Workflow

1. Claim the issue on GitHub before coding. Sign every issue/PR comment: `_Written by {Model} ({Platform}) on behalf of the maintainer._`
2. Branch `feature/*` (or `fix/*`) from `development`. Never from `main`. Never bump `Cargo.toml` version.
3. Small, logical commits. Conventional commit titles (`feat:`, `fix:`, `test:`, `docs:`, etc.).
4. PR targets `development` only. Body includes `Closes #N`, what changed, how tested, and the signature.

### Verify before every push

```bash
just test
cargo fmt
cargo fmt --check
cargo clippy --all-targets
```

All must pass. CI runs the same and fails on any warning.

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
