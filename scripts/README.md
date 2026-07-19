# scripts

Developer tooling. Not needed to build or run sshub; only to regenerate
vendored assets.

## OS logos

sshub ships two vendored logo sets, both keyed by the canonical distro ids in
`src/osinfo/logos.rs` (`CANONICAL_IDS`):

- **Small Braille** — `assets/os_logos.json`, used by the normal (compact)
  host-detail card. Generated offline from the `font-logos` SVG set via
  `magick -alpha extract` + `chafa --symbols braille` (see the docstring in
  `src/osinfo/logos.rs`). No script is checked in for this set.
- **Large full-colour** — `assets/os_logos_large.json`, used by the *zoomed*
  host-detail panel (`z` on the Hosts dashboard). Generated from
  [fastfetch](https://github.com/fastfetch-cli/fastfetch)'s built-in logos.

### Regenerating the large logos

Requires `fastfetch` on `PATH`.

```sh
python3 scripts/gen_os_logos.py    # writes assets/os_logos_large.json
```

For each canonical id it runs `fastfetch --logo <name> --pipe false -s " "`
(prints only the logo, with ANSI colour codes), parses the SGR colours into
per-span ANSI indices / truecolour, and stores structured coloured lines. Rust
maps ANSI indices to ratatui named `Color`s at load time, so logo colours follow
the terminal theme. Commit the regenerated JSON.
