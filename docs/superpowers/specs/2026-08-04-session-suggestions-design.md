# Session suggestions: in-terminal history and snippets (#72)

## Problem

SSHub owns the PTY and forwards raw input to arbitrary remote shells and full-
screen programs. It cannot safely assume Bash/readline state, detect a shell
prompt, or send shell-specific editing commands. Session suggestions must add
local history and snippets without corrupting remote applications or creating a
silent credential-leak surface.

## Decisions

- V1 opens a local picker through an explicit configurable chord, default
  `Ctrl+Space`. No automatic popup on printable input.
- Current-session history is always in memory and is bounded.
- Persisted history is opt-in and disabled by default.
- Persisted history is limited to managed hosts with a managed ID. SSH config
  hosts without a managed ID never receive a persistent history index.
- Persisted history uses a bounded SQLite table, separate from raw session logs.
- Use one conservative whole-command redaction classifier for history and any
  future command-input logging path. When uncertain, drop the whole command.
- V1 inserts suggestions at the remote cursor. It does not try to replace the
  current remote line with `Ctrl+U` or cursor movement.
- `Tab` inserts without executing; `Enter` inserts and executes; `Esc` closes
  without writing to the PTY.
- History and snippets share one provider and picker model.
- V1 does not run hidden remote commands for path completion or shell probing.
- History is local-only and must not be included in future host-sync data.

## UX

While a live session is focused:

```text
Ctrl+Space
```

opens an overlay:

```text
┌─ Command suggestions · web-prod ─────────────────────────────┐
│ > dock                                                       │
│                                                              │
│ ▸ docker compose logs -f                                     │
│   docker compose ps                                          │
│   docker system prune                                        │
│   sshub sync                                                  │
│                                                              │
│ ↑↓ select · Tab insert · Enter insert + run · Esc close      │
└──────────────────────────────────────────────────────────────┘
```

Input while picker is open stays local. The remote PTY continues draining and
the session remains rendered behind the overlay.

- `Ctrl+Space`: open picker; the chord is not forwarded.
- Printable keys: edit local query.
- `Backspace`: remove query character.
- `Up` / `Down`: move selection.
- `Tab`: insert selected command without Enter.
- `Enter`: insert selected command and send Enter.
- `Esc`: close without writing to PTY.
- `Ctrl+C`: close picker without sending remote interrupt.

The picker closes safely when the session exits or is detached.

## Suggestion model

Use one model for all sources:

```rust
pub struct Suggestion {
    pub text: String,
    pub title: String,
    pub source: SuggestionSource,
    pub detail: Option<String>,
}

pub enum SuggestionSource {
    SessionHistory,
    HostHistory,
    Snippet,
    RemoteHint,
}
```

Providers receive session context and local query:

```rust
pub trait SuggestionProvider {
    fn suggestions(
        &self,
        context: &SuggestionContext,
        query: &str,
    ) -> Vec<Suggestion>;
}
```

Provider order:

1. Current-session history
2. Persisted host history
3. Snippets from #2
4. Future remote hints

Deduplicate by normalized command text. Rank exact prefix, word-prefix, fuzzy
match, source priority, recency, then use count. Reuse existing `nucleo`
dependency for fuzzy matching.

## PTY insertion

Use existing bracketed-paste-aware session writing:

```rust
session.write_paste(command.as_bytes())?;
```

On `Enter`, write the suggestion first, then encoded Enter. Suggestions are
single-line only; newline, NUL, and other control characters are rejected.

Do not implement remote line replacement in v1. `Ctrl+U`, cursor movement, and
shell history controls are not portable and could damage Vim, Tmux, `fzf`, a
password prompt, or another interactive program.

## Current-session history

Each `Session` gets bounded in-memory state:

```rust
pub struct SessionHistory {
    entries: Vec<HistoryEntry>,
    input: InputTracker,
}
```

The tracker observes locally generated input before it is forwarded. It handles
printable characters, backspace, Enter, `Ctrl+C`, `Ctrl+U`, and paste events.
On Enter it trims the tracked line, rejects empty input, applies the redaction
classifier, records safe input, and clears the tracker.

Tracking is best-effort. Remote readline history, vi mode, cursor movement, and
full-screen applications may desynchronize it. Desynchronization may lose a
history item but must never change PTY behavior.

Suggested bound: 100 commands per session.

Pasted multiline text is split into candidate lines. Empty lines and content
captured while a password/passphrase prompt is visible are not indexed.

## Persisted history

Configuration:

```toml
[command_history]
enabled = false
max_entries_per_host = 200
```

Add `CommandHistoryConfig` to `AppConfig`. Disabling persistence stops new
writes but does not silently delete existing entries. Deletion is explicit.

Use the existing owner-only `launcher.db`:

```sql
CREATE TABLE command_history (
    id          INTEGER PRIMARY KEY,
    host_id     INTEGER NOT NULL,
    command     TEXT NOT NULL,
    last_used   INTEGER NOT NULL,
    use_count   INTEGER NOT NULL DEFAULT 1,
    created_at  INTEGER NOT NULL,
    UNIQUE(host_id, command),
    FOREIGN KEY(host_id) REFERENCES hosts(id) ON DELETE CASCADE
);

CREATE INDEX command_history_host_recent
ON command_history(host_id, last_used DESC);
```

Enforce maximum command length and per-host row count transactionally. Host
deletion cascades history. History is scoped by managed host ID and is never
created for unmanaged SSH config entries.

History is local-only. Future host-sync operations must exclude this table;
commands may contain sensitive local workflow data even after redaction.

## Redaction

Add one shared classifier, for example `src/command_safety.rs`:

```rust
pub enum CommandSafety {
    Safe,
    Sensitive { reason: &'static str },
}

pub fn classify_command(command: &str) -> CommandSafety;
```

Conservatively reject common credential forms:

- `password=`, `passwd=`, `token=`, `secret=`, `api_key=`
- `*_PASSWORD`, `*_TOKEN`, and `*_SECRET` assignments
- `Authorization: Bearer ...`
- `curl -u user:password`
- `mysql -pSECRET`
- `sshpass`
- `docker login -p ...`
- `PGPASSWORD=...`
- `AWS_SECRET_ACCESS_KEY=...`

Also reject commands with control characters, excessive length, or input
captured while a password/passphrase prompt is visible. Drop whole commands
instead of attempting partial replacement. False positives are preferable to
persisting a credential.

This classifier controls history indexing. Existing raw transcript logging
remains unchanged in this issue; any future command-input logging must call the
same classifier instead of adding a second redaction list.

## Picker state and routing

Add a session-only picker state to `App`:

```rust
pub struct SessionSuggestionPicker {
    pub query: String,
    pub selected: usize,
    pub suggestions: Vec<Suggestion>,
    pub return_mode: AppMode,
}
```

Add `AppMode::SessionSuggestions`. Route input before normal session handling:

```text
App::handle_key
  -> suggestion picker, if open
  -> session actions
  -> scroll handling
  -> encode key and write to PTY
```

Only one picker exists because only one session is visible and active at once.

## Snippets and remote hints

The picker must not know snippet storage details. Issue #2 should expose a
provider using the same suggestion model. Selecting a snippet does not add it
to history until it is actually sent to the PTY.

Do not run hidden remote commands such as `compgen`, `find`, or `pwd` in v1.
They can pollute shell history, execute in the wrong application, expose remote
paths, and require unreliable shell detection. Path/common-subcommand hints
remain an extension point for a future explicit shell capability or installed
remote helper.

## Settings and clearing

Expose persistence in Settings:

```text
Persist command history: off / on
Maximum commands per host: N
```

Provide explicit actions for:

- Clear current-session history
- Clear selected host history
- Clear all persisted command history

## Security requirements

- Persistence off by default.
- No persistent history for unmanaged hosts.
- Owner-only database permissions.
- No command text in diagnostics, notices, audit records, or toasts.
- No remote command execution for suggestions.
- Query text stays local to SSHub.
- Escape never writes to the PTY.
- Bounded memory, command length, and database rows.
- History excluded from future sync.

## Testing

### Pure tests

- Redaction accepts safe commands and rejects credential patterns.
- Prompt-context input is rejected.
- Newline, control-character, and length limits work.
- Input tracker handles characters, backspace, Enter, paste, and Ctrl+C.
- History deduplication, recency, bounds, and ranking are deterministic.

### Store tests

- Migration creates `command_history`.
- Insert/update increments `use_count`.
- Host histories remain isolated.
- Per-host bounds are enforced.
- Host deletion removes history.
- Unmanaged hosts cannot write history.
- Database permissions remain owner-only.

### App and render tests

- `Ctrl+Space` opens picker and is not written to PTY.
- Query keys remain local while picker is open.
- `Esc` closes without PTY writes.
- `Tab` inserts without Enter.
- `Enter` inserts and executes.
- Remote output does not dismiss picker.
- Session exit closes picker safely.
- Closed picker preserves existing session forwarding.
- TestBackend renders empty, selected, long-command, and narrow-terminal states.
- Sensitive commands never appear in suggestions.

## Implementation phases

1. Add command-safety classifier and pure tests.
2. Add session input tracker and bounded in-memory history.
3. Add opt-in config and SQLite history migration.
4. Add provider model and picker overlay.
5. Add configurable trigger and insertion behavior.
6. Integrate snippets provider from #2.
7. Add settings and history-clearing actions.
8. Update README, in-app help, changelog, and OpenWiki.
9. Design remote hints separately after a shell-capability decision.
