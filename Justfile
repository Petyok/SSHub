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

# Record README GIFs with VHS (requires `vhs` on PATH).
record-gifs: build
    #!/usr/bin/env bash
    set -euo pipefail
    export SSHUB_CONFIG_DIR="$PWD/demo/home/.config/sshub"
    export SSHUB_DATA_DIR="$PWD/demo/home/.local/share/sshub"
    export SSHUB_SSH_CONFIG="$PWD/demo/home/ssh_config"
    export PATH="$PWD/demo/bin:$PWD/target/release:$PATH"
    vhs demo/tapes/overview.tape
    vhs demo/tapes/connect.tape
    vhs demo/tapes/add-host.tape

# Run with dry-run (no TUI)
dry-run:
    cargo run -- --dry-run

# Bump the version (odometer, each field 0-9; see CLAUDE.md "Versioning").
#   just bump patch   # every commit to development
#   just bump minor   # on release (merge development -> main); resets patch
#   just bump major   # milestone / manual
# Carries over: 0.4.9 + patch -> 0.5.0, 0.9.9 + patch -> 1.0.0.
bump kind:
    #!/usr/bin/env bash
    set -euo pipefail
    ver=$(grep -m1 '^version = ' Cargo.toml | sed -E 's/version = "([^"]+)".*/\1/')
    IFS=. read -r X Y Z <<< "$ver"
    case "{{kind}}" in
      patch) Z=$((Z + 1)); if [ "$Z" -gt 9 ]; then Z=0; Y=$((Y + 1)); fi
             if [ "$Y" -gt 9 ]; then Y=0; X=$((X + 1)); fi ;;
      minor) Y=$((Y + 1)); Z=0; if [ "$Y" -gt 9 ]; then Y=0; X=$((X + 1)); fi ;;
      major) X=$((X + 1)); Y=0; Z=0 ;;
      *) echo "usage: just bump patch|minor|major" >&2; exit 1 ;;
    esac
    new="$X.$Y.$Z"
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
