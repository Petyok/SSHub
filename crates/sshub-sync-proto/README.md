# sshub-sync-proto

The protocol core of SSHub host sync (`docs/host-sync-contract.md` in the SSHub repository): device
crypto, op-log types and canonical bytes, wire frames, the SAS enrollment core, and the pure media
decoders (FSK, QR, BLE link layer). Pure Rust, no I/O, no threads, no clock; it builds for Android as
well as for the terminal.

Not a stable public API: it is published only because the `sshub` crate depends on it.
