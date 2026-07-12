# ssh-launcher — common dev commands

default:
    @just --list

# Run all test targets (unit + integration). CI-friendly; no TTY required.
test:
    cargo test
    cargo test --test smoke
    cargo test --test e2e
    cargo test --test config_load

# Build release binary (install depends on this recipe — no cargo in the install script).
build:
    cargo build --release

# Record README GIFs + screenshots with VHS (requires `vhs` and `ffmpeg` on
# PATH). Pass tape names to record a subset: `just record-gifs overview sftp`.
record-gifs *tapes: build
    bash demo/record.sh {{tapes}}

# Run with dry-run (no TUI)
dry-run:
    cargo run -- --dry-run

# Bump the version (odometer, each field 0-9; see CLAUDE.md "Versioning").
#   just bump patch       # every commit to development
#   just bump minor       # on release (merge development -> main); resets patch
#   just bump major       # milestone / manual
#   just bump set 0.7.0   # set an explicit version (e.g. to jump ahead)
# Carries over: 0.4.9 + patch -> 0.5.0, 0.9.9 + patch -> 1.0.0.
bump kind version="":
    #!/usr/bin/env bash
    set -euo pipefail
    ver=$(grep -m1 '^version = ' Cargo.toml | sed -E 's/version = "([^"]+)".*/\1/')
    IFS=. read -r X Y Z <<< "$ver"
    case "{{kind}}" in
      patch) Z=$((Z + 1)); if [ "$Z" -gt 9 ]; then Z=0; Y=$((Y + 1)); fi
             if [ "$Y" -gt 9 ]; then Y=0; X=$((X + 1)); fi; new="$X.$Y.$Z" ;;
      minor) Y=$((Y + 1)); Z=0; if [ "$Y" -gt 9 ]; then Y=0; X=$((X + 1)); fi; new="$X.$Y.$Z" ;;
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

# Cut a release: merge development -> main with a --no-ff merge commit, tag,
# and push. The tag triggers the release workflow (binaries + crates.io
# publish). Development is then fast-forwarded to the release merge so both
# branches point at the same commit and the next release merges cleanly.
# `git log --first-parent main` shows one entry per release; reverting a whole
# release is `git revert -m 1 <merge>`, reverting one feature is a revert of
# its squashed commit (note: after reverting a merge, re-landing the same
# history needs a revert of the revert).
#
#   just release          # minor feature release: bump Y (Z->0) -> vX.Y.0
#   just release minor    # same as above
#   just release patch    # hotfix: publish the CURRENT vX.Y.Z as-is, no bump
#   just release 0.7.0    # release an explicit version (jump ahead)
#
# `patch` ships whatever version development currently carries (the running
# odometer Z from the pre-commit hook) straight to main — for hotfixes you don't
# want to disguise as a new minor. So main is NOT always X.Y.0.
# Run from a clean `development`. Pushing to protected `main` relies on your
# owner/admin bypass.
release kind="minor":
    #!/usr/bin/env bash
    set -euo pipefail
    case "{{kind}}" in minor|patch) ;; [0-9]*.[0-9]*.[0-9]*) ;; *) echo "usage: just release [minor|patch|X.Y.Z]" >&2; exit 1;; esac
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

# Install the release binary to ~/.local/bin and a launcher entry so sshub
# shows up in your application launcher (GNOME, rofi, etc). Uses kitty if
# available, otherwise falls back to xterm. Runs `just build` first.
install: build
    #!/usr/bin/env bash
    set -euo pipefail
    bin="$HOME/.local/bin/sshub"
    term="$(command -v kitty || command -v ghostty || command -v alacritty || command -v foot || echo xterm)"
    install -Dm755 target/release/sshub "$bin"
    install -Dm644 assets/sshub.svg "$HOME/.local/share/icons/hicolor/scalable/apps/sshub.svg"
    mkdir -p "$HOME/.local/share/applications"
    sed -e "s|@TERM@|$term|g" -e "s|@BIN@|$bin|g" \
        assets/sshub.desktop > "$HOME/.local/share/applications/sshub.desktop"
    update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
    gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
    echo "Installed $bin, icon and launcher entry (terminal: $term)."
    echo "If it doesn't show up, log out/in or run: update-desktop-database ~/.local/share/applications"

# Remove the installed binary and launcher entry.
uninstall:
    rm -f "$HOME/.local/bin/sshub" \
          "$HOME/.local/share/applications/sshub.desktop" \
          "$HOME/.local/share/icons/hicolor/scalable/apps/sshub.svg"
    @echo "Removed sshub binary, icon and launcher entry."
