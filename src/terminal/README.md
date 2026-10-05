# Embedded terminal model

Adapted from Jesse Luehrs vt100 and iksteen/shellglass crates/vt100 at
revision dc7f399f4ffc3daf0593ff9a272338ac9b1affc6 (MIT; see LICENSE).
The source is compiled as an SSHub module, not a path/git dependency or a
Cargo patch. It is included in the SSHub crates.io package, so packaging and
cargo install use exactly the same terminal model as development builds.

SSHub owns the integration adapter, bounded REP and OSC storage changes.

The adapter is adapted from tui-term 0.3.4 src/vt100_imp.rs,
Copyright (c) 2023 a-kenji, MIT; see LICENSE-tui-term.

The retained URI table is capped at 8192 entries of at most 2048 UTF-8 bytes;
titles are capped at 4096 UTF-8 bytes, with a 16-entry title stack. The raw
OSC buffer is fixed at 96 KiB (including unterminated OSC); valid 64 KiB
clipboard payloads fit after base64 encoding. REP is capped at grid area.

SSHub answers DSR 5/6, DA1, character-size and DECRQM 2026 through its streaming
query observer. It does not tee terminal output to the outer terminal and
leaves unsupported query types unanswered, preserving PR #114.
