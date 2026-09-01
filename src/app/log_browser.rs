use super::*;

use crate::log_browser as lb;
use crate::store::{LogBookmark, NewLogBookmark};
use std::path::PathBuf;

/// Lines moved per PageUp / PageDown in the viewer.
const VIEWER_PAGE: usize = 20;

impl LogBrowserState {
    /// Bookmarks belonging to the host currently open, newest first.
    fn host_bookmarks(&self) -> Vec<&LogBookmark> {
        match &self.current_host {
            Some(host) => self
                .bookmarks
                .iter()
                .filter(|b| &b.host_dir == host)
                .collect(),
            None => Vec::new(),
        }
    }
}

impl App {
    /// Open the log browser against the profile's real `logs/` dir.
    pub(crate) fn open_log_browser(&mut self) -> Result<()> {
        let root = self
            .runtime_data_dir()
            .map(|d| lb::logs_root(&d))
            .unwrap_or_else(|| PathBuf::from("logs"));
        self.open_log_browser_at(root)
    }

    /// Open the log browser against an explicit `logs/` root. Tests point this at
    /// a temp dir so they never read the real user data dir.
    pub(crate) fn open_log_browser_at(&mut self, logs_root: PathBuf) -> Result<()> {
        let hosts = lb::list_log_hosts(&logs_root);
        let bookmarks = self.store.list_log_bookmarks().unwrap_or_default();
        self.log_browser = Some(LogBrowserState {
            view: LogBrowserView::Hosts,
            logs_root,
            hosts,
            host_sel: 0,
            current_host: None,
            segments: Vec::new(),
            seg_sel: 0,
            current_seg: None,
            lines: Vec::new(),
            truncated: false,
            scroll: 0,
            query: String::new(),
            searching: false,
            matches: Vec::new(),
            match_idx: 0,
            naming: None,
            show_bookmarks: false,
            bookmark_sel: 0,
            bookmarks,
            notice: None,
        });
        self.mode = AppMode::LogBrowser;
        Ok(())
    }

    pub(crate) fn handle_key_log_browser(&mut self, key: KeyEvent) -> Result<()> {
        let Some(view) = self.log_browser.as_ref().map(|s| s.view) else {
            self.mode = AppMode::Normal;
            return Ok(());
        };
        match view {
            LogBrowserView::Hosts => self.log_key_hosts(key),
            LogBrowserView::Segments => self.log_key_segments(key),
            LogBrowserView::Viewer => self.log_key_viewer(key),
        }
    }

    // ── Hosts pane ──────────────────────────────────────────────

    fn log_key_hosts(&mut self, key: KeyEvent) -> Result<()> {
        if self.is_action(KeyAction::Quit, &key) {
            self.request_quit();
            return Ok(());
        }
        if self.is_action(KeyAction::Cancel, &key) || self.is_action(KeyAction::TabHosts, &key) {
            self.log_browser = None;
            self.mode = AppMode::Normal;
            return Ok(());
        }
        if self.is_action(KeyAction::MoveDown, &key) {
            if let Some(s) = self.log_browser.as_mut() {
                if !s.hosts.is_empty() {
                    s.host_sel = (s.host_sel + 1).min(s.hosts.len() - 1);
                }
            }
            return Ok(());
        }
        if self.is_action(KeyAction::MoveUp, &key) {
            if let Some(s) = self.log_browser.as_mut() {
                s.host_sel = s.host_sel.saturating_sub(1);
            }
            return Ok(());
        }
        if key.code == KeyCode::Enter {
            self.log_open_selected_host();
        }
        Ok(())
    }

    fn log_open_selected_host(&mut self) {
        let Some(s) = self.log_browser.as_mut() else {
            return;
        };
        let Some(host) = s.hosts.get(s.host_sel) else {
            return;
        };
        let dir = host.dir_name.clone();
        let segments = lb::list_segments(&s.logs_root.join(&dir));
        s.current_host = Some(dir);
        s.segments = segments;
        s.seg_sel = 0;
        s.view = LogBrowserView::Segments;
    }

    // ── Segments pane ───────────────────────────────────────────

    fn log_key_segments(&mut self, key: KeyEvent) -> Result<()> {
        if self.is_action(KeyAction::Quit, &key) {
            self.request_quit();
            return Ok(());
        }
        if self.is_action(KeyAction::Cancel, &key) {
            if let Some(s) = self.log_browser.as_mut() {
                s.view = LogBrowserView::Hosts;
            }
            return Ok(());
        }
        if self.is_action(KeyAction::MoveDown, &key) {
            if let Some(s) = self.log_browser.as_mut() {
                if !s.segments.is_empty() {
                    s.seg_sel = (s.seg_sel + 1).min(s.segments.len() - 1);
                }
            }
            return Ok(());
        }
        if self.is_action(KeyAction::MoveUp, &key) {
            if let Some(s) = self.log_browser.as_mut() {
                s.seg_sel = s.seg_sel.saturating_sub(1);
            }
            return Ok(());
        }
        if key.code == KeyCode::Enter {
            self.log_open_selected_segment();
        }
        Ok(())
    }

    fn log_open_selected_segment(&mut self) {
        let Some(s) = self.log_browser.as_mut() else {
            return;
        };
        let Some(seg) = s.segments.get(s.seg_sel) else {
            return;
        };
        let (lines, truncated) = lb::read_segment_lines(&seg.path, lb::VIEWER_READ_CAP);
        s.current_seg = Some(seg.file_name.clone());
        s.lines = lines;
        s.truncated = truncated;
        s.scroll = 0;
        s.query.clear();
        s.matches.clear();
        s.match_idx = 0;
        s.searching = false;
        s.naming = None;
        s.show_bookmarks = false;
        s.notice = None;
        s.view = LogBrowserView::Viewer;
    }

    // ── Viewer pane ─────────────────────────────────────────────

    fn log_key_viewer(&mut self, key: KeyEvent) -> Result<()> {
        // Sub-modes claim keys first.
        if self
            .log_browser
            .as_ref()
            .is_some_and(|s| s.naming.is_some())
        {
            return self.log_key_naming(key);
        }
        if self.log_browser.as_ref().is_some_and(|s| s.searching) {
            return self.log_key_searching(key);
        }
        if self.log_browser.as_ref().is_some_and(|s| s.show_bookmarks) {
            return self.log_key_bookmarks(key);
        }

        if self.is_action(KeyAction::Quit, &key) {
            self.request_quit();
            return Ok(());
        }
        if self.is_action(KeyAction::Cancel, &key) {
            if let Some(s) = self.log_browser.as_mut() {
                s.view = LogBrowserView::Segments;
                s.notice = None;
            }
            return Ok(());
        }
        if self.is_action(KeyAction::MoveDown, &key) {
            self.log_scroll(1);
            return Ok(());
        }
        if self.is_action(KeyAction::MoveUp, &key) {
            self.log_scroll(-1);
            return Ok(());
        }
        match key.code {
            KeyCode::PageDown => self.log_scroll(VIEWER_PAGE as isize),
            KeyCode::PageUp => self.log_scroll(-(VIEWER_PAGE as isize)),
            KeyCode::Home => {
                if let Some(s) = self.log_browser.as_mut() {
                    s.scroll = 0;
                }
            }
            KeyCode::End => {
                if let Some(s) = self.log_browser.as_mut() {
                    s.scroll = s.lines.len().saturating_sub(1);
                }
            }
            KeyCode::Char('/') => {
                if let Some(s) = self.log_browser.as_mut() {
                    s.searching = true;
                    s.query.clear();
                    s.notice = None;
                }
            }
            KeyCode::Char('n') => self.log_jump_match(1),
            KeyCode::Char('N') => self.log_jump_match(-1),
            KeyCode::Char('b') => self.log_start_bookmark(),
            KeyCode::Char('m') => {
                if let Some(s) = self.log_browser.as_mut() {
                    s.show_bookmarks = !s.show_bookmarks;
                    s.bookmark_sel = 0;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn log_scroll(&mut self, delta: isize) {
        if let Some(s) = self.log_browser.as_mut() {
            let max = s.lines.len().saturating_sub(1) as isize;
            s.scroll = (s.scroll as isize + delta).clamp(0, max.max(0)) as usize;
        }
    }

    fn log_jump_match(&mut self, dir: isize) {
        if let Some(s) = self.log_browser.as_mut() {
            if s.matches.is_empty() {
                s.notice = Some("No matches; press / to search".into());
                return;
            }
            let len = s.matches.len() as isize;
            s.match_idx = (s.match_idx as isize + dir).rem_euclid(len) as usize;
            s.scroll = s.matches[s.match_idx];
            s.notice = Some(format!("Match {}/{}", s.match_idx + 1, s.matches.len()));
        }
    }

    fn log_start_bookmark(&mut self) {
        if let Some(s) = self.log_browser.as_mut() {
            if s.current_seg.is_none() {
                return;
            }
            // Start blank; an empty name falls back to "line N" on save.
            s.naming = Some(String::new());
            s.notice = None;
        }
    }

    // ── Viewer sub-mode: naming a bookmark ──────────────────────

    fn log_key_naming(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                if let Some(s) = self.log_browser.as_mut() {
                    s.naming = None;
                }
            }
            KeyCode::Enter => self.log_save_bookmark()?,
            KeyCode::Backspace => {
                if let Some(s) = self.log_browser.as_mut() {
                    if let Some(name) = s.naming.as_mut() {
                        name.pop();
                    }
                }
            }
            KeyCode::Char(c)
                if !c.is_control()
                    && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) =>
            {
                if let Some(s) = self.log_browser.as_mut() {
                    if let Some(name) = s.naming.as_mut() {
                        name.push(c);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn log_save_bookmark(&mut self) -> Result<()> {
        let data = self.log_browser.as_ref().and_then(|s| {
            match (
                s.current_host.clone(),
                s.current_seg.clone(),
                s.naming.clone(),
            ) {
                (Some(host), Some(file), Some(name)) => {
                    Some((host, file, name.trim().to_string(), s.scroll as i64))
                }
                _ => None,
            }
        });
        let Some((host_dir, file_name, name, line)) = data else {
            if let Some(s) = self.log_browser.as_mut() {
                s.naming = None;
            }
            return Ok(());
        };
        // A blank name is fine: fall back to the line number so a quick
        // press-b-then-Enter still leaves a usable bookmark.
        let name = if name.is_empty() {
            format!("line {}", line + 1)
        } else {
            name
        };

        self.store.create_log_bookmark(&NewLogBookmark {
            host_dir,
            file_name,
            line,
            name: name.clone(),
        })?;
        let bookmarks = self.store.list_log_bookmarks().unwrap_or_default();
        if let Some(s) = self.log_browser.as_mut() {
            s.bookmarks = bookmarks;
            s.naming = None;
            s.notice = Some(format!("Bookmarked line {} as '{}'", line + 1, name));
        }
        Ok(())
    }

    // ── Viewer sub-mode: search ─────────────────────────────────

    fn log_key_searching(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                if let Some(s) = self.log_browser.as_mut() {
                    s.searching = false;
                }
            }
            KeyCode::Enter => self.log_commit_search(),
            KeyCode::Backspace => {
                if let Some(s) = self.log_browser.as_mut() {
                    s.query.pop();
                }
            }
            KeyCode::Char(c)
                if !c.is_control()
                    && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) =>
            {
                if let Some(s) = self.log_browser.as_mut() {
                    s.query.push(c);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn log_commit_search(&mut self) {
        if let Some(s) = self.log_browser.as_mut() {
            s.matches = lb::search_lines(&s.lines, &s.query);
            s.match_idx = 0;
            s.searching = false;
            match s.matches.first() {
                Some(&first) => {
                    s.scroll = first;
                    s.notice = Some(format!("{} match(es); n / N to move", s.matches.len()));
                }
                None => s.notice = Some("No matches".into()),
            }
        }
    }

    // ── Viewer sub-mode: bookmarks list ─────────────────────────

    fn log_key_bookmarks(&mut self, key: KeyEvent) -> Result<()> {
        if self.is_action(KeyAction::Cancel, &key) {
            if let Some(s) = self.log_browser.as_mut() {
                s.show_bookmarks = false;
            }
            return Ok(());
        }
        if self.is_action(KeyAction::MoveDown, &key) {
            if let Some(s) = self.log_browser.as_mut() {
                let n = s.host_bookmarks().len();
                if n > 0 {
                    s.bookmark_sel = (s.bookmark_sel + 1).min(n - 1);
                }
            }
            return Ok(());
        }
        if self.is_action(KeyAction::MoveUp, &key) {
            if let Some(s) = self.log_browser.as_mut() {
                s.bookmark_sel = s.bookmark_sel.saturating_sub(1);
            }
            return Ok(());
        }
        match key.code {
            KeyCode::Enter => self.log_jump_bookmark(),
            KeyCode::Char('d') => self.log_delete_bookmark()?,
            _ => {}
        }
        Ok(())
    }

    fn log_jump_bookmark(&mut self) {
        let target = {
            let Some(s) = self.log_browser.as_ref() else {
                return;
            };
            s.host_bookmarks()
                .get(s.bookmark_sel)
                .map(|b| (b.file_name.clone(), b.line as usize))
        };
        let Some((file_name, line)) = target else {
            return;
        };

        // A bookmark can point at a different segment of the same host; load it.
        let need_load = self
            .log_browser
            .as_ref()
            .map(|s| s.current_seg.as_deref() != Some(file_name.as_str()))
            .unwrap_or(false);
        if need_load {
            let seg_path = self.log_browser.as_ref().and_then(|s| {
                s.current_host
                    .as_ref()
                    .map(|h| s.logs_root.join(h).join(&file_name))
            });
            if let Some(path) = seg_path {
                let (lines, truncated) = lb::read_segment_lines(&path, lb::VIEWER_READ_CAP);
                if let Some(s) = self.log_browser.as_mut() {
                    s.lines = lines;
                    s.truncated = truncated;
                    s.current_seg = Some(file_name.clone());
                    s.matches.clear();
                    s.query.clear();
                    s.match_idx = 0;
                    s.seg_sel = s
                        .segments
                        .iter()
                        .position(|sg| sg.file_name == file_name)
                        .unwrap_or(s.seg_sel);
                }
            }
        }
        if let Some(s) = self.log_browser.as_mut() {
            let max = s.lines.len().saturating_sub(1);
            s.scroll = line.min(max);
            s.show_bookmarks = false;
            s.notice = Some(format!("Jumped to line {}", s.scroll + 1));
        }
    }

    fn log_delete_bookmark(&mut self) -> Result<()> {
        let id = self
            .log_browser
            .as_ref()
            .and_then(|s| s.host_bookmarks().get(s.bookmark_sel).map(|b| b.id));
        let Some(id) = id else {
            return Ok(());
        };
        self.store.delete_log_bookmark(id)?;
        let bookmarks = self.store.list_log_bookmarks().unwrap_or_default();
        if let Some(s) = self.log_browser.as_mut() {
            s.bookmarks = bookmarks;
            let n = s.host_bookmarks().len();
            s.bookmark_sel = s.bookmark_sel.min(n.saturating_sub(1));
            s.notice = Some("Bookmark deleted".into());
        }
        Ok(())
    }
}
