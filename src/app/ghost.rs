//! Kitty-style ghost-text completion for live sessions.
//!
//! Presentation only: the data layer (`InputTracker`, `SessionHistory`,
//! persisted host history, snippets, `LocalSuggestions` ranking) is shared
//! with the snippet picker and untouched here. This module computes the single
//! inline suffix for the current input line, accepts it, or dismisses it.
//! Painting lives in `crate::session::render` and touches only ratatui buffer
//! cells — never the parser, the PTY, or the remote.

use super::*;
use crate::session::history::HistoryEntry;
use crate::suggestions::{LocalSuggestions, SuggestionContext, SuggestionProvider};

/// Cached per-host persisted history for the ghost context:
/// `((host id, opt-in flag, per-host limit), rows)`. `None` is cold.
pub(crate) type GhostHostCache = Option<((Option<i64>, bool, usize), Vec<HistoryEntry>)>;

/// Minimum typed characters before a ghost may appear. A single character
/// matches nearly everything and the ghost would only add noise.
pub(crate) const GHOST_MIN_PREFIX: usize = 2;

/// The single inline completion for the current input line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhostMatch {
    /// Full suggestion text (first line shown; whole text accepted).
    pub text: String,
    /// `text` minus the already-typed prefix: the only bytes accept writes.
    pub suffix: String,
    /// First line of `suffix`: what the renderer paints (hard-clipped to the
    /// viewport there). The provider never yields controls today — the safety
    /// classifier rejects them — but the split is load-bearing if that ever
    /// changes: the renderer must never be handed a newline.
    pub shown: String,
}

impl App {
    /// Typed query backing the ghost, or `None` when no ghost may show for
    /// input reasons (not a live shell, uncertain tracker, too short, or a
    /// sticky Esc dismiss for exactly this line).
    fn ghost_query(&self) -> Option<&str> {
        let session = self.active_session()?;
        if !session.is_live_authenticated() {
            return None;
        }
        let typed = session.history.input.typed();
        if typed.chars().count() < GHOST_MIN_PREFIX {
            return None;
        }
        if self.ghost_dismissed_for.as_deref() == Some(typed) {
            return None;
        }
        Some(typed)
    }

    /// Per-host persisted history for the ghost context, cached so the render
    /// pass (which only holds `&App`) never hits the store per frame. The key
    /// is the active session's host id plus the opt-in flag and the per-host
    /// limit; a mismatch (session switch, settings toggle, limit edit) or an
    /// explicit invalidate reloads.
    fn ghost_host_history(&self) -> Vec<HistoryEntry> {
        let host_id = self.active_session().and_then(|s| s.meta.host_id);
        let enabled = self.config.command_history.enabled;
        let limit = self.config.command_history.max_entries_per_host;
        let mut cache = self.ghost_host_cache.borrow_mut();
        let cold = match cache.as_ref() {
            Some(((id, en, lim), _)) => *id != host_id || *en != enabled || *lim != limit,
            None => true,
        };
        if cold {
            let rows = if enabled {
                match host_id {
                    Some(id) => self.store.command_history(id, limit).unwrap_or_default(),
                    None => Vec::new(),
                }
            } else {
                Vec::new()
            };
            *cache = Some(((host_id, enabled, limit), rows));
        }
        cache
            .as_ref()
            .map(|(_, rows)| rows.clone())
            .unwrap_or_default()
    }

    /// The current ghost match, or `None` when hidden. Pure read apart from
    /// warming the host-history cache (SQLite, no PTY I/O), so the render
    /// pass may call it every frame.
    pub(crate) fn ghost_match(&self) -> Option<GhostMatch> {
        let query = self.ghost_query()?;
        let session = self.active_session()?;
        // A remote full-screen app owns its grid: never paint over vim/tmux.
        if session.parser.screen().alternate_screen() {
            return None;
        }
        // Scrolled back: the cursor row is not the input line any more.
        if session.parser.scrollback() > 0 {
            return None;
        }
        // An active selection owns the same cells the ghost would paint.
        if session.selection.is_some() {
            return None;
        }
        let host_history = self.ghost_host_history();
        let ranked = LocalSuggestions.suggestions(
            &SuggestionContext {
                session_history: session.history.entries.as_slice(),
                host_history: &host_history,
                snippets: &self.snippets,
            },
            query,
        );
        // Tier-0 prefix contract: only a suggestion that EXTENDS the typed
        // line can render inline. Fuzzy tier-1/2 matches are list-picker
        // concepts; painting one after the cursor would show text the accept
        // path cannot produce by suffix append. Ranking folds case, but the
        // boundary must be exact-case: folding only for the prefix test would
        // slice the suffix at the wrong bytes (typed `DO` off `docker ps`
        // would accept as `DOcker ps`), so the first exact-case prefix match
        // in rank order wins and a case-only match shows no ghost.
        let mut chosen = None;
        for top in ranked.into_iter() {
            if top.text.starts_with(query) {
                chosen = Some(top);
                break;
            }
        }
        let top = chosen?;
        // `starts_with` guarantees the char boundary, so this cannot slice
        // mid-char (or the wrong suffix).
        let suffix = top.text.get(query.len()..)?.to_owned();
        // Fully typed: nothing to complete, and Tab must reach the shell.
        if suffix.is_empty() {
            return None;
        }
        let shown = suffix.split(['\n', '\r']).next().unwrap_or("").to_owned();
        if shown.is_empty() {
            return None;
        }
        Some(GhostMatch {
            text: top.text,
            suffix,
            shown,
        })
    }

    /// Accept the current ghost: write ONLY the suffix (the prefix is already
    /// in the PTY) through the paste channel so the tracker stays in sync.
    /// Never appends Enter. Returns false when no ghost was visible, so the
    /// key falls through to its normal meaning (Tab reaches the shell).
    pub(crate) fn accept_ghost(&mut self) -> bool {
        let Some(ghost) = self.ghost_match() else {
            return false;
        };
        self.ghost_dismissed_for = None;
        let Some(session) = self.active_session_mut() else {
            return false;
        };
        session.observe_paste(&ghost.suffix);
        if session.write_paste(ghost.suffix.as_bytes()).is_err() {
            session.history.input.invalidate();
            self.suggestion_notice("Could not insert completion into session.");
        }
        true
    }

    /// Sticky Esc dismiss: hide until the input line changes. The tracker line
    /// is the key, so any further keystroke re-arms the ghost naturally.
    pub(crate) fn dismiss_ghost(&mut self) {
        if let Some(session) = self.active_session() {
            self.ghost_dismissed_for = Some(session.history.input.typed().to_owned());
        }
    }

    /// Drop ghost UI state across session transitions: a new tab's tracker
    /// line could coincidentally equal a dismissed line.
    pub(crate) fn reset_ghost(&mut self) {
        self.ghost_dismissed_for = None;
        self.ghost_host_cache.borrow_mut().take();
    }

    /// Invalidate the cached host history after the store changed underneath
    /// it (a command was just recorded); the next frame reloads.
    pub(crate) fn invalidate_ghost_host_cache(&self) {
        self.ghost_host_cache.borrow_mut().take();
    }
}
