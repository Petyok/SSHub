use super::*;

impl App {
    /// Map a Y offset (relative to hosts panel content area) to a host index,
    /// accounting for group headers and blank separators.
    /// Flattened host-tree layout: total visual rows (group headers + blank
    /// separators + host rows) and the visual row of the selected host.
    pub fn host_visual_layout(&self) -> (usize, Option<usize>) {
        let rows = self.host_visual_rows();
        let sel = rows.iter().position(|r| {
            matches!(
                r,
                VisualRow::Header { selected: true, .. } | VisualRow::Host { selected: true, .. }
            )
        });
        (rows.len(), sel)
    }

    /// Scroll offset (in visual rows) for a host panel of `body_h` rows that
    /// keeps the selected host roughly centered and on screen.
    pub fn host_scroll_offset(&self, body_h: usize) -> usize {
        if body_h == 0 {
            return 0;
        }
        let (total, sel) = self.host_visual_layout();
        let max_offset = total.saturating_sub(body_h);
        match sel {
            Some(s) => s.saturating_sub(body_h / 2).min(max_offset),
            None => 0,
        }
    }

    /// Scroll offset, in whole card-rows, for the keys tab. Keeps the selected
    /// identity card on screen (roughly centered) when the grid overflows.
    /// `card_row_stride` is the height of one card row (card height + gap).
    pub fn keys_scroll_row_offset(
        &self,
        area_height: u16,
        cards_per_row: usize,
        card_row_stride: u16,
    ) -> usize {
        let cpr = cards_per_row.max(1);
        let stride = card_row_stride.max(1) as usize;
        let total_rows = self.identities.len().div_ceil(cpr);
        let visible_rows = ((area_height as usize) / stride).max(1);
        let selected_row = self.identity_selected / cpr;
        let max_off = total_rows.saturating_sub(visible_rows);
        selected_row.saturating_sub(visible_rows / 2).min(max_off)
    }

    /// Map a click at visible row `rel_y` (within a `body_h`-row panel) to the
    /// host index under it, accounting for the current scroll offset.
    pub(crate) fn host_row_to_index(&self, rel_y: u16, body_h: usize) -> Option<usize> {
        let target = rel_y as usize + self.host_scroll_offset(body_h);
        match self.host_visual_rows().get(target) {
            Some(VisualRow::Host { host_idx, .. }) => Some(*host_idx),
            _ => None,
        }
    }

    /// Map a click at visible row `rel_y` to a group-header section index (for
    /// click-to-collapse), accounting for the current scroll offset.
    pub(crate) fn host_row_to_header(&self, rel_y: u16, body_h: usize) -> Option<usize> {
        let target = rel_y as usize + self.host_scroll_offset(body_h);
        match self.host_visual_rows().get(target) {
            Some(VisualRow::Header { section, .. }) => Some(*section),
            _ => None,
        }
    }

    pub fn selected_host_index(&self) -> Option<usize> {
        match self.nav_rows.get(self.selected) {
            Some(NavRow::Host(i)) => Some(*i),
            _ => None,
        }
    }

    /// The full rendered layout of the hosts tree: blank separators, group
    /// headers and host rows, with per-row selection state. Single source of
    /// truth shared by rendering, scroll math and click mapping.
    pub fn host_visual_rows(&self) -> Vec<VisualRow> {
        // Driven by `nav_rows` (the single source of truth for what's visible
        // and navigable), so hidden subtrees never leak host rows. Blank
        // separators go before each top-level header (except the first row);
        // nested headers sit flush under their parent. `depth` drives indent.
        let mut rows = Vec::new();
        let mut cur_host_depth = 1usize;
        let mut first = true;
        for (nav_idx, row) in self.nav_rows.iter().enumerate() {
            match *row {
                NavRow::Header(si) => {
                    let section = &self.group_sections[si];
                    if !first && section.depth == 0 {
                        rows.push(VisualRow::Blank);
                    }
                    rows.push(VisualRow::Header {
                        section: si,
                        collapsed: section.collapsed,
                        selected: self.selected == nav_idx,
                        depth: section.depth,
                    });
                    cur_host_depth = section.depth + 1;
                }
                NavRow::Host(host_idx) => {
                    rows.push(VisualRow::Host {
                        host_idx,
                        selected: self.selected == nav_idx,
                        // Flat (no-group) lists have no headers → depth 0.
                        depth: if self.groups.is_empty() { 0 } else { cur_host_depth },
                    });
                }
            }
            first = false;
        }
        rows
    }

    pub fn selected_entry(&self) -> Option<&HostEntry> {
        let host_idx = self.selected_host_index()?;
        self.hosts.get(host_idx)
    }

    /// The section index if the current selection is a group header.
    pub fn selected_nav_header(&self) -> Option<usize> {
        match self.nav_rows.get(self.selected) {
            Some(NavRow::Header(si)) => Some(*si),
            _ => None,
        }
    }

    pub(crate) fn load_collapsed_groups(&mut self) {
        if let Ok(Some(raw)) = self.store.get_ui_state("collapsed_groups") {
            if let Ok(ids) = serde_json::from_str::<Vec<i64>>(&raw) {
                self.collapsed_groups = ids.into_iter().collect();
            }
        }
    }

    pub(crate) fn persist_collapsed_groups(&self) {
        let mut ids: Vec<i64> = self.collapsed_groups.iter().copied().collect();
        ids.sort_unstable();
        if let Ok(json) = serde_json::to_string(&ids) {
            let _ = self.store.set_ui_state("collapsed_groups", &json);
        }
    }

    /// Toggle collapse of the group header under the selection, keeping the
    /// selection on that header, and persist the new state.
    pub(crate) fn toggle_selected_group(&mut self) {
        if let Some(si) = self.selected_nav_header() {
            self.toggle_group_by_section(si);
        }
    }

    pub(crate) fn toggle_group_by_section(&mut self, si: usize) {
        let Some(section) = self.group_sections.get(si) else {
            return;
        };
        let key = section.key();
        if !self.collapsed_groups.remove(&key) {
            self.collapsed_groups.insert(key);
        }
        self.persist_collapsed_groups();
        self.rebuild_filter();
        if let Some(pos) = self
            .nav_rows
            .iter()
            .position(|r| matches!(r, NavRow::Header(s) if self.group_sections[*s].key() == key))
        {
            self.selected = pos;
        }
    }

    /// Collapse (`false`) or expand (`true`) every group at once.
    pub(crate) fn set_all_groups_collapsed(&mut self, collapsed: bool) {
        if collapsed {
            self.collapsed_groups = self.group_sections.iter().map(|s| s.key()).collect();
        } else {
            self.collapsed_groups.clear();
        }
        self.persist_collapsed_groups();
        let sel_key = self
            .selected_nav_header()
            .map(|si| self.group_sections[si].key());
        self.rebuild_filter();
        if let Some(key) = sel_key {
            if let Some(pos) = self.nav_rows.iter().position(
                |r| matches!(r, NavRow::Header(s) if self.group_sections[*s].key() == key),
            ) {
                self.selected = pos;
            }
        }
    }

    pub(crate) fn toggle_favorite(&mut self) -> Result<()> {
        let Some(host_idx) = self.selected_host_index() else {
            return Ok(());
        };

        if let HostEntry::Managed(m) = &self.hosts[host_idx] {
            let id = m.id;
            let new_fav = !m.favorite;
            self.store.update_host(
                id,
                &HostUpdate {
                    favorite: Some(new_fav),
                    ..Default::default()
                },
            )?;
            if let HostEntry::Managed(m) = &mut self.hosts[host_idx] {
                m.favorite = new_fav;
            }
            return Ok(());
        }

        let host_name = self.hosts[host_idx].name().to_string();
        self.metadata.toggle_favorite(&host_name)?;
        if let Some((_, meta)) = self.hosts[host_idx].legacy_mut() {
            if let Some(stored) = self.metadata.get(&host_name)? {
                meta.favorite = stored.favorite;
            }
        }
        Ok(())
    }

    pub fn cycle_sort_mode(&mut self) {
        self.sort_mode = self.sort_mode.next();
        self.rebuild_filter();
    }

    pub(crate) fn move_host_manual(&mut self, delta: i32) -> Result<()> {
        if self.sort_mode != SortMode::Manual {
            return Ok(());
        }
        let Some(id) = self.selected_entry().and_then(|e| e.managed_id()) else {
            return Ok(());
        };
        let name = self.selected_entry().map(|e| e.name().to_string());
        // Find the adjacent *host* nav row in the requested direction (skip
        // group headers so manual reorder only swaps hosts).
        let mut probe = self.selected as i32 + delta;
        let other_idx = loop {
            if probe < 0 || probe >= self.nav_rows.len() as i32 {
                return Ok(());
            }
            match self.nav_rows[probe as usize] {
                NavRow::Host(i) => break i,
                NavRow::Header(_) => probe += delta,
            }
        };
        let Some(other_id) = self.hosts[other_idx].managed_id() else {
            return Ok(());
        };

        self.store.swap_host_sort_orders(id, other_id)?;
        self.reload_hosts()?;
        if let Some(name) = name {
            self.restore_selection_by_name(&name);
        }
        Ok(())
    }

    pub(crate) fn rebuild_filter(&mut self) {
        let candidates: Vec<usize> = if self.tag_filters.is_empty() {
            (0..self.hosts.len()).collect()
        } else {
            self.hosts
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    let tags = entry.tags();
                    // AND: the host must carry every selected tag.
                    self.tag_filters.iter().all(|f| tags.iter().any(|t| t == f))
                })
                .map(|(idx, _)| idx)
                .collect()
        };

        let entries: Vec<HostEntry> = candidates
            .iter()
            .map(|&idx| self.hosts[idx].clone())
            .collect();
        let local_matches = self.search.update_query(&entries, &self.search_query);
        let mut filtered: Vec<usize> = local_matches
            .into_iter()
            .map(|local_idx| candidates[local_idx])
            .collect();

        sort_host_indices(&self.hosts, &mut filtered, self.sort_mode);
        // Partition by group, then flatten back so filtered_indices walks in
        // visual order. Within each section the existing sort_mode order is
        // preserved by build_group_sections. Without this, j/k steps through
        // the alphabetical list while the screen shows grouped sections, so
        // moving past a grouped host visually "teleports" to the group at the
        // top of the list and back.
        self.group_sections = build_group_sections(&self.hosts, &self.groups, &filtered);
        // While a filter is active, drop groups whose whole subtree has no
        // matching hosts — but keep a parent that itself is empty when a
        // descendant still matches, so nested results stay reachable.
        let filtering = !self.tag_filters.is_empty() || !self.search_query.is_empty();
        if filtering {
            let keep = subtree_has_hosts(&self.group_sections);
            let mut it = keep.into_iter();
            self.group_sections.retain(|_| it.next().unwrap_or(false));
        }
        self.filtered_indices = self
            .group_sections
            .iter()
            .flat_map(|s| s.host_indices.iter().copied())
            .collect();

        // Tree mode (navigable, collapsible headers) kicks in only once the
        // user has real groups — a pure ssh_config list stays a flat host list.
        // Collapsing a group hides its hosts AND its whole descendant subtree;
        // `hidden_below` tracks the depth of the nearest collapsed ancestor.
        let tree_mode = !self.groups.is_empty();
        let mut nav = Vec::new();
        let mut hidden_below: Option<usize> = None;
        for (si, section) in self.group_sections.iter_mut().enumerate() {
            let depth = section.depth;
            if let Some(cd) = hidden_below {
                if depth <= cd {
                    hidden_below = None; // left the collapsed subtree
                }
            }
            section.collapsed = tree_mode && self.collapsed_groups.contains(&section.key());
            if hidden_below.is_some() {
                continue; // an ancestor is collapsed: skip header and hosts
            }
            if tree_mode {
                nav.push(NavRow::Header(si));
            }
            if section.collapsed {
                hidden_below = Some(depth);
            } else {
                nav.extend(section.host_indices.iter().map(|&h| NavRow::Host(h)));
            }
        }
        self.nav_rows = nav;
        self.clamp_selected();
    }

    pub(crate) fn clamp_selected(&mut self) {
        if self.nav_rows.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.nav_rows.len() {
            self.selected = self.nav_rows.len() - 1;
        }
    }

    /// Make the host at `idx` (an index into [`App::hosts`]) the current
    /// selection: drop any tag/search filter that hides it, expand its group if
    /// collapsed, and point `selected` at its navigation row. Returns whether
    /// the host is now selectable (found in `nav_rows`).
    ///
    /// Used by quick-connect, where the fuzzy palette can pick a host that the
    /// current filter or a collapsed group would otherwise hide.
    pub fn reveal_host(&mut self, idx: usize) -> bool {
        if idx >= self.hosts.len() {
            return false;
        }
        // A tag/search filter may hide the chosen host — drop it rather than
        // silently landing on a different row.
        if !self.filtered_indices.contains(&idx) {
            self.tag_filters.clear();
            self.search_query.clear();
            self.rebuild_filter();
        }
        // Expand the host's group AND every ancestor group so its row is
        // navigable (a collapsed ancestor hides the whole subtree).
        let mut changed = false;
        let mut group = self.hosts.get(idx).and_then(|h| h.group_id());
        if group.is_none() {
            changed |= self.collapsed_groups.remove(&UNGROUPED_KEY);
        }
        while let Some(gid) = group {
            changed |= self.collapsed_groups.remove(&gid);
            group = self.groups.iter().find(|g| g.id == gid).and_then(|g| g.parent_id);
        }
        if changed {
            self.persist_collapsed_groups();
            self.rebuild_filter();
        }
        if let Some(pos) = self
            .nav_rows
            .iter()
            .position(|r| matches!(r, NavRow::Host(i) if *i == idx))
        {
            self.selected = pos;
            true
        } else {
            false
        }
    }

    pub(crate) fn restore_selection_by_name(&mut self, name: &str) {
        let host_idx = self.hosts.iter().position(|h| h.name() == name);
        if let Some(hi) = host_idx {
            if let Some(pos) = self
                .nav_rows
                .iter()
                .position(|r| matches!(r, NavRow::Host(i) if *i == hi))
            {
                self.selected = pos;
                return;
            }
        }
        self.clamp_selected();
    }
}
