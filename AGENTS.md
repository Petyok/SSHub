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
