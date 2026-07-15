<!-- OPENWIKI:START -->

## OpenWiki

This repository uses OpenWiki for recurring code documentation. Start with `openwiki/quickstart.md`, then follow its links to architecture, workflows, domain concepts, operations, integrations, testing guidance, and source maps.

Implementation flow (issue → PR → merge): [docs/implementation-flow.md](docs/implementation-flow.md).

When posting GitHub issue/PR comments, always end with `_Written by {Model} ({Platform}) on behalf of the maintainer._` See the [GitHub comments section](docs/implementation-flow.md#github-comments-ai-agents) in implementation-flow.

## Lint before push (required)

**Always run lints locally before every commit or push** — do not rely on CI to catch formatting or clippy issues.

```bash
cargo fmt
cargo fmt --check    # must exit 0 (CI runs this)
cargo clippy --all-targets
```

Run these after code changes and again immediately before `git push`. If `cargo fmt --check` fails, run `cargo fmt` and include the formatting diff in your commit.

The scheduled OpenWiki GitHub Actions workflow regenerates this wiki by default. After each automated or manual update, validate and correct pages against the codebase before merge — automated output is a starting point, not the source of truth.

<!-- OPENWIKI:END -->
