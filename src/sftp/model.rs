//! Pure SFTP UI state model — NO I/O.
//!
//! Everything here is deterministic path/selection math so it can be unit
//! tested without a network, a filesystem, or a live SSH session. The worker
//! ([`super::worker`]) and transport ([`super::transport`]) own all I/O; this
//! module only decides *what* to do and computes the paths involved.

use std::path::PathBuf;

/// Name of the synthetic row that walks a pane up to its parent directory.
pub const PARENT_ROW: &str = "..";

/// The directory one level up from `path`, or `None` at the root.
///
/// `Path::parent` alone is wrong for the relative paths the remote pane starts
/// on: the server resolves the login directory from `"."`, whose `parent()` is
/// the empty path -- a listing request the server rejects, which is why walking
/// up from a fresh remote pane did nothing at all. Relative paths therefore
/// grow a `..` component instead of losing their last one.
pub fn parent_of(path: &std::path::Path) -> Option<PathBuf> {
    let last = path.components().next_back();
    // Already climbing (".", "..", "a/.."): another `..` is the only way up.
    if matches!(
        last,
        Some(std::path::Component::CurDir) | Some(std::path::Component::ParentDir)
    ) {
        return Some(path.join(PARENT_ROW));
    }
    match path.parent() {
        // A bare name ("work") sits in the pane's own directory.
        Some(p) if p.as_os_str().is_empty() => Some(PathBuf::from(".")),
        Some(p) => Some(p.to_path_buf()),
        // Filesystem root: nowhere left to go.
        None => None,
    }
}

/// Which of the two panes a path/entry belongs to.
///
/// `Remote` is the right-hand pane, always a server. `Local` is the left-hand
/// one, which is the local filesystem by default but can be pointed at a
/// second server ([`SftpState::left_host`]) -- the name is kept for its default
/// and because the worker protocol is written in these terms.
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
    /// True if the entry is a symlink. Recursive transfer/delete must NOT
    /// descend into these (avoids following a link outside the tree or a cycle).
    pub is_symlink: bool,
    /// Unix permission bits (the low 12 bits of the mode), if known. Used to
    /// seed the chmod prompt and could be shown in the row.
    pub perm: Option<u32>,
}

/// A transfer staged in the queue but not yet run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedTransfer {
    pub direction: Direction,
    pub src: PathBuf,
    pub dst: PathBuf,
    pub name: String,
    /// Whether `src` is a directory — the worker transfers it recursively.
    pub is_dir: bool,
}

impl FileEntry {
    /// The synthetic ".." row. Enterable, but never a transfer or file-op
    /// target -- see [`FileEntry::is_parent`].
    pub fn parent_row() -> Self {
        Self {
            name: PARENT_ROW.to_string(),
            is_dir: true,
            size: 0,
            is_symlink: false,
            perm: None,
        }
    }

    /// Whether this is the synthetic parent row rather than a real listing
    /// entry. No real file can be named `..`, so the name is enough.
    pub fn is_parent(&self) -> bool {
        self.name == PARENT_ROW
    }
}

/// One browsable directory column (remote or local).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    pub cwd: PathBuf,
    pub entries: Vec<FileEntry>,
    pub selected: usize,
    pub filter: String,
    /// Whether dotfiles are listed. Off by default: in a home directory they
    /// are most of the listing, and what someone came for sits below them.
    pub show_hidden: bool,
}

impl Pane {
    fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            entries: Vec::new(),
            selected: 0,
            filter: String::new(),
            show_hidden: false,
        }
    }

    /// Whether `entry` is listed right now: it must survive the text filter
    /// and, unless dotfiles are shown, not be one.
    ///
    /// The synthetic `..` row is exempt from the dotfile rule -- its name
    /// starts with a dot, but it is the way out of the directory rather than an
    /// entry in it. It is *not* exempt from the text filter: while searching,
    /// the user is after something specific, and a `..` sitting at the top of
    /// the results would take the cursor `set_filter` parks on row zero.
    fn is_visible(&self, entry: &FileEntry) -> bool {
        if entry.is_parent() {
            return self.filter.is_empty();
        }
        if !self.show_hidden && entry.name.starts_with('.') {
            return false;
        }
        self.filter.is_empty()
            || entry
                .name
                .to_lowercase()
                .contains(&self.filter.to_lowercase())
    }

    /// Indices into `entries` that are currently listed.
    pub fn visible_indices(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| self.is_visible(e))
            .map(|(i, _)| i)
            .collect()
    }

    /// Number of entries currently listed.
    pub fn visible_len(&self) -> usize {
        self.entries.iter().filter(|e| self.is_visible(e)).count()
    }

    /// Set the filter text and move the cursor to the top of the filtered view.
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.selected = 0;
    }

    /// The entry under the cursor, if any.
    pub fn selected_entry(&self) -> Option<&FileEntry> {
        self.entries
            .iter()
            .filter(|e| self.is_visible(e))
            .nth(self.selected)
    }

    /// Replace the listing and clamp the cursor to the new bounds.
    ///
    /// Prepends the synthetic [`PARENT_ROW`] unless the pane is at the root, so
    /// walking up is a visible row you can select rather than a keybind you
    /// have to know about.
    pub fn set_entries(&mut self, entries: Vec<FileEntry>) {
        self.entries = entries;
        if parent_of(&self.cwd).is_some() {
            self.entries.insert(0, FileEntry::parent_row());
        }
        self.filter = String::new();
        self.clamp_selection();
    }

    /// Drop the entry named `name` (if present) and clamp the cursor. Used for
    /// optimistic feedback after a delete is dispatched, before the async
    /// re-listing lands.
    pub fn remove_named(&mut self, name: &str) {
        if let Some(i) = self.entries.iter().position(|e| e.name == name) {
            self.entries.remove(i);
            self.clamp_selection();
        }
    }

    pub(crate) fn clamp_selection(&mut self) {
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
    /// True from connect until the worker reports `Connected`, so the UI shows a
    /// "connecting…" state (the picker) instead of an empty browser.
    pub connecting: bool,
    /// Name of the second server the left pane is browsing, or `None` when it
    /// is showing the local filesystem.
    pub left_host: Option<String>,
    /// True from connecting the left pane's server until it reports `Connected`.
    pub left_connecting: bool,
    /// The transfers handed to the worker for the run in flight. The queue can
    /// be added to while it runs, so completion clears exactly this snapshot
    /// rather than everything staged.
    pub running: Vec<QueuedTransfer>,
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
            connecting: false,
            left_host: None,
            left_connecting: false,
            running: Vec::new(),
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
        if entry.is_parent() {
            return self.parent_dir();
        }
        if !entry.is_dir {
            return None;
        }
        Some((self.focused_side(), pane.cwd.join(&entry.name)))
    }

    /// Compute the parent path of the focused pane's cwd. Returns
    /// `(Side, parent)` or `None` when already at the root. Pure PathBuf math.
    pub fn parent_dir(&self) -> Option<(Side, PathBuf)> {
        let parent = parent_of(&self.focused_pane().cwd)?;
        Some((self.focused_side(), parent))
    }

    /// Stage the **focused pane's** selection for transfer toward `target`.
    ///
    /// The arrow keys point at the destination pane (left = local, right =
    /// remote) and the source is always what the cursor is on. Staging used to
    /// read the remote pane whichever side was focused, so pressing ← while
    /// browsing locally queued whatever the remote cursor happened to sit on --
    /// and queued it again after each local `cd`, since the destination path
    /// had changed and the duplicate guard compares both ends.
    pub fn stage_toward(&mut self, target: Side) -> Result<(), String> {
        if self.focused_side() == target {
            let msg = "that pane is the destination — Tab to pick a source".to_string();
            self.notice = Some(msg.clone());
            return Err(msg);
        }
        match target {
            Side::Local => self.stage_download(),
            Side::Remote => self.stage_upload(),
        }
    }

    /// Stage the focused-remote selection for download into `local.cwd`.
    /// Directories are staged too and transferred recursively by the worker.
    pub fn stage_download(&mut self) -> Result<(), String> {
        let entry = self
            .remote
            .selected_entry()
            .cloned()
            .ok_or_else(|| "nothing selected".to_string())?;
        if entry.is_parent() {
            return Err("that row just walks up a directory".to_string());
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
            is_dir: entry.is_dir,
        });
        Ok(())
    }

    /// Stage the focused-local selection for upload into `remote.cwd`.
    /// Directories are staged too and transferred recursively by the worker.
    pub fn stage_upload(&mut self) -> Result<(), String> {
        let entry = self
            .local
            .selected_entry()
            .cloned()
            .ok_or_else(|| "nothing selected".to_string())?;
        if entry.is_parent() {
            return Err("that row just walks up a directory".to_string());
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
            is_dir: entry.is_dir,
        });
        Ok(())
    }

    /// Show or hide dotfiles in both panes at once -- they are browsed as a
    /// pair, so splitting the setting would just be two keys to press. Returns
    /// the new state so the caller can persist it.
    pub fn toggle_hidden(&mut self) -> bool {
        let show = !self.local.show_hidden;
        self.local.show_hidden = show;
        self.remote.show_hidden = show;
        // The cursor was indexing the old listing; keep it inside the new one.
        self.local.clamp_selection();
        self.remote.clamp_selection();
        show
    }

    /// Whether the left pane is browsing a second server rather than the local
    /// filesystem, which decides where its listings and file ops are sent.
    pub fn left_is_remote(&self) -> bool {
        self.left_host.is_some()
    }

    /// Take the transfers just finished out of the queue, leaving anything
    /// staged while the run was in flight. Returns whether work is left.
    pub fn finish_run(&mut self) -> bool {
        let done = std::mem::take(&mut self.running);
        self.queue.retain(|q| !done.contains(q));
        !self.queue.is_empty()
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

    /// Index of the row named `name` in `pane`, so tests address rows by name
    /// and stay honest about the synthetic ".." row sitting at index 0.
    fn row(pane: &Pane, name: &str) -> usize {
        pane.entries
            .iter()
            .position(|e| e.name == name)
            .unwrap_or_else(|| panic!("no row named {name}"))
    }

    fn state_with_entries() -> SftpState {
        let mut s = SftpState::new("/srv", "/home/me");
        s.remote.set_entries(vec![
            FileEntry {
                name: "docs".into(),
                is_dir: true,
                size: 0,
                is_symlink: false,
                perm: None,
            },
            FileEntry {
                name: "a.txt".into(),
                is_dir: false,
                size: 10,
                is_symlink: false,
                perm: None,
            },
        ]);
        s.local.set_entries(vec![
            FileEntry {
                name: "photos".into(),
                is_dir: true,
                size: 0,
                is_symlink: false,
                perm: None,
            },
            FileEntry {
                name: "b.bin".into(),
                is_dir: false,
                size: 20,
                is_symlink: false,
                perm: None,
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
        // "..", "docs", "a.txt"
        s.move_selection(-5);
        assert_eq!(s.remote.selected, 0);
        s.move_selection(1);
        assert_eq!(s.remote.selected, 1);
        s.move_selection(10);
        assert_eq!(s.remote.selected, 2); // clamped to len-1
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
        s.remote.selected = row(&s.remote, "docs");
        assert_eq!(
            s.enter_dir(),
            Some((Side::Remote, PathBuf::from("/srv/docs")))
        );
        // A file isn't enterable.
        s.remote.selected = row(&s.remote, "a.txt");
        assert_eq!(s.enter_dir(), None);
    }

    #[test]
    fn parent_row_leads_the_listing_and_walks_up() {
        let mut s = state_with_entries();
        assert_eq!(s.remote.entries[0].name, PARENT_ROW, "\"..\" comes first");
        assert!(s.remote.entries[0].is_parent());
        // Selecting it and entering walks up, rather than into "/srv/..".
        s.remote.selected = 0;
        assert_eq!(s.enter_dir(), Some((Side::Remote, PathBuf::from("/"))));
        // It is never a transfer target.
        assert!(s.stage_download().is_err());
        assert!(s.queue.is_empty());
    }

    #[test]
    fn parent_row_absent_at_the_root() {
        let mut s = SftpState::new("/", "/");
        s.remote.set_entries(vec![FileEntry {
            name: "etc".into(),
            is_dir: true,
            size: 0,
            is_symlink: false,
            perm: None,
        }]);
        assert_eq!(s.remote.entries[0].name, "etc", "nowhere to walk up to");
    }

    #[test]
    fn parent_of_climbs_relative_paths() {
        use std::path::Path;
        // The remote pane starts on ".", whose `parent()` is the empty path;
        // walking up has to grow a "..", not produce an unlistable path.
        assert_eq!(parent_of(Path::new(".")), Some(PathBuf::from("./..")));
        assert_eq!(parent_of(Path::new("..")), Some(PathBuf::from("../..")));
        assert_eq!(parent_of(Path::new("work")), Some(PathBuf::from(".")));
        assert_eq!(
            parent_of(Path::new("/srv/www")),
            Some(PathBuf::from("/srv"))
        );
        assert_eq!(parent_of(Path::new("/")), None);
    }

    #[test]
    fn enter_dir_respects_focus() {
        let mut s = state_with_entries();
        s.toggle_focus(); // now Local
        s.local.selected = row(&s.local, "photos");
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
        s.remote.selected = row(&s.remote, "a.txt");
        assert!(s.stage_download().is_ok());
        assert_eq!(s.queue.len(), 1);
        let q = &s.queue[0];
        assert_eq!(q.direction, Direction::Download);
        assert_eq!(q.src, PathBuf::from("/srv/a.txt"));
        assert_eq!(q.dst, PathBuf::from("/home/me/a.txt"));
        assert_eq!(q.name, "a.txt");
    }

    #[test]
    fn stage_download_directory_is_queued_recursively() {
        let mut s = state_with_entries();
        s.remote.selected = row(&s.remote, "docs");
        assert!(s.stage_download().is_ok());
        assert_eq!(s.queue.len(), 1);
        let q = &s.queue[0];
        assert_eq!(q.direction, Direction::Download);
        assert!(q.is_dir, "a directory stages as a recursive transfer");
        assert_eq!(q.name, "docs");
    }

    #[test]
    fn stage_upload_file() {
        let mut s = state_with_entries();
        s.local.selected = row(&s.local, "b.bin");
        assert!(s.stage_upload().is_ok());
        let q = &s.queue[0];
        assert_eq!(q.direction, Direction::Upload);
        assert_eq!(q.src, PathBuf::from("/home/me/b.bin"));
        assert_eq!(q.dst, PathBuf::from("/srv/b.bin"));
    }

    #[test]
    fn stage_upload_directory_is_queued_recursively() {
        let mut s = state_with_entries();
        s.local.selected = row(&s.local, "photos");
        assert!(s.stage_upload().is_ok());
        assert_eq!(s.queue.len(), 1);
        assert!(s.queue[0].is_dir);
    }

    fn with_dotfiles() -> SftpState {
        let mut s = SftpState::new("/srv", "/home/me");
        let entry = |name: &str| FileEntry {
            name: name.into(),
            is_dir: false,
            size: 1,
            is_symlink: false,
            perm: None,
        };
        s.remote
            .set_entries(vec![entry(".ssh"), entry("app.log"), entry(".bashrc")]);
        s.local
            .set_entries(vec![entry(".config"), entry("notes.txt")]);
        s
    }

    #[test]
    fn dotfiles_are_hidden_until_toggled() {
        let mut s = with_dotfiles();
        // Listed: "..", "app.log". The two dotfiles are filtered out.
        assert_eq!(s.remote.visible_len(), 2);
        let names: Vec<&str> = s
            .remote
            .visible_indices()
            .iter()
            .map(|i| s.remote.entries[*i].name.as_str())
            .collect();
        assert_eq!(names, vec![PARENT_ROW, "app.log"]);

        // The toggle applies to both panes at once and reports the new state.
        assert!(s.toggle_hidden());
        assert_eq!(s.remote.visible_len(), 4);
        assert_eq!(s.local.visible_len(), 3);
        assert!(!s.toggle_hidden());
        assert_eq!(s.remote.visible_len(), 2);
    }

    /// The parent row's name starts with a dot but is the way out of the
    /// directory, not an entry in it: hiding dotfiles must not hide it.
    #[test]
    fn parent_row_survives_the_dotfile_filter() {
        let s = with_dotfiles();
        assert!(!s.remote.show_hidden);
        assert!(s.remote.selected_entry().unwrap().is_parent());
        assert!(s
            .remote
            .visible_indices()
            .iter()
            .any(|i| s.remote.entries[*i].is_parent()));
    }

    /// Hiding entries under the cursor must not leave it pointing past the end
    /// of the listing.
    #[test]
    fn toggling_hidden_keeps_the_cursor_in_range() {
        let mut s = with_dotfiles();
        s.toggle_hidden();
        s.remote.selected = 3; // ".bashrc", only reachable while shown
        s.toggle_hidden();
        assert!(
            s.remote.selected < s.remote.visible_len(),
            "cursor left past the end of the listing"
        );
        assert!(s.remote.selected_entry().is_some());
    }

    /// Searching is for finding entries, so the way-out row steps aside --
    /// otherwise it would take row zero, where `set_filter` parks the cursor,
    /// and Enter on a search result would walk up instead.
    #[test]
    fn parent_row_steps_aside_while_filtering() {
        let mut s = with_dotfiles();
        assert!(s.remote.selected_entry().unwrap().is_parent());
        s.remote.set_filter("app".into());
        assert_eq!(s.remote.visible_len(), 1);
        assert_eq!(s.remote.selected_entry().unwrap().name, "app.log");
    }

    /// The text filter and the dotfile filter compose rather than override.
    #[test]
    fn text_filter_still_respects_hidden() {
        let mut s = with_dotfiles();
        s.remote.set_filter("sh".into());
        // ".ssh" and ".bashrc" both match "sh" but are hidden.
        assert_eq!(s.remote.visible_len(), 0);
        s.toggle_hidden();
        assert_eq!(s.remote.visible_len(), 2);
    }

    #[test]
    fn stage_toward_takes_the_source_from_the_focused_pane() {
        let mut s = state_with_entries();
        // Focused remote, arrow points left: download, as before.
        s.remote.selected = row(&s.remote, "a.txt");
        assert!(s.stage_toward(Side::Local).is_ok());
        assert_eq!(s.queue.len(), 1);
        assert_eq!(s.queue[0].direction, Direction::Download);

        // Focused remote, arrow points right: that pane is where the cursor
        // already is, so nothing is staged.
        assert!(s.stage_toward(Side::Remote).is_err());
        assert_eq!(s.queue.len(), 1);
        assert!(s.notice.is_some());

        // Focused local, arrow points right: upload.
        s.toggle_focus();
        s.local.selected = row(&s.local, "b.bin");
        assert!(s.stage_toward(Side::Remote).is_ok());
        assert_eq!(s.queue[1].direction, Direction::Upload);
    }

    /// Regression: pressing ← while browsing locally used to queue the remote
    /// cursor's entry, and re-queue it after every local `cd`, because the
    /// duplicate guard compares src *and* dst and the dst had moved.
    #[test]
    fn browsing_locally_never_queues_the_remote_cursor() {
        let mut s = state_with_entries();
        s.toggle_focus(); // Local
        assert!(s.stage_toward(Side::Local).is_err());
        assert!(s.queue.is_empty());

        // ...including after walking into another local directory.
        s.local.cwd = PathBuf::from("/home/me/work");
        s.local.set_entries(Vec::new());
        assert!(s.stage_toward(Side::Local).is_err());
        assert!(s.queue.is_empty());
    }

    #[test]
    fn finishing_a_run_keeps_what_was_staged_meanwhile() {
        let mut s = state_with_entries();
        s.remote.selected = row(&s.remote, "a.txt");
        s.stage_download().unwrap();
        // The run goes out with what is queued right now.
        s.running = s.queue.clone();
        s.phase = Phase::Running;

        // More work is staged while it runs.
        s.remote.selected = row(&s.remote, "docs");
        s.stage_download().unwrap();
        assert_eq!(s.queue.len(), 2);

        // Completion clears only the snapshot that ran, and reports that there
        // is more to do.
        assert!(s.finish_run(), "the mid-run entry is still pending");
        assert_eq!(s.queue.len(), 1);
        assert_eq!(s.queue[0].name, "docs");
        assert!(s.running.is_empty());

        // A run with nothing staged behind it reports itself as the last one.
        s.running = s.queue.clone();
        assert!(!s.finish_run());
        assert!(s.queue.is_empty());
    }

    #[test]
    fn unstage_removes() {
        let mut s = state_with_entries();
        s.remote.selected = row(&s.remote, "a.txt"); // download
        s.stage_download().unwrap();
        s.local.selected = row(&s.local, "b.bin"); // upload
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
        s.remote.selected = row(&s.remote, "a.txt");
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
        s.remote.selected = row(&s.remote, "a.txt");
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
            is_symlink: false,
            perm: None,
        }]);
        assert!(s.remote.filter.is_empty());
        // The new row, plus the ".." the pane grows below the root.
        assert_eq!(s.remote.visible_len(), 2);
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
        s.remote.selected = 5;
        s.remote.set_entries(vec![FileEntry {
            name: "only".into(),
            is_dir: false,
            size: 1,
            is_symlink: false,
            perm: None,
        }]);
        // Clamped to the last row of the new listing ("..", "only").
        assert_eq!(s.remote.selected, 1);
    }
}
