use super::*;

impl App {
    pub(crate) fn open_tag_filter(&mut self) {
        self.mode = AppMode::TagFilter;
        self.search_query.clear();
        self.tag_filter_selected = 0;
    }

    /// Rows shown in the tag-filter popup: `["(all)", <matching unique tags>]`,
    /// filtered by the typed query (case-insensitive substring). Currently
    /// selected tags always appear even if they no longer match the query, so
    /// they can be toggled back off.
    pub fn tag_filter_rows(&self) -> Vec<String> {
        let query = self.search_query.to_lowercase();
        let mut rows = vec!["(all)".to_string()];
        let mut tags: Vec<String> = self
            .hosts
            .iter()
            .flat_map(|entry| entry.tags().iter().cloned())
            .filter(|t| {
                query.is_empty()
                    || t.to_lowercase().contains(&query)
                    || self.tag_filters.contains(t)
            })
            .collect();
        tags.sort();
        tags.dedup();
        rows.extend(tags);
        rows
    }

    /// Whether `tag` is one of the active filters.
    pub fn is_tag_active(&self, tag: &str) -> bool {
        self.tag_filters.iter().any(|t| t == tag)
    }

    pub(crate) fn toggle_tag_filter(&mut self, tag: &str) {
        if let Some(pos) = self.tag_filters.iter().position(|t| t == tag) {
            self.tag_filters.remove(pos);
        } else {
            self.tag_filters.push(tag.to_string());
        }
        self.rebuild_filter();
    }

    /// Toggle the tag under the popup cursor, or clear all when on "(all)".
    pub(crate) fn toggle_highlighted_tag(&mut self) {
        let rows = self.tag_filter_rows();
        match rows.get(self.tag_filter_selected) {
            Some(_) if self.tag_filter_selected == 0 => {
                self.tag_filters.clear();
                self.rebuild_filter();
            }
            Some(tag) => {
                let tag = tag.clone();
                self.toggle_tag_filter(&tag);
            }
            None => {}
        }
    }

    pub(crate) fn handle_key_tag_filter(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            // Esc closes without changing the current selection (toggles made
            // with Space are already applied live).
            KeyCode::Esc => {
                self.search_query.clear();
                self.mode = AppMode::Normal;
                self.rebuild_filter();
            }
            // Enter confirms and closes. It never removes a tag: it clears all
            // on "(all)", adds the highlighted tag if it isn't active yet (the
            // fast single-pick path), and otherwise just closes.
            KeyCode::Enter => {
                let rows = self.tag_filter_rows();
                match rows.get(self.tag_filter_selected) {
                    Some(_) if self.tag_filter_selected == 0 => self.tag_filters.clear(),
                    Some(tag) if !self.is_tag_active(tag) => {
                        let tag = tag.clone();
                        self.tag_filters.push(tag);
                    }
                    _ => {}
                }
                self.search_query.clear();
                self.mode = AppMode::Normal;
                self.rebuild_filter();
            }
            // Space toggles the highlighted tag and keeps the menu open so the
            // user can select several tags in a row.
            KeyCode::Char(' ') => {
                self.toggle_highlighted_tag();
            }
            KeyCode::Down => {
                let len = self.tag_filter_rows().len();
                if len > 0 {
                    self.tag_filter_selected = (self.tag_filter_selected + 1) % len;
                }
            }
            KeyCode::Up => {
                let len = self.tag_filter_rows().len();
                if len > 0 {
                    self.tag_filter_selected = (self.tag_filter_selected + len - 1) % len;
                }
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.reset_tag_filter_selection();
            }
            KeyCode::Char(c) if key.modifiers.is_empty() && !c.is_control() => {
                self.search_query.push(c);
                self.reset_tag_filter_selection();
            }
            _ => {}
        }
        Ok(())
    }

    /// After the query changes, highlight the first matching tag (index 1) so
    /// Space toggles it. Falls back to "(all)" when nothing matches or the
    /// query is empty.
    pub(crate) fn reset_tag_filter_selection(&mut self) {
        let has_match = self.tag_filter_rows().len() > 1;
        self.tag_filter_selected = if !self.search_query.is_empty() && has_match {
            1
        } else {
            0
        };
    }
}
