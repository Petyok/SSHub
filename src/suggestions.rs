//! Local-only suggestion providers. No provider runs remote commands.
use crate::command_safety::{classify_command, CommandSafety};
use crate::session::history::HistoryEntry;
use crate::store::Snippet;
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SuggestionSource {
    SessionHistory,
    HostHistory,
    Snippet,
    RemoteHint,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub text: String,
    pub title: String,
    pub source: SuggestionSource,
    pub detail: Option<String>,
}
pub struct SuggestionContext<'a> {
    pub session_history: &'a [HistoryEntry],
    pub host_history: &'a [HistoryEntry],
    pub snippets: &'a [Snippet],
}
pub trait SuggestionProvider {
    fn suggestions(&self, context: &SuggestionContext<'_>, query: &str) -> Vec<Suggestion>;
}
pub struct LocalSuggestions;

impl SuggestionProvider for LocalSuggestions {
    fn suggestions(&self, context: &SuggestionContext<'_>, query: &str) -> Vec<Suggestion> {
        thread_local! {
            static MATCHER: std::cell::RefCell<Matcher> =
                std::cell::RefCell::new(Matcher::new(Config::DEFAULT));
        }
        MATCHER.with(|cell| {
            let mut matcher = cell.borrow_mut();
            let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
            let lower_query = query.to_lowercase();
            let mut buffer = Vec::new();
            let mut candidates = Vec::new();
            let mut add = |suggestion: Suggestion, recency: i64, count: u64| {
                if classify_command(&suggestion.text) != CommandSafety::Safe {
                    return;
                }
                let text = suggestion.text.to_lowercase();
                let tier = if text.starts_with(&lower_query) {
                    0
                } else if text.split_whitespace().any(|w| w.starts_with(&lower_query)) {
                    1
                } else {
                    2
                };
                let mut score = if query.is_empty() { Some(0) } else { None };
                for field in [
                    Some(suggestion.text.as_str()),
                    Some(suggestion.title.as_str()),
                    suggestion.detail.as_deref(),
                ]
                .into_iter()
                .flatten()
                {
                    buffer.clear();
                    if let Some(value) =
                        pattern.score(Utf32Str::new(field, &mut buffer), &mut matcher)
                    {
                        score = Some(score.map_or(value, |previous: u32| previous.max(value)));
                    }
                }
                if let Some(score) = score {
                    candidates.push((suggestion, tier, score, recency, count));
                }
            };
            for (entries, source) in [
                (context.session_history, SuggestionSource::SessionHistory),
                (context.host_history, SuggestionSource::HostHistory),
            ] {
                for entry in entries {
                    add(
                        Suggestion {
                            text: entry.command.clone(),
                            title: entry.command.clone(),
                            source,
                            detail: None,
                        },
                        entry.last_used,
                        entry.use_count,
                    );
                }
            }
            for snippet in context.snippets {
                let detail = format!(
                    "{} {}",
                    snippet.tags.join(" "),
                    snippet.description.as_deref().unwrap_or("")
                );
                add(
                    Suggestion {
                        text: snippet.command.clone(),
                        title: snippet.name.clone(),
                        source: SuggestionSource::Snippet,
                        detail: Some(detail),
                    },
                    snippet.updated_at,
                    0,
                );
            }
            candidates.sort_by(|a, b| {
                a.1.cmp(&b.1)
                    .then_with(|| b.2.cmp(&a.2))
                    .then_with(|| a.0.source.cmp(&b.0.source))
                    .then_with(|| b.3.cmp(&a.3))
                    .then_with(|| b.4.cmp(&a.4))
                    .then_with(|| a.0.text.cmp(&b.0.text))
            });
            // Trimming only: collapsing interior whitespace merges distinct quoted commands.
            let mut seen = std::collections::HashSet::new();
            candidates
                .into_iter()
                .filter_map(|(s, _, _, _, _)| seen.insert(s.text.trim().to_owned()).then_some(s))
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(text: &str, time: i64) -> HistoryEntry {
        HistoryEntry {
            command: text.into(),
            last_used: time,
            use_count: 1,
        }
    }
    #[test]
    fn provider_ranks_deduplicates_and_rejects_sensitive_sources() {
        let session = [
            entry("docker ps", 1),
            entry("echo docker", 2),
            entry("curl --password bad", 3),
        ];
        let host = [entry("docker ps", 99), entry("docker logs", 4)];
        let snippets = [Snippet {
            id: 1,
            name: "danger".into(),
            command: "sshpass -p bad ssh host".into(),
            tags: vec![],
            description: None,
            created_at: 0,
            updated_at: 0,
        }];
        let context = SuggestionContext {
            session_history: &session,
            host_history: &host,
            snippets: &snippets,
        };
        let result = LocalSuggestions.suggestions(&context, "dock");
        assert_eq!(
            result.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
            ["docker ps", "docker logs", "echo docker"]
        );
        assert_eq!(result[0].source, SuggestionSource::SessionHistory);
        assert_eq!(LocalSuggestions.suggestions(&context, "").len(), 3);
    }
}
