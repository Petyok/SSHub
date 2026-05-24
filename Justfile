# ssh-launcher — common dev commands

default:
    @just --list

# Run all test targets (unit + integration). CI-friendly; no TTY required.
test:
    cargo test
    cargo test --test smoke
    cargo test --test e2e
    cargo test --test config_load

# Build release binary
build:
    cargo build --release

# Run with dry-run (no TUI)
dry-run:
    cargo run -- --dry-run
