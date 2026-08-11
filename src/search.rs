use crate::app::HostEntry;
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};

/// Fuzzy filter over host entries backed by nucleo.
#[derive(Debug)]
pub struct HostSearch {
    matcher: Matcher,
}

impl Default for HostSearch {
    fn default() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
        }
    }
}

impl HostSearch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update_query(&mut self, entries: &[HostEntry], query: &str) -> Vec<usize> {
        if query.is_empty() {
            return (0..entries.len()).collect();
        }

        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();
        let mut scored = Vec::new();

        for (idx, entry) in entries.iter().enumerate() {
            if let Some(score) = score_entry(&pattern, &mut self.matcher, entry, &mut buf) {
                scored.push((score, idx));
            }
            buf.clear();
        }

        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, idx)| idx).collect()
    }

    /// Char indices of `query` inside `field`, ascending and de-duplicated.
    ///
    /// Same pattern parse and same matcher the filter itself uses, so what the
    /// host list highlights is exactly what made the row match — not a second,
    /// subtly different substring search. Empty when the query is empty or this
    /// particular field does not match (a row can be in the filter on a tag or
    /// description while its name matched nothing).
    pub fn display_match_indices(&mut self, field: &str, query: &str) -> Vec<u32> {
        if query.is_empty() || field.is_empty() {
            return Vec::new();
        }
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();
        let mut indices = Vec::new();
        if pattern
            .indices(
                Utf32Str::new(field, &mut buf),
                &mut self.matcher,
                &mut indices,
            )
            .is_none()
        {
            return Vec::new();
        }
        // `Pattern::indices` neither sorts nor de-duplicates across atoms.
        indices.sort_unstable();
        indices.dedup();
        indices
    }
}

fn score_field(
    pattern: &Pattern,
    matcher: &mut Matcher,
    field: &str,
    buf: &mut Vec<char>,
    best: &mut Option<u32>,
) {
    if field.is_empty() {
        return;
    }
    buf.clear();
    if let Some(score) = pattern.score(Utf32Str::new(field, buf), matcher) {
        *best = Some(best.map_or(score, |current| current.max(score)));
    }
}

fn score_entry(
    pattern: &Pattern,
    matcher: &mut Matcher,
    entry: &HostEntry,
    buf: &mut Vec<char>,
) -> Option<u32> {
    let mut best = None;
    score_field(pattern, matcher, entry.name(), buf, &mut best);
    score_field(pattern, matcher, entry.display_name(), buf, &mut best);
    let ssh = entry.ssh_host();
    if let Some(hostname) = ssh.hostname.as_deref() {
        score_field(pattern, matcher, hostname, buf, &mut best);
    }
    let tags = entry.tags().join(" ");
    score_field(pattern, matcher, &tags, buf, &mut best);
    if let Some(description) = entry.description() {
        score_field(pattern, matcher, description, buf, &mut best);
    }
    if let Some(environment) = entry.environment() {
        score_field(pattern, matcher, environment, buf, &mut best);
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::SshHost;

    fn fixture() -> Vec<HostEntry> {
        vec![
            host(
                "web-prod",
                Some("10.0.0.1"),
                &["prod", "web"],
                Some("Production web tier"),
                Some("production"),
            ),
            host(
                "db-staging",
                Some("10.0.0.2"),
                &["staging", "db"],
                Some("Staging database"),
                Some("staging"),
            ),
            host("bastion", Some("jump.example.com"), &["ops"], None, None),
            host(
                "dev-box",
                None,
                &["dev"],
                Some("Local dev machine"),
                Some("development"),
            ),
        ]
    }

    fn host(
        name: &str,
        hostname: Option<&str>,
        tags: &[&str],
        description: Option<&str>,
        environment: Option<&str>,
    ) -> HostEntry {
        let mut entry = HostEntry::new(SshHost::new(name));
        if let HostEntry::Legacy { host, meta } = &mut entry {
            host.hostname = hostname.map(str::to_string);
            meta.tags = tags.iter().map(|tag| (*tag).to_string()).collect();
            meta.description = description.map(str::to_string);
            meta.environment = environment.map(str::to_string);
        }
        entry
    }

    #[test]
    fn empty_query_returns_all_indices_in_order() {
        let entries = fixture();
        let mut search = HostSearch::new();
        assert_eq!(search.update_query(&entries, ""), vec![0, 1, 2, 3]);
    }

    #[test]
    fn matches_host_name() {
        let entries = fixture();
        let mut search = HostSearch::new();
        assert_eq!(search.update_query(&entries, "bastion"), vec![2]);
    }

    #[test]
    fn matches_hostname() {
        let entries = fixture();
        let mut search = HostSearch::new();
        assert_eq!(search.update_query(&entries, "jump.example"), vec![2]);
    }

    #[test]
    fn matches_joined_tags() {
        let entries = fixture();
        let mut search = HostSearch::new();
        assert_eq!(search.update_query(&entries, "staging"), vec![1]);
        assert_eq!(search.update_query(&entries, "ops"), vec![2]);
    }

    #[test]
    fn matches_description() {
        let entries = fixture();
        let mut search = HostSearch::new();
        assert_eq!(search.update_query(&entries, "Local dev"), vec![3]);
    }

    #[test]
    fn matches_environment() {
        let entries = fixture();
        let mut search = HostSearch::new();
        assert_eq!(search.update_query(&entries, "development"), vec![3]);
    }

    #[test]
    fn smart_case_matching_ignores_case_for_lowercase_queries() {
        let entries = fixture();
        let mut search = HostSearch::new();
        assert_eq!(search.update_query(&entries, "web"), vec![0]);
        assert_eq!(search.update_query(&entries, "production"), vec![0]);
    }

    #[test]
    fn smart_case_matching_respects_uppercase_queries() {
        let entries = fixture();
        let mut search = HostSearch::new();
        assert!(search.update_query(&entries, "WEB").is_empty());
    }

    #[test]
    fn multi_token_query_requires_all_tokens() {
        let entries = fixture();
        let mut search = HostSearch::new();
        assert_eq!(search.update_query(&entries, "web prod"), vec![0]);
        assert!(search.update_query(&entries, "web staging").is_empty());
    }

    #[test]
    fn no_matches_returns_empty() {
        let entries = fixture();
        let mut search = HostSearch::new();
        assert!(search.update_query(&entries, "zzzznotfound").is_empty());
    }

    #[test]
    fn ranks_better_name_matches_first() {
        let entries = vec![
            host("web", None, &[], Some("misc web notes"), None),
            host("web-server", None, &[], None, None),
        ];
        let mut search = HostSearch::new();
        assert_eq!(search.update_query(&entries, "web"), vec![0, 1]);
    }

    #[test]
    fn stable_order_for_equal_scores() {
        let entries = vec![
            host("alpha-host", None, &[], None, None),
            host("beta-host", None, &[], None, None),
        ];
        let mut search = HostSearch::new();
        assert_eq!(search.update_query(&entries, "host"), vec![0, 1]);
    }

    /// The host list underlines what the query actually matched, so the search
    /// has to hand back *where* it matched in the string the list renders.
    #[test]
    fn display_match_indices_point_at_the_matched_characters() {
        let mut search = HostSearch::new();

        // A contiguous substring: the indices are the substring's own columns.
        assert_eq!(
            search.display_match_indices("web-prod", "prod"),
            vec![4, 5, 6, 7]
        );

        // A genuinely fuzzy match: scattered, still in ascending order.
        let scattered = search.display_match_indices("db-staging", "dbg");
        assert_eq!(scattered.first().copied(), Some(0));
        assert!(
            scattered.windows(2).all(|w| w[0] < w[1]),
            "indices must be ascending and unique: {scattered:?}"
        );
        assert!(scattered
            .iter()
            .all(|&i| (i as usize) < "db-staging".chars().count()));

        // Case-insensitive, like the filter itself.
        assert_eq!(
            search.display_match_indices("Bastion", "bas"),
            vec![0, 1, 2]
        );
    }

    /// No query, or a query that does not match this field, highlights nothing.
    #[test]
    fn display_match_indices_are_empty_without_a_match() {
        let mut search = HostSearch::new();
        assert!(search.display_match_indices("web-prod", "").is_empty());
        // The entry can still be in the filter via a tag or description; the
        // *name* simply has nothing to underline.
        assert!(search.display_match_indices("web-prod", "zzz").is_empty());
    }
}
