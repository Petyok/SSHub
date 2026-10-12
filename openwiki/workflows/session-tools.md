---
type: Workflow
title: Session Tools - auth modal, snippets, ghost-text suggestions, command history, and the session log browser
description: The helpers layered on embedded sessions - the interactive SSH authentication modal (src/app/auth.rs), the command snippet library and in-session picker, local-only ghost-text completion from session history, opt-in per-host command history and snippets, and the read-only session log browser with search and bookmarks.
resource: src/app/session.rs
tags: [session, auth, snippets, suggestions, history, logs, workflow]
---

# Session Tools

These features sit on top of the [embedded sessions](sessions-sftp.md). None of them sends anything to a remote host except bytes the user explicitly chose (an answer, a snippet, an accepted completion).

## Interactive authentication modal

`AppMode::AuthPrompt`, `src/app/auth.rs`, screen `src/tui/screens/auth.rs`. Embedded ssh runs with `StrictHostKeyChecking=ask` and `NumberOfPasswordPrompts=3`, and its `SSH_ASKPASS` helper reports every prompt over a private socket (`src/session/askpass_channel.rs`). `App::poll_authentication` raises the modal for the first session with an open request, including background sessions, but never replaces another open editor.

Prompts are classified by `PromptClass::classify` (`src/session/auth_prompt.rs`) into password, passphrase, host key, or generic.

- **Password / passphrase / generic**: type the answer (paste is accepted unless it contains control characters; 4096-byte cap), `Ctrl+U` clears before the cursor, `Ctrl+W` deletes a word, Enter submits, `Esc` or `Ctrl+C` cancels authentication. `Tab` focuses the **Remember** checkbox and `Space` ticks it. The box is never pre-ticked and only offered when the prompt comes from the destination ssh (not a ProxyJump hop) of a managed host: a password prompt saves to the host credential, a passphrase prompt to the identity credential. The value is persisted only after ssh reports the connection authenticated; a failed save shows a notice and writes one `fail` audit row without the secret.
- **New host key** (first connection): `y` answers `yes`, `n` refuses. Enter does nothing, so trust can never be granted by a default key.
- **Changed host key** (ssh refused because the key changed): the modal shows a warning and the new fingerprint. Capital `A` removes the stale `known_hosts` entry (`known_hosts::remove_host`, same wildcard guard as the [known-hosts manager](known-hosts.md)) and reconnects; `n` or `Esc` closes.
- `PageUp` / `PageDown` scroll long prompts. Each session allows three password/passphrase prompts and three host-key questions before the channel is cancelled (`InteractiveAuth::poll`).
- A stored credential is answered silently when its kind matches the prompt class from the destination ssh; the modal only appears for what remains.

## Command snippets

A library of named commands stored in the `snippets` table of `launcher.db` (`src/store/snippets.rs`), shared across hosts.

- **Manager** (`Shift+S` on the dashboard, `AppMode::SnippetManage`): `j`/`k` move, `a` add, `e` or Enter edit, `d` delete (confirmed), Esc or `h` back.
- **Form** (`AppMode::SnippetForm`): fields Name, Command, Description (optional), Tags (space-separated, optional). Enter moves to the next field and saves on Tags; `F2`/`Ctrl+S` saves from any field; `Tab`/`Shift+Tab` and arrows move; `Ctrl+U` clears before the cursor; Esc with unsaved changes asks save/discard.
- **Picker** (`Ctrl+N` inside a running session, `AppMode::SnippetPicker`): floats over the session; typing fuzzy-filters over name, command, tags and description (`rank_snippets`, nucleo); arrows move; **Enter** pastes the command and sends Enter, **Tab** pastes it without Enter; Esc returns. Insertion goes through the session's paste path (`App::insert_session_command`), and failures show as a notice popup.

## Ghost-text suggestions

`src/app/ghost.rs`, ranking in `src/suggestions.rs` (`LocalSuggestions`). While typing at a live, authenticated shell, SSHub may paint one dimmed completion after the cursor. It is presentation only: nothing reaches the remote until accepted.

- Shown only when at least 2 characters are typed, the input tracker is certain, the screen is not the alternate screen, the view is not scrolled back, and no selection is active. Esc hides it until the line changes.
- Candidates come from this session's history, the host's persisted history (when enabled), and snippets; any candidate that `command_safety::classify_command` does not mark `Safe` (credential-like, oversized, control characters, shell expansion) is dropped. Only an exact-case prefix match can show inline.
- **Accept**: `Tab` or `Ctrl+F` (`KeyAction::GhostAccept`) writes only the missing suffix and never presses Enter. With no ghost visible, `Tab` goes to the remote shell as usual.

## Command history

- **Session history**: commands submitted with Enter in a live session are kept in memory (`SessionHistory`, up to 100 per session) after the same safety classification.
- **Persisted per-host history**: off by default. Settings "Persist command history" sets `[command_history] enabled`; "Maximum commands per host" adjusts `max_entries_per_host` (default 200, range 1 to 2000) with Left/Right. Rows live in the `command_history` table of `launcher.db`, keyed by managed host (ssh_config aliases without a launcher row are not persisted). Settings also offers Enter-to-clear rows for the current session, the selected host, or all hosts. The setting hint warns that stored commands may include secrets.

## Session log browser

`Shift+L` on the dashboard opens `AppMode::LogBrowser` (`src/app/log_browser.rs`, reader in `src/log_browser.rs`) over the active profile's `logs/` directory written by [session logging](sessions-sftp.md#session-logging-srcsession_logrs). It is read-only.

1. **Hosts**: one row per host directory with at least one segment. Enter opens it; Esc or `h` closes the browser.
2. **Segments**: the rotated `*.log` files for that host. Enter opens one; Esc goes back.
3. **Viewer**: the segment with terminal control sequences stripped (`strip_ansi`), at most 4 MiB read (`VIEWER_READ_CAP`, truncation is flagged). `j`/`k`, PageUp/PageDown (20 lines), Home/End scroll. `/` starts a case-insensitive search, Enter runs it, `n`/`N` cycle matches. `b` bookmarks the current line (blank name falls back to `line N`), `m` toggles the bookmark list for this host, where Enter jumps (loading another segment if needed) and `d` deletes. Esc steps back.

Bookmarks are stored in the `log_bookmarks` table (host dir, file name, line, name); a bookmark whose file name is not a single safe `*.log` component is refused before touching the filesystem (`is_safe_segment_name`). A segment pruned by retention reports "gone or unreadable" instead of opening blank.

## Change guidance

- Auth: tests in `src/app/auth.rs` and `src/session/interactive_auth.rs`; never log prompt answers (`AuthModal` deliberately has no `Debug`).
- Snippets: `src/app/tests/snippets.rs`, `tests/e2e/snippet_crud.rs`. Ghost and history: `src/app/tests/ghost.rs`, `src/app/tests/suggestions.rs`. Log browser: `src/app/tests/log_browser.rs`.
- Any new path that indexes or offers commands must go through `command_safety::classify_command`.
