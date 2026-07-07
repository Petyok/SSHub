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
