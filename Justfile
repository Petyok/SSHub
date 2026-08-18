# ssh-launcher — common dev commands

default:
    @just --list

# Run all test targets (unit + integration). CI-friendly; no TTY required.
test:
    cargo test --no-default-features
    cargo test --no-default-features --test smoke
    cargo test --no-default-features --test e2e
    cargo test --no-default-features --test config_load

# Coverage, plus the functions no test executes even once (docs/coverage-map.md).
coverage:
    #!/usr/bin/env bash
    # Regenerates the tables in docs/coverage-map.md — read that file for why the
    # never-executed list matters more than the percentage, and for the two ways
    # this report is easy to misread.
    #
    # Needs cargo-llvm-cov plus llvm-cov/llvm-profdata. With rustup that is
    # `cargo install cargo-llvm-cov` + `rustup component add llvm-tools-preview`;
    # on a distro toolchain (no rustup) install the system `llvm` package
    # instead — the recipe finds those binaries itself.
    set -euo pipefail
    command -v cargo-llvm-cov >/dev/null || { echo "cargo-llvm-cov not installed — see the comments in this recipe" >&2; exit 1; }
    # Distro toolchains have no llvm-tools-preview; point cargo-llvm-cov at the
    # system binaries when they are the only ones present.
    if ! command -v rustup >/dev/null; then
        export LLVM_COV="${LLVM_COV:-$(command -v llvm-cov)}"
        export LLVM_PROFDATA="${LLVM_PROFDATA:-$(command -v llvm-profdata)}"
    fi
    out=$(mktemp -d)/cov.json
    cargo llvm-cov --json --output-path "$out"
    cargo llvm-cov --summary-only
    echo
    echo "Functions no test executes even once (src/ only, largest first):"
    scripts/uncovered-functions.py "$out"

# Build release binary (install depends on this recipe — no cargo in the install script).
build:
    cargo build --release

# Assemble the npm packages into npm/dist from the tarballs attached to the
# matching GitHub release. Nothing is compiled and nothing is published; the
# shim is exercised end to end first. Version defaults to Cargo.toml's.
npm-build version="":
    npm/build.sh {{version}}

# Assemble, verify, then publish to npm. Needs `npm login` (or a token in
# ~/.npmrc). Platform packages publish first, the `sshub` wrapper last, so the
# wrapper never lands pointing at binaries that do not exist yet.
npm-publish version="":
    npm/build.sh {{version}} --publish

# Record the README GIFs + screenshots (requires `agg` and `ffmpeg` on PATH:
# cargo install --git https://github.com/asciinema/agg). Pass scenario names to
# record a subset: `just record-gifs hero sftp`.
record-gifs *scenarios:
    demo/record.py {{scenarios}}

# Run with dry-run (no TUI)
dry-run:
    cargo run -- --dry-run

# Preview the man page (man/sshub.1) without installing it.
man:
    man -l man/sshub.1

# Install shell completions so they work with no manual setup. bash and fish
# files drop into auto-loaded dirs; zsh gets a sourced line appended to
# ~/.zshrc (idempotent, marked, removed by `just uninstall`). Included by
# `just install`; run standalone to (re)install just the completions.
install-completions: build
    #!/usr/bin/env bash
    set -euo pipefail
    bin=target/release/sshub
    bash_dir="${XDG_DATA_HOME:-$HOME/.local/share}/bash-completion/completions"
    fish_dir="${XDG_CONFIG_HOME:-$HOME/.config}/fish/completions"
    install -d "$bash_dir" "$fish_dir"
    "$bin" completions bash > "$bash_dir/sshub"
    "$bin" completions fish > "$fish_dir/sshub.fish"
    echo "completions: bash -> $bash_dir/sshub"
    echo "completions: fish -> $fish_dir/sshub.fish"
    # zsh has no user dir on the default fpath, so source the completion from
    # ~/.zshrc (only if it exists) instead. compinit is ensured before compdef.
    zshrc="$HOME/.zshrc"
    marker="# >>> sshub completions >>>"
    if [ -f "$zshrc" ] && grep -qF "$marker" "$zshrc"; then
      echo "completions: zsh -> ~/.zshrc already wired"
    elif [ -f "$zshrc" ]; then
      {
        echo ""
        echo "$marker"
        echo '(( $+functions[compdef] )) || { autoload -Uz compinit && compinit -u; }'
        echo 'source <(command sshub completions zsh)'
        echo "# <<< sshub completions <<<"
      } >> "$zshrc"
      echo "completions: zsh -> appended sourcing to ~/.zshrc (run: exec zsh)"
    else
      echo "completions: zsh -> no ~/.zshrc found; add: source <(sshub completions zsh)"
    fi
    echo "bash and fish auto-load in a new shell."

# Bump the version (odometer; Z 0-9, Y 0-99; see CLAUDE.md "Versioning").
#   just bump patch       # every commit to development
#   just bump minor       # on release (merge development -> main); resets patch
#   just bump major       # milestone / manual
#   just bump set 0.7.0   # set an explicit version (e.g. to jump ahead)
# Carries over: 0.4.9 + patch -> 0.5.0, 0.9.9 + patch -> 0.10.0,
# 0.99.0 + minor -> 1.0.0, 0.99.9 + patch -> 1.0.0.
bump kind version="":
    #!/usr/bin/env bash
    set -euo pipefail
    ver=$(grep -m1 '^version = ' Cargo.toml | sed -E 's/version = "([^"]+)".*/\1/')
    IFS=. read -r X Y Z <<< "$ver"
    case "{{kind}}" in
      patch) Z=$((Z + 1)); if [ "$Z" -gt 9 ]; then Z=0; Y=$((Y + 1)); fi
             if [ "$Y" -gt 99 ]; then Y=0; X=$((X + 1)); fi; new="$X.$Y.$Z" ;;
      minor) Y=$((Y + 1)); Z=0; if [ "$Y" -gt 99 ]; then Y=0; X=$((X + 1)); fi; new="$X.$Y.$Z" ;;
      major) X=$((X + 1)); Y=0; Z=0; new="$X.$Y.$Z" ;;
      set)   new="{{version}}"
             echo "$new" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$' \
               || { echo "usage: just bump set X.Y.Z" >&2; exit 1; } ;;
      *) echo "usage: just bump patch|minor|major|set X.Y.Z" >&2; exit 1 ;;
    esac
    sed -i -E "s/^version = \"[^\"]+\"/version = \"$new\"/" Cargo.toml
    # Update the sshub entry in Cargo.lock too (the version line right after its
    # name), so no `cargo` invocation is needed — keeps the pre-commit hook fast
    # and offline.
    sed -i "/^name = \"sshub\"$/{n;s/^version = .*/version = \"$new\"/}" Cargo.lock
    echo "bumped $ver -> $new"

# One-time per clone: point git at the tracked hooks in .githooks (enables the
# auto patch-bump pre-commit hook on the development branch).
setup-hooks:
    git config core.hooksPath .githooks
    @echo "git hooks enabled (core.hooksPath = .githooks)"

# Point this checkout's build output at the shared ../.cargo-target (sibling of
# the main checkout). One Rust artifact dir for the main checkout + every agent
# worktree. Safe to re-run; run it once per checkout/worktree.
#
# Uses .cargo/config.toml (untracked) rather than a `target` symlink: `cargo
# clean` DELETES the symlink, after which cargo silently starts building into a
# private ./target again — that is how one shared 1 GB dir turns into N copies.
# The path written is absolute because worktrees sit one level deeper than the
# main checkout, so no single relative path is correct for both.
setup-shared-target:
    #!/usr/bin/env bash
    set -euo pipefail
    root="$(git rev-parse --show-toplevel)"
    cd "$root"
    # --git-common-dir points at the MAIN checkout's .git from any worktree.
    common="$(realpath "$(git rev-parse --git-common-dir)")"
    shared="$(dirname "$(dirname "$common")")/.cargo-target"
    mkdir -p "$shared" .cargo
    cfg=.cargo/config.toml
    if [ -f "$cfg" ] && ! grep -q '^target-dir' "$cfg"; then
      echo "$cfg exists without a target-dir — add [build] target-dir by hand" >&2
      exit 1
    fi
    # Migrate whatever ./target is today, then get it out of the way.
    if [ -L target ]; then
      rm -f target
    elif [ -d target ]; then
      echo "moving ./target into $shared"
      cp -a target/. "$shared"/ && rm -rf target
    fi
    printf '[build]\ntarget-dir = "%s"\n' "$shared" > "$cfg"
    echo "target-dir -> $shared ($(du -sh "$shared" | cut -f1))"

# Prune the shared target dir. Cargo NEVER deletes artifacts of branches you
# moved off, so the dir only grows: a full dev+test+release cycle is ~1.2 GB and
# every stale fingerprint stays forever. Keeps it under a size cap, dropping the
# oldest artifacts first.
#
#   just sweep           # cap at 4GB
#   just sweep 2GB
#
# By age instead of size: cargo sweep --time 7 ../.cargo-target
sweep size="4GB":
    #!/usr/bin/env bash
    set -euo pipefail
    command -v cargo-sweep >/dev/null || { echo "needs: cargo install cargo-sweep" >&2; exit 1; }
    common="$(realpath "$(git rev-parse --git-common-dir)")"
    shared="$(dirname "$(dirname "$common")")/.cargo-target"
    before="$(du -sh "$shared" | cut -f1)"
    # cargo-sweep wants the PROJECT path (it resolves target-dir itself), not the
    # target dir — pointed at the latter it looks for a Cargo.toml inside it.
    cargo sweep --maxsize {{size}} "$(git rev-parse --show-toplevel)"
    echo "$shared: $before -> $(du -sh "$shared" | cut -f1)"

# Add an isolated worktree under ../.worktrees/<name> on branch <branch>
# (default: feature/<name>), then link its target/ at the shared cargo dir.
#   just worktree-add agent-foo
#   just worktree-add agent-foo feature/bar
# Run from the main checkout (or any worktree of this repo).
worktree-add name branch="":
    #!/usr/bin/env bash
    set -euo pipefail
    repo="$(git rev-parse --show-toplevel)"
    parent="$(dirname "$repo")"
    shared="$parent/.cargo-target"
    name="{{name}}"
    branch="{{branch}}"
    [ -n "$name" ] || { echo "usage: just worktree-add <name> [branch]" >&2; exit 1; }
    [ -n "$branch" ] || branch="feature/$name"
    dest="$parent/.worktrees/$name"
    mkdir -p "$parent/.worktrees"
    mkdir -p "$shared"
    if [ -e "$dest" ]; then
      echo "already exists: $dest" >&2
      exit 1
    fi
    # Ensure main checkout shares target too (no-op if already set up).
    just setup-shared-target
    if git show-ref --verify --quiet "refs/heads/$branch"; then
      git worktree add "$dest" "$branch"
    else
      git worktree add -b "$branch" "$dest"
    fi
    (cd "$dest" && just setup-shared-target)
    echo "worktree: $dest"
    echo "branch:   $branch"
    echo "target:   $shared (shared)"
    echo "cd $dest"

# Remove a worktree created by worktree-add. Does NOT delete the shared
# .cargo-target. Branch is left intact unless you pass delete-branch.
#   just worktree-rm agent-foo
#   just worktree-rm agent-foo delete-branch
worktree-rm name mode="":
    #!/usr/bin/env bash
    set -euo pipefail
    repo="$(git rev-parse --show-toplevel)"
    parent="$(dirname "$repo")"
    name="{{name}}"
    mode="{{mode}}"
    dest="$parent/.worktrees/$name"
    [ -n "$name" ] || { echo "usage: just worktree-rm <name> [delete-branch]" >&2; exit 1; }
    if [ ! -e "$dest" ]; then
      echo "no worktree at $dest" >&2
      exit 1
    fi
    branch="$(git -C "$dest" branch --show-current || true)"
    git worktree remove --force "$dest"
    if [ "$mode" = "delete-branch" ] && [ -n "$branch" ]; then
      git branch -d "$branch" || git branch -D "$branch"
      echo "deleted branch $branch"
    fi
    rmdir "$parent/.worktrees" 2>/dev/null || true
    echo "removed $dest"
    # That branch's artifacts are now unreachable but still in the shared dir.
    command -v cargo-sweep >/dev/null && just sweep || true

# Cut a release: merge development -> main with a --no-ff merge commit, tag,
# and push. The tag triggers the release workflow (binaries + crates.io
# publish). Development is then fast-forwarded to the release merge so both
# branches point at the same commit and the next release merges cleanly.
# `git log --first-parent main` shows one entry per release; reverting a whole
# release is `git revert -m 1 <merge>`, reverting one feature is a revert of
# its squashed commit (note: after reverting a merge, re-landing the same
# history needs a revert of the revert).
#
#   just release minor    # minor feature release: bump Y (Z->0) -> vX.Y.0
#   just release patch    # publish the CURRENT vX.Y.Z as-is, no bump
#   just release 0.7.0    # release an explicit version (jump ahead)
#
# No default on purpose: a bare `just release` used to mean `minor`, which
# silently bumped the version when you meant to ship what Cargo.toml already
# says (that is `patch`). Now it refuses and makes you say which one.
#
# `patch` ships whatever version development currently carries (the running
# odometer Z from the pre-commit hook) straight to main — for hotfixes you don't
# want to disguise as a new minor. So main is NOT always X.Y.0.
# Run from a clean `development`. Pushing to protected `main` relies on your
# owner/admin bypass.
release kind="":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "{{kind}}" ]; then
      echo "just release needs an explicit kind:" >&2
      echo "  patch  — ship the CURRENT Cargo.toml version as-is" >&2
      echo "  minor  — bump Y, reset Z -> vX.Y.0" >&2
      echo "  X.Y.Z  — release exactly that version" >&2
      exit 1
    fi
    case "{{kind}}" in minor|patch) ;; [0-9]*.[0-9]*.[0-9]*) ;; *) echo "usage: just release minor|patch|X.Y.Z" >&2; exit 1;; esac
    [ "$(git rev-parse --abbrev-ref HEAD)" = development ] || { echo "run from development" >&2; exit 1; }
    git diff --quiet && git diff --cached --quiet || { echo "working tree not clean" >&2; exit 1; }
    git fetch origin --quiet
    # Settle the release version ON DEVELOPMENT, so its odometer continues
    # from the released X.Y.Z instead of going stale (a stale dev version made
    # the next `just release minor` collide with an existing tag).
    # minor: bump Y and reset Z. patch: keep development's current X.Y.Z.
    # X.Y.Z: set that exact version.
    case "{{kind}}" in
      minor) just bump minor ;;
      patch) ;;
      *)     just bump set "{{kind}}" ;;
    esac
    ver=$(grep -m1 '^version = ' Cargo.toml | sed -E 's/version = "([^"]+)".*/\1/')
    if git rev-parse "v$ver" >/dev/null 2>&1; then
      echo "v$ver is already tagged — pick another version (or 'just release minor')" >&2
      git checkout -- Cargo.toml Cargo.lock; exit 1
    fi
    # Roll the changelog: [Unreleased] becomes [$ver] - <today>, with a fresh
    # empty [Unreleased] back on top. Skipped if $ver already has a section
    # (recovery re-run) or there is no [Unreleased] header.
    if grep -qF "## [$ver]" CHANGELOG.md; then
      echo "CHANGELOG.md already has a $ver section — skipping the roll"
    elif grep -q '^## \[Unreleased\]' CHANGELOG.md; then
      sed -i "0,/^## \[Unreleased\]/s//## [Unreleased]\n\n## [$ver] - $(date +%F)/" CHANGELOG.md
    else
      echo "warning: no [Unreleased] section in CHANGELOG.md — skipping the roll" >&2
    fi
    # Prep commit on development. --no-verify: the patch-bump pre-commit hook
    # must not move the version we just settled.
    if ! git diff --quiet; then
      git add Cargo.toml Cargo.lock CHANGELOG.md
      git commit --no-verify -m "chore: prep release v$ver"
    fi
    git push origin development
    git checkout main
    git pull --ff-only origin main
    # Real merge (--no-ff): main gets one release merge commit on its
    # first-parent line, while blame/bisect/revert see the full feature
    # history (version + changelog are already settled on dev). Merges
    # cleanly because dev is ff'd to main after every release, so main is
    # always an ancestor of dev here. If main ever gets a direct commit,
    # merge main into development first.
    git merge --no-ff development -m "chore: release v$ver"
    git tag -a "v$ver" -m "SSHub v$ver"
    git push origin main --follow-tags
    git checkout development
    # Fast-forward development to the release merge: both branches now point
    # at the same commit, ahead/behind is clean, and the next dev commit
    # hook-bumps the patch version from the released X.Y.Z.
    git merge --ff-only main
    git push origin development
    echo "released v$ver ({{kind}}) — 'chore: release v$ver' merged to main; the release workflow builds binaries and publishes to crates.io"

# Sync development -> main WITHOUT cutting a release: no version bump, no
# CHANGELOG roll, no tag (so the release workflow is NOT triggered). main gets
# a single `chore: sync development into main` merge commit on its first-parent
# line, then development is fast-forwarded back so both branches point at the
# same commit and the next release still merges cleanly. Use this to bring main
# up to date with dev (e.g. docs/CI fixes) between releases.
# Run from a clean `development`. Pushing to protected `main` relies on your
# owner/admin bypass.
sync:
    #!/usr/bin/env bash
    set -euo pipefail
    [ "$(git rev-parse --abbrev-ref HEAD)" = development ] || { echo "run from development" >&2; exit 1; }
    git diff --quiet && git diff --cached --quiet || { echo "working tree not clean" >&2; exit 1; }
    git fetch origin --quiet
    git push origin development
    if git merge-base --is-ancestor development main; then
      echo "main already contains development — nothing to sync"; exit 0
    fi
    git checkout main
    git pull --ff-only origin main
    # Real merge (--no-ff): main gets one sync merge commit on its first-parent
    # line. Merges cleanly because dev is ff'd to main after every release/sync,
    # so main is always an ancestor of dev here.
    git merge --no-ff development -m "chore: sync development into main"
    git push origin main
    git checkout development
    # Fast-forward development to the sync merge: both branches now point at the
    # same commit and ahead/behind stays clean.
    git merge --ff-only main
    git push origin development
    echo "synced development -> main (no release; no tag, no version bump)"

# Install the release binary to ~/.local/bin and a launcher entry so sshub
# shows up in your application launcher (GNOME, rofi, etc). Uses kitty if
# available, otherwise falls back to xterm. Runs `just build` first.
install: build install-completions
    #!/usr/bin/env bash
    set -euo pipefail
    bin="$HOME/.local/bin/sshub"
    term="$(command -v kitty || command -v ghostty || command -v alacritty || command -v foot || echo xterm)"
    install -Dm755 target/release/sshub "$bin"
    install -Dm644 man/sshub.1 "$HOME/.local/share/man/man1/sshub.1"
    install -Dm644 assets/sshub.svg "$HOME/.local/share/icons/hicolor/scalable/apps/sshub.svg"
    mkdir -p "$HOME/.local/share/applications"
    sed -e "s|@TERM@|$term|g" -e "s|@BIN@|$bin|g" \
        assets/sshub.desktop > "$HOME/.local/share/applications/sshub.desktop"
    update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
    gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
    echo "Installed $bin, man page, icon and launcher entry (terminal: $term)."
    echo "If it doesn't show up, log out/in or run: update-desktop-database ~/.local/share/applications"

# Remove the installed binary, man page, completions, icon and launcher entry.
uninstall:
    #!/usr/bin/env bash
    set -euo pipefail
    rm -f "$HOME/.local/bin/sshub" \
          "$HOME/.local/share/man/man1/sshub.1" \
          "${XDG_DATA_HOME:-$HOME/.local/share}/bash-completion/completions/sshub" \
          "${XDG_DATA_HOME:-$HOME/.local/share}/zsh/site-functions/_sshub" \
          "${XDG_CONFIG_HOME:-$HOME/.config}/fish/completions/sshub.fish" \
          "$HOME/.local/share/applications/sshub.desktop" \
          "$HOME/.local/share/icons/hicolor/scalable/apps/sshub.svg"
    # Strip the sshub completions block from ~/.zshrc if we added it.
    zshrc="$HOME/.zshrc"
    if [ -f "$zshrc" ] && grep -qF '# >>> sshub completions >>>' "$zshrc"; then
      sed -i '/# >>> sshub completions >>>/,/# <<< sshub completions <<</d' "$zshrc"
      echo "Removed sshub completions block from ~/.zshrc"
    fi
    echo "Removed sshub binary, man page, completions, icon and launcher entry."
