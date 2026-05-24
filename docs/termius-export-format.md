# Termius export format (for importers)

This folder contains artifacts produced by `termius-exporter.js`. This README describes **data format** and **field relationships** (not install/run instructions).

## Output files

- `L00t.csv`: main host list (connections merged with identity/key hints)
- `ssh_keys/`: exported private keys (and optional passphrases)
- `snippets.csv`: exported snippets (label + script)

## `L00t.csv`

CSV details:

- Encoding: UTF-8 with BOM (first bytes are `EF BB BF`)
- Separator: comma (`,`)
- Quoting: all values are wrapped in `"`; `"` inside values are escaped as `""`
- Header:

```
Label,Host,Port,Username,Password,SSH_Key,OS
```

Row meaning (strings):

- `Label`: Termius connection title (may be empty)
- `Host`: hostname or IP
- `Port`: TCP port (e.g. `22`)
- `Username`: SSH username
- `Password`: **best-effort** password resolved from identities (see below). Empty string if unknown.
- `SSH_Key`: **best-effort** key label resolved from `key_id` (see below). Empty string if unknown.
- `OS`: Termius host OS hint (e.g. `ubuntu`). Can be empty.

### How `Password` is derived (identities)

Exporter parses decrypted objects and collects “identities” as JSON objects containing both:

- `username`
- `password`

Then, for each connection row, it tries:

- if any identity has `identity.username === connection.user_name` and `identity.password` is non-empty, then `Password` is set to that identity password.

Notes for importers:

- Matching is only by username (no host scoping), so if same username used on multiple hosts with different passwords, result can be wrong.
- Identities are **not** currently exported to a separate CSV; only the resolved password is written into `L00t.csv`.

### How `SSH_Key` is derived (keys)

Termius connections reference keys via opaque `key_id`. Exporter also extracts key objects (with fields like `label`, `private_key`, optional `passphrase`).

Because connection `key_id` does not directly include key label, current exporter uses a heuristic:

- It counts `key_id` frequency across connections.
- It sorts key_ids by frequency and assigns them to key labels in insertion order.
- The resulting label is written into `SSH_Key`.
- If mapping fails, `SSH_Key` becomes `key_id:<id>`.

Notes for importers:

- Treat `SSH_Key` in `L00t.csv` as **hint**, not a guaranteed correct mapping.
- Better approach (recommended): let user pick key from `ssh_keys/` in import UI when `SSH_Key` is empty or starts with `key_id:`.

## `ssh_keys/`

Directory contains:

- `*-<fp>.pem`: private key material as plain text (PEM)
- `*-<fp>.passphrase`: passphrase (if Termius stored one), plain text

Where:

- `<fp>` is a SHA-256 fingerprint of the private key text, truncated to 16 hex chars.
- Filename base is derived from Termius key label, but sanitized and truncated; if label is missing/invalid/too long (or looks like PEM), exporter uses `key-<fp>` as base.

Importer guidance:

- Prefer using `<fp>` to link `.pem` and `.passphrase`.
- Do not rely on filename base being stable; fingerprint is the stable identifier.

## `snippets.csv`

Header:

```
Label,Script
```

Details:

- `Script` newlines are encoded as literal `\n` sequences.
- `"` inside `Script` are escaped as `""`.

## Suggested import UI (popup)

For each row in `L00t.csv`:

- Show: `Label`, `Host`, `Port`, `Username`, `OS`
- Auth options:
  - If `Password` non-empty: allow “password auth” prefilled.
  - If `SSH_Key` non-empty and you can map to `ssh_keys/`: allow “key auth” preselected.
  - If mapping is uncertain: ask user to choose key from `ssh_keys/` list (display label base + `<fp>`).


