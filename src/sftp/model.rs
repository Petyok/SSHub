//! Pure SFTP UI state model — NO I/O.
//!
//! Everything here is deterministic path/selection math so it can be unit
//! tested without a network, a filesystem, or a live SSH session. The worker
//! ([`super::worker`]) and transport ([`super::transport`]) own all I/O; this
//! module only decides *what* to do and computes the paths involved.

use std::path::PathBuf;

/// Which of the two panes a path/entry belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Remote,
    Local,
}

/// Direction of a queued transfer relative to the local machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Remote → local.
    Download,
    /// Local → remote.
    Upload,
}

/// One row in a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// A transfer staged in the queue but not yet run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedTransfer {
    pub direction: Direction,
    pub src: PathBuf,
    pub dst: PathBuf,
    pub name: String,
}

/// One browsable directory column (remote or local).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    pub cwd: PathBuf,
    pub entries: Vec<FileEntry>,
    pub selected: usize,
    pub filter: String,
}

impl Pane {
    fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            entries: Vec::new(),
            selected: 0,
            filter: String::new(),
        }
    }

    /// Indices into `entries` matching the current filter (all if the filter is empty).
    pub fn visible_indices(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.entries.len()).collect();
        }
        let needle = self.filter.to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.name.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect()
    }

    /// Number of entries currently visible under the filter.
    pub fn visible_len(&self) -> usize {
        self.visible_indices().len()
    }

    /// Set the filter text and move the cursor to the top of the filtered view.
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.selected = 0;
    }

    /// The entry under the cursor, if any.
    pub fn selected_entry(&self) -> Option<&FileEntry> {
        let idx = *self.visible_indices().get(self.selected)?;
        self.entries.get(idx)
    }

    /// Replace the listing and clamp the cursor to the new bounds.
    pub fn set_entries(&mut self, entries: Vec<FileEntry>) {
        self.entries = entries;
        self.filter = String::new();
        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        let len = self.visible_len();
        if len == 0 {
            self.selected = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }
}

/// Which pane currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Remote,
    Local,
}

/// Whether the browser is idle or a queue run is in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Browsing,
    Running,
}

/// Live progress for the transfer currently running out of the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// 0-based index of the running transfer within the queue snapshot.
    pub index: usize,
    /// Total number of transfers in the running queue.
    pub total: usize,
    /// Bytes moved so far for the current transfer.
    pub transferred: u64,
    /// Total size of the current transfer, if known.
    pub size: u64,
}

/// The whole SFTP tab state. Pure — mutated only through the helpers below and
/// by the app wiring that feeds in listings from the worker / local fs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpState {
    pub remote: Pane,
    pub local: Pane,
    pub queue: Vec<QueuedTransfer>,
    pub focus: Focus,
    pub phase: Phase,
    pub progress: Option<Progress>,
    pub notice: Option<String>,
    pub searching: bool,
}

impl SftpState {
    /// Fresh browsing state rooted at the given working directories.
    pub fn new(remote_cwd: impl Into<PathBuf>, local_cwd: impl Into<PathBuf>) -> Self {
        Self {
            remote: Pane::new(remote_cwd.into()),
            local: Pane::new(local_cwd.into()),
            queue: Vec::new(),
            focus: Focus::Remote,
            phase: Phase::Browsing,
            progress: None,
            notice: None,
            searching: false,
        }
    }

    /// Begin filtering the focused pane with a fresh, empty query.
    pub fn start_search(&mut self) {
        self.searching = true;
        self.focused_pane_mut().set_filter(String::new());
    }
    /// Append a char to the focused pane's filter.
    pub fn search_push(&mut self, c: char) {
        let mut f = self.focused_pane().filter.clone();
        f.push(c);
        self.focused_pane_mut().set_filter(f);
    }
    /// Delete the last char of the focused pane's filter.
    pub fn search_backspace(&mut self) {
        let mut f = self.focused_pane().filter.clone();
        f.pop();
        self.focused_pane_mut().set_filter(f);
    }
    /// Confirm: leave input mode but keep the filter applied.
    pub fn search_confirm(&mut self) {
        self.searching = false;
    }
    /// Cancel: clear the filter and leave input mode.
    pub fn search_cancel(&mut self) {
        self.searching = false;
        self.focused_pane_mut().set_filter(String::new());
    }

    /// Move the cursor in the focused pane by `delta`, clamped to `[0, len-1]`.
    pub fn move_selection(&mut self, delta: i64) {
        let pane = self.focused_pane_mut();
        let len = pane.visible_len();
        if len == 0 {
            pane.selected = 0;
            return;
        }
        let max = len as i64 - 1;
        pane.selected = (pane.selected as i64 + delta).clamp(0, max) as usize;
    }

    /// Swap keyboard focus between the two panes.
    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Remote => Focus::Local,
            Focus::Local => Focus::Remote,
        };
    }

    pub fn focused_side(&self) -> Side {
        match self.focus {
            Focus::Remote => Side::Remote,
            Focus::Local => Side::Local,
        }
    }

    pub fn focused_pane(&self) -> &Pane {
        match self.focus {
            Focus::Remote => &self.remote,
            Focus::Local => &self.local,
        }
    }

    pub fn focused_pane_mut(&mut self) -> &mut Pane {
        match self.focus {
            Focus::Remote => &mut self.remote,
            Focus::Local => &mut self.local,
        }
    }

    /// Compute the child path when descending into the focused pane's selected
    /// directory. Returns `(Side, new_cwd)` so the caller can trigger a fresh
    /// listing. Pure PathBuf math — no fs access. Returns `None` when the
    /// selection isn't a directory (files aren't enterable).
    pub fn enter_dir(&self) -> Option<(Side, PathBuf)> {
        let pane = self.focused_pane();
        let entry = pane.selected_entry()?;
        if !entry.is_dir {
            return None;
        }
        Some((self.focused_side(), pane.cwd.join(&entry.name)))
    }

    /// Compute the parent path of the focused pane's cwd. Returns
    /// `(Side, parent)` or `None` when already at the root. Pure PathBuf math.
    pub fn parent_dir(&self) -> Option<(Side, PathBuf)> {
        let pane = self.focused_pane();
        let parent = pane.cwd.parent()?;
        Some((self.focused_side(), parent.to_path_buf()))
    }

    /// Stage the focused-remote selection for download into `local.cwd`.
    /// Directories can't be staged in v1.
    pub fn stage_download(&mut self) -> Result<(), String> {
        let entry = self
            .remote
            .selected_entry()
            .cloned()
            .ok_or_else(|| "nothing selected".to_string())?;
        if entry.is_dir {
            let msg = "directories can't be staged in v1".to_string();
            self.notice = Some(msg.clone());
            return Err(msg);
        }
        let src = self.remote.cwd.join(&entry.name);
        let dst = self.local.cwd.join(&entry.name);
        if self.queue.iter().any(|q| q.src == src && q.dst == dst) {
            let msg = format!("{} is already queued", entry.name);
            self.notice = Some(msg.clone());
            return Err(msg);
        }
        self.queue.push(QueuedTransfer {
            direction: Direction::Download,
            src,
            dst,
            name: entry.name,
        });
        Ok(())
    }

    /// Stage the focused-local selection for upload into `remote.cwd`.
    /// Directories can't be staged in v1.
    pub fn stage_upload(&mut self) -> Result<(), String> {
        let entry = self
            .local
            .selected_entry()
            .cloned()
            .ok_or_else(|| "nothing selected".to_string())?;
        if entry.is_dir {
            let msg = "directories can't be staged in v1".to_string();
            self.notice = Some(msg.clone());
            return Err(msg);
        }
        let src = self.local.cwd.join(&entry.name);
        let dst = self.remote.cwd.join(&entry.name);
        if self.queue.iter().any(|q| q.src == src && q.dst == dst) {
            let msg = format!("{} is already queued", entry.name);
            self.notice = Some(msg.clone());
            return Err(msg);
        }
        self.queue.push(QueuedTransfer {
            direction: Direction::Upload,
            src,
            dst,
            name: entry.name,
        });
        Ok(())
    }

    /// Remove the queued transfer at `idx` (no-op if out of range).
    pub fn unstage(&mut self, idx: usize) {
        if idx < self.queue.len() {
            self.queue.remove(idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_entries() -> SftpState {
        let mut s = SftpState::new("/srv", "/home/me");
        s.remote.set_entries(vec![
            FileEntry {
                name: "docs".into(),
                is_dir: true,
                size: 0,
            },
            FileEntry {
                name: "a.txt".into(),
                is_dir: false,
                size: 10,
            },
        ]);
        s.local.set_entries(vec![
            FileEntry {
                name: "photos".into(),
                is_dir: true,
                size: 0,
            },
            FileEntry {
                name: "b.bin".into(),
                is_dir: false,
                size: 20,
            },
        ]);
        s
    }

    #[test]
    fn new_defaults() {
        let s = SftpState::new("/srv", "/home/me");
        assert_eq!(s.remote.cwd, PathBuf::from("/srv"));
        assert_eq!(s.local.cwd, PathBuf::from("/home/me"));
        assert_eq!(s.focus, Focus::Remote);
        assert_eq!(s.phase, Phase::Browsing);
        assert!(s.queue.is_empty());
        assert!(s.progress.is_none());
    }

    #[test]
    fn move_selection_clamps() {
        let mut s = state_with_entries();
        s.move_selection(-5);
        assert_eq!(s.remote.selected, 0);
        s.move_selection(1);
        assert_eq!(s.remote.selected, 1);
        s.move_selection(10);
        assert_eq!(s.remote.selected, 1); // clamped to len-1
    }

    #[test]
    fn move_selection_empty_pane() {
        let mut s = SftpState::new("/", "/");
        s.move_selection(3);
        assert_eq!(s.remote.selected, 0);
    }

    #[test]
    fn toggle_focus_flips() {
        let mut s = SftpState::new("/", "/");
        assert_eq!(s.focus, Focus::Remote);
        s.toggle_focus();
        assert_eq!(s.focus, Focus::Local);
        s.toggle_focus();
        assert_eq!(s.focus, Focus::Remote);
    }

    #[test]
    fn enter_dir_only_for_directories() {
        let mut s = state_with_entries();
        // selected 0 = "docs" (dir)
        assert_eq!(
            s.enter_dir(),
            Some((Side::Remote, PathBuf::from("/srv/docs")))
        );
        // selected 1 = "a.txt" (file) → None
        s.remote.selected = 1;
        assert_eq!(s.enter_dir(), None);
    }

    #[test]
    fn enter_dir_respects_focus() {
        let mut s = state_with_entries();
        s.toggle_focus(); // now Local, selected 0 = "photos"
        assert_eq!(
            s.enter_dir(),
            Some((Side::Local, PathBuf::from("/home/me/photos")))
        );
    }

    #[test]
    fn parent_dir_math() {
        let s = SftpState::new("/srv/www", "/");
        assert_eq!(s.parent_dir(), Some((Side::Remote, PathBuf::from("/srv"))));
        let root = SftpState::new("/", "/");
        assert_eq!(root.parent_dir(), None);
    }

    #[test]
    fn stage_download_file() {
        let mut s = state_with_entries();
        s.remote.selected = 1; // a.txt
        assert!(s.stage_download().is_ok());
        assert_eq!(s.queue.len(), 1);
        let q = &s.queue[0];
        assert_eq!(q.direction, Direction::Download);
        assert_eq!(q.src, PathBuf::from("/srv/a.txt"));
        assert_eq!(q.dst, PathBuf::from("/home/me/a.txt"));
        assert_eq!(q.name, "a.txt");
    }

    #[test]
    fn stage_download_directory_refused() {
        let mut s = state_with_entries();
        s.remote.selected = 0; // docs (dir)
        let err = s.stage_download().unwrap_err();
        assert_eq!(err, "directories can't be staged in v1");
        assert!(s.queue.is_empty());
        assert_eq!(
            s.notice.as_deref(),
            Some("directories can't be staged in v1")
        );
    }

    #[test]
    fn stage_upload_file() {
        let mut s = state_with_entries();
        s.local.selected = 1; // b.bin
        assert!(s.stage_upload().is_ok());
        let q = &s.queue[0];
        assert_eq!(q.direction, Direction::Upload);
        assert_eq!(q.src, PathBuf::from("/home/me/b.bin"));
        assert_eq!(q.dst, PathBuf::from("/srv/b.bin"));
    }

    #[test]
    fn stage_upload_directory_refused() {
        let mut s = state_with_entries();
        s.local.selected = 0; // photos (dir)
        assert!(s.stage_upload().is_err());
        assert!(s.queue.is_empty());
    }

    #[test]
    fn unstage_removes() {
        let mut s = state_with_entries();
        s.remote.selected = 1; // a.txt → download
        s.stage_download().unwrap();
        s.local.selected = 1; // b.bin → upload
        s.stage_upload().unwrap();
        assert_eq!(s.queue.len(), 2);
        s.unstage(0);
        assert_eq!(s.queue.len(), 1);
        s.unstage(99); // out of range → no-op
        assert_eq!(s.queue.len(), 1);
    }

    #[test]
    fn staging_same_file_twice_is_deduped() {
        let mut s = state_with_entries();
        s.remote.selected = 1; // a.txt
        s.stage_download().unwrap();
        // Second identical stage is rejected (no duplicate queue entry).
        assert!(s.stage_download().is_err());
        assert_eq!(s.queue.len(), 1);
        assert!(s.notice.is_some());
    }

    #[test]
    fn stage_download_nothing_selected() {
        let mut s = SftpState::new("/srv", "/home/me"); // remote pane empty
        let err = s.stage_download().unwrap_err();
        assert_eq!(err, "nothing selected");
        assert!(s.queue.is_empty());
    }

    #[test]
    fn stage_upload_nothing_selected() {
        let mut s = SftpState::new("/srv", "/home/me"); // local pane empty
        let err = s.stage_upload().unwrap_err();
        assert_eq!(err, "nothing selected");
        assert!(s.queue.is_empty());
    }

    #[test]
    fn stage_download_uses_local_cwd_as_dst_regardless_of_local_focus() {
        // Downloads always land in local.cwd even when focus is on Local.
        let mut s = state_with_entries();
        s.toggle_focus(); // focus Local, but stage_download reads the remote pane
        s.remote.selected = 1; // a.txt
        s.stage_download().unwrap();
        assert_eq!(s.queue[0].src, PathBuf::from("/srv/a.txt"));
        assert_eq!(s.queue[0].dst, PathBuf::from("/home/me/a.txt"));
    }

    #[test]
    fn focused_side_tracks_focus() {
        let mut s = SftpState::new("/", "/");
        assert_eq!(s.focused_side(), Side::Remote);
        s.toggle_focus();
        assert_eq!(s.focused_side(), Side::Local);
    }

    #[test]
    fn move_selection_uses_focused_pane() {
        // A move on the focused (Local) pane must not touch the remote cursor.
        let mut s = state_with_entries();
        s.toggle_focus(); // Local
        s.move_selection(1);
        assert_eq!(s.local.selected, 1);
        assert_eq!(s.remote.selected, 0);
    }

    #[test]
    fn filter_limits_visible_and_selection() {
        let mut s = state_with_entries(); // remote: "docs"(dir), "a.txt"(file)
        s.remote.set_filter("txt".into());
        assert_eq!(s.remote.visible_len(), 1);
        assert_eq!(s.remote.selected, 0);
        assert_eq!(s.remote.selected_entry().unwrap().name, "a.txt");
    }

    #[test]
    fn set_entries_clears_filter() {
        let mut s = state_with_entries();
        s.remote.set_filter("txt".into());
        s.remote.set_entries(vec![FileEntry {
            name: "z".into(),
            is_dir: false,
            size: 1,
        }]);
        assert!(s.remote.filter.is_empty());
        assert_eq!(s.remote.visible_len(), 1);
    }

    #[test]
    fn move_selection_clamps_to_visible() {
        let mut s = state_with_entries();
        s.focus = Focus::Remote;
        s.remote.set_filter("txt".into()); // only 1 visible
        s.move_selection(5);
        assert_eq!(s.remote.selected, 0);
    }

    #[test]
    fn set_entries_clamps_selection() {
        let mut s = state_with_entries();
        s.remote.selected = 1;
        s.remote.set_entries(vec![FileEntry {
            name: "only".into(),
            is_dir: false,
            size: 1,
        }]);
        assert_eq!(s.remote.selected, 0);
    }
}
