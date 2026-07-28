# sshub-tui

A terminal UI for the SSH hosts you actually use: browse and search them, connect in embedded
tabs, move files over SFTP with a dual-pane browser, manage keys and tunnels.

```bash
npx sshub-tui              # run without installing
npm install -g sshub-tui   # then just: sshub
```

The command it installs is `sshub`. The package is `sshub-tui` because npm refuses the bare
`sshub` name for being too close to the existing `ssh2` and `sshpk`.

This package ships prebuilt binaries, so there is no build step and nothing is downloaded from
outside the registry: the platform-specific binary arrives as an optional dependency and npm
skips the ones that do not match your machine.

Prebuilt for Linux x64, macOS arm64 and macOS x64. On anything else, build from source:

```bash
cargo install sshub
```

`ssh` must be on your `PATH`. On Linux, host passwords and key passphrases are stored through a
Secret Service provider (gnome-keyring, KWallet), which has to be running and unlocked;
otherwise SSHub says so and ssh falls back to prompting.

Docs, screenshots and the changelog: https://github.com/Petyok/SSHub

Licensed under AGPL-3.0-or-later.
