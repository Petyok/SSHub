//! User-remappable keybindings.
//!
//! Each entry is a list of key specs so a user can add their own binding
//! without losing the defaults. Specs look like `"F2"`, `"Ctrl+S"`,
//! `"Alt+Enter"`, `"F10"` (parsed in [`crate::app::util::parse_keyspec`]).

use serde::{Deserialize, Serialize};

/// An action whose keybinding is user-configurable and editable in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Save,
    Quit,
    Help,
    Search,
    KeybindEditor,
    ForceQuit,
    Connect,
    AddHost,
    GenerateKey,
    Edit,
    Delete,
    Duplicate,
    TagFilter,
    Favorite,
    ToggleGroup,
    FoldGroupIn,
    FoldGroupOut,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    MoveGroupUp,
    MoveGroupDown,
    MoveHostUp,
    MoveHostDown,
    CollapseAll,
    DetailFocus,
    ClearSshLog,
    SortCycle,
    YankLog,
    RevealSecret,
    CopySecret,
    UiZoomIn,
    UiZoomOut,
    ExportSsh,
    ImportSsh,
    ImportTermius,
    GroupsManage,
    RenameGroup,
    DeleteGroup,
    TabHosts,
    TabSftp,
    TabTunnels,
    TabKeys,
    TabAudit,
    IdentityColumnsInc,
    IdentityColumnsDec,
    AddToAgent,
    RemoveFromAgent,
    PushKey,
    TunnelKill,
    ToggleTunnel,
    AuditFilter,
    AuditRange,
    SessionNewTab,
    SessionCloseTab,
    SessionTabPrev,
    SessionTabNext,
    SessionDetach,
    SessionOpenSftp,
    LocalShell,
    SessionFocus,
    SessionSwitcher,
    SessionScrollUp,
    SessionScrollDown,
    SessionCancel,
    SessionToggleLog,
    ConfirmYes,
    ConfirmNo,
    Cancel,
    TogglePanelZoom,
    FocusPanelLeft,
    FocusPanelRight,
    FocusPanelUp,
    FocusPanelDown,
    Broadcast,
    BroadcastCancel,
    KnownHosts,
    KnownHostsDelete,
    KnownHostsRefresh,
    LogsBrowser,
    SnippetsManage,
    SessionSnippets,
    ProfilesManage,
    GhostAccept,
}

impl KeyAction {
    /// All editable actions, in display order.
    pub const ALL: [KeyAction; 85] = [
        KeyAction::Save,
        KeyAction::Quit,
        KeyAction::Help,
        KeyAction::Search,
        KeyAction::KeybindEditor,
        KeyAction::ForceQuit,
        KeyAction::Connect,
        KeyAction::AddHost,
        KeyAction::GenerateKey,
        KeyAction::Edit,
        KeyAction::Delete,
        KeyAction::Duplicate,
        KeyAction::TagFilter,
        KeyAction::Favorite,
        KeyAction::ToggleGroup,
        KeyAction::FoldGroupIn,
        KeyAction::FoldGroupOut,
        KeyAction::MoveUp,
        KeyAction::MoveDown,
        KeyAction::MoveLeft,
        KeyAction::MoveRight,
        KeyAction::MoveGroupUp,
        KeyAction::MoveGroupDown,
        KeyAction::MoveHostUp,
        KeyAction::MoveHostDown,
        KeyAction::CollapseAll,
        KeyAction::DetailFocus,
        KeyAction::ClearSshLog,
        KeyAction::SortCycle,
        KeyAction::YankLog,
        KeyAction::RevealSecret,
        KeyAction::CopySecret,
        KeyAction::UiZoomIn,
        KeyAction::UiZoomOut,
        KeyAction::ExportSsh,
        KeyAction::ImportSsh,
        KeyAction::ImportTermius,
        KeyAction::GroupsManage,
        KeyAction::RenameGroup,
        KeyAction::DeleteGroup,
        KeyAction::TabHosts,
        KeyAction::TabSftp,
        KeyAction::TabTunnels,
        KeyAction::TabKeys,
        KeyAction::TabAudit,
        KeyAction::IdentityColumnsInc,
        KeyAction::IdentityColumnsDec,
        KeyAction::AddToAgent,
        KeyAction::RemoveFromAgent,
        KeyAction::PushKey,
        KeyAction::TunnelKill,
        KeyAction::ToggleTunnel,
        KeyAction::AuditFilter,
        KeyAction::AuditRange,
        KeyAction::SessionNewTab,
        KeyAction::SessionCloseTab,
        KeyAction::SessionTabPrev,
        KeyAction::SessionTabNext,
        KeyAction::SessionDetach,
        KeyAction::SessionOpenSftp,
        KeyAction::LocalShell,
        KeyAction::SessionFocus,
        KeyAction::SessionSwitcher,
        KeyAction::SessionScrollUp,
        KeyAction::SessionScrollDown,
        KeyAction::SessionCancel,
        KeyAction::SessionToggleLog,
        KeyAction::ConfirmYes,
        KeyAction::ConfirmNo,
        KeyAction::Cancel,
        KeyAction::TogglePanelZoom,
        KeyAction::FocusPanelLeft,
        KeyAction::FocusPanelRight,
        KeyAction::FocusPanelUp,
        KeyAction::FocusPanelDown,
        KeyAction::Broadcast,
        KeyAction::BroadcastCancel,
        KeyAction::KnownHosts,
        KeyAction::KnownHostsDelete,
        KeyAction::KnownHostsRefresh,
        KeyAction::LogsBrowser,
        KeyAction::SnippetsManage,
        KeyAction::SessionSnippets,
        KeyAction::ProfilesManage,
        KeyAction::GhostAccept,
    ];

    pub fn label(self) -> &'static str {
        match self {
            KeyAction::Save => "Save form",
            KeyAction::Quit => "Quit",
            KeyAction::Help => "Help",
            KeyAction::Search => "Search / palette",
            KeyAction::KeybindEditor => "Edit keybindings",
            KeyAction::ForceQuit => "Force quit",
            KeyAction::Connect => "Connect / confirm",
            KeyAction::AddHost => "Add host / identity / tunnel",
            KeyAction::GenerateKey => "Generate SSH key",
            KeyAction::Edit => "Edit",
            KeyAction::Delete => "Delete",
            KeyAction::Duplicate => "Duplicate host",
            KeyAction::TagFilter => "Filter by tag",
            KeyAction::Favorite => "Toggle favorite",
            KeyAction::ToggleGroup => "Toggle group fold",
            KeyAction::FoldGroupIn => "Fold group in",
            KeyAction::FoldGroupOut => "Fold group out",
            KeyAction::MoveUp => "Move up",
            KeyAction::MoveDown => "Move down",
            KeyAction::MoveLeft => "Move left",
            KeyAction::MoveRight => "Move right",
            KeyAction::MoveGroupUp => "Jump to previous group",
            KeyAction::MoveGroupDown => "Jump to next group",
            KeyAction::MoveHostUp => "Move host up",
            KeyAction::MoveHostDown => "Move host down",
            KeyAction::CollapseAll => "Collapse / expand all groups",
            KeyAction::DetailFocus => "Toggle detail panel",
            KeyAction::ClearSshLog => "Clear SSH log",
            KeyAction::SortCycle => "Cycle sort mode",
            KeyAction::YankLog => "Copy SSH log",
            KeyAction::RevealSecret => "Show and copy the stored secret",
            KeyAction::CopySecret => "Copy the stored secret",
            KeyAction::UiZoomIn => "Zoom in (hosts column)",
            KeyAction::UiZoomOut => "Zoom out (hosts column)",
            KeyAction::ExportSsh => "Export to ssh config",
            KeyAction::ImportSsh => "Import from ssh config",
            KeyAction::ImportTermius => "Import Termius backup",
            KeyAction::GroupsManage => "Manage groups",
            KeyAction::RenameGroup => "Edit group",
            KeyAction::DeleteGroup => "Delete group",
            KeyAction::TabHosts => "Hosts tab",
            KeyAction::TabSftp => "SFTP tab",
            KeyAction::TabTunnels => "Tunnels tab",
            KeyAction::TabKeys => "Identities tab",
            KeyAction::TabAudit => "Audit tab",
            KeyAction::IdentityColumnsInc => "More identity columns",
            KeyAction::IdentityColumnsDec => "Fewer identity columns",
            KeyAction::AddToAgent => "Add key to agent",
            KeyAction::RemoveFromAgent => "Remove key from agent",
            KeyAction::PushKey => "Push public key to host",
            KeyAction::TunnelKill => "Kill tunnel",
            KeyAction::ToggleTunnel => "Start / stop tunnel",
            KeyAction::AuditFilter => "Cycle audit filter",
            KeyAction::AuditRange => "Cycle audit range",
            KeyAction::SessionNewTab => "New session tab",
            KeyAction::SessionCloseTab => "Close session tab",
            KeyAction::SessionTabPrev => "Previous session tab",
            KeyAction::SessionTabNext => "Next session tab",
            KeyAction::SessionDetach => "Detach to dashboard",
            KeyAction::SessionOpenSftp => "Open SFTP for this host",
            KeyAction::LocalShell => "Open local shell tab",
            KeyAction::SessionFocus => "Focus session tab",
            KeyAction::SessionSwitcher => "Switch to an open session",
            KeyAction::SessionScrollUp => "Scroll session up",
            KeyAction::SessionScrollDown => "Scroll session down",
            KeyAction::SessionCancel => "Cancel connecting",
            KeyAction::SessionToggleLog => "Toggle connect debug log",
            KeyAction::ConfirmYes => "Confirm yes",
            KeyAction::ConfirmNo => "Confirm no",
            KeyAction::Cancel => "Cancel / back",
            KeyAction::TogglePanelZoom => "Zoom dashboard panel",
            KeyAction::FocusPanelLeft => "Focus panel left",
            KeyAction::FocusPanelRight => "Focus panel right",
            KeyAction::FocusPanelUp => "Focus panel up",
            KeyAction::FocusPanelDown => "Focus panel down",
            KeyAction::Broadcast => "Broadcast command",
            KeyAction::BroadcastCancel => "Cancel broadcast",
            KeyAction::KnownHosts => "Known hosts",
            KeyAction::KnownHostsDelete => "Known hosts: delete",
            KeyAction::KnownHostsRefresh => "Known hosts: refresh",
            KeyAction::LogsBrowser => "Browse session logs",
            KeyAction::SnippetsManage => "Manage command snippets",
            KeyAction::SessionSnippets => "Session: run a command snippet",
            KeyAction::ProfilesManage => "Add or manage profiles",
            KeyAction::GhostAccept => "Session: accept ghost completion",
        }
    }
}

/// User-remappable keybindings. Each field is a list of key specs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeybindsConfig {
    pub save: Vec<String>,
    pub quit: Vec<String>,
    pub help: Vec<String>,
    pub search: Vec<String>,
    pub keybind_editor: Vec<String>,
    pub force_quit: Vec<String>,
    pub connect: Vec<String>,
    pub add_host: Vec<String>,
    pub generate_key: Vec<String>,
    pub edit: Vec<String>,
    pub delete: Vec<String>,
    pub duplicate: Vec<String>,
    pub tag_filter: Vec<String>,
    pub favorite: Vec<String>,
    pub toggle_group: Vec<String>,
    pub fold_group_in: Vec<String>,
    pub fold_group_out: Vec<String>,
    pub move_up: Vec<String>,
    pub move_down: Vec<String>,
    pub move_left: Vec<String>,
    pub move_right: Vec<String>,
    pub move_group_up: Vec<String>,
    pub move_group_down: Vec<String>,
    pub move_host_up: Vec<String>,
    pub move_host_down: Vec<String>,
    pub collapse_all: Vec<String>,
    pub detail_focus: Vec<String>,
    pub clear_ssh_log: Vec<String>,
    pub sort_cycle: Vec<String>,
    pub yank_log: Vec<String>,
    pub reveal_secret: Vec<String>,
    pub copy_secret: Vec<String>,
    pub ui_zoom_in: Vec<String>,
    pub ui_zoom_out: Vec<String>,
    pub export_ssh: Vec<String>,
    pub import_ssh: Vec<String>,
    pub import_termius: Vec<String>,
    pub groups_manage: Vec<String>,
    pub rename_group: Vec<String>,
    pub delete_group: Vec<String>,
    pub tab_hosts: Vec<String>,
    pub tab_sftp: Vec<String>,
    pub tab_tunnels: Vec<String>,
    pub tab_keys: Vec<String>,
    pub tab_audit: Vec<String>,
    pub identity_columns_inc: Vec<String>,
    pub identity_columns_dec: Vec<String>,
    pub add_to_agent: Vec<String>,
    pub remove_from_agent: Vec<String>,
    pub push_key: Vec<String>,
    pub tunnel_kill: Vec<String>,
    pub toggle_tunnel: Vec<String>,
    pub audit_filter: Vec<String>,
    pub audit_range: Vec<String>,
    pub session_new_tab: Vec<String>,
    pub session_close_tab: Vec<String>,
    pub session_tab_prev: Vec<String>,
    pub session_tab_next: Vec<String>,
    pub session_detach: Vec<String>,
    pub session_open_sftp: Vec<String>,
    pub local_shell: Vec<String>,
    pub session_focus: Vec<String>,
    pub session_switcher: Vec<String>,
    pub session_scroll_up: Vec<String>,
    pub session_scroll_down: Vec<String>,
    pub session_cancel: Vec<String>,
    pub session_toggle_log: Vec<String>,
    pub confirm_yes: Vec<String>,
    pub confirm_no: Vec<String>,
    pub cancel: Vec<String>,
    pub toggle_panel_zoom: Vec<String>,
    pub focus_panel_left: Vec<String>,
    pub focus_panel_right: Vec<String>,
    pub focus_panel_up: Vec<String>,
    pub focus_panel_down: Vec<String>,
    pub broadcast: Vec<String>,
    pub broadcast_cancel: Vec<String>,
    pub known_hosts: Vec<String>,
    pub known_hosts_delete: Vec<String>,
    pub known_hosts_refresh: Vec<String>,
    pub logs_browser: Vec<String>,
    pub snippets_manage: Vec<String>,
    pub session_snippets: Vec<String>,
    pub profiles_manage: Vec<String>,
    pub ghost_accept: Vec<String>,
}

impl Default for KeybindsConfig {
    fn default() -> Self {
        Self {
            save: vec!["F2".into(), "Ctrl+S".into()],
            quit: vec!["q".into()],
            help: vec!["?".into()],
            search: vec!["/".into()],
            keybind_editor: vec!["Ctrl+K".into()],
            force_quit: vec!["Ctrl+C".into()],
            connect: vec!["Enter".into()],
            add_host: vec!["a".into()],
            generate_key: vec!["g".into()],
            edit: vec!["e".into()],
            delete: vec!["d".into()],
            duplicate: vec!["Shift+D".into()],
            tag_filter: vec!["#".into()],
            favorite: vec!["f".into()],
            toggle_group: vec!["Space".into()],
            fold_group_in: vec!["Left".into()],
            fold_group_out: vec!["Right".into()],
            move_up: vec!["k".into(), "Up".into()],
            move_down: vec!["j".into(), "Down".into()],
            move_left: vec!["Left".into()],
            move_right: vec!["l".into(), "Right".into()],
            move_group_up: vec!["Shift+Up".into()],
            move_group_down: vec!["Shift+Down".into()],
            move_host_up: vec!["Ctrl+Up".into()],
            move_host_down: vec!["Ctrl+Down".into()],
            collapse_all: vec!["Shift+Z".into()],
            detail_focus: vec!["Tab".into()],
            clear_ssh_log: vec!["c".into()],
            sort_cycle: vec!["s".into()],
            yank_log: vec!["y".into()],
            reveal_secret: vec!["Ctrl+R".into()],
            copy_secret: vec!["Ctrl+Y".into()],
            ui_zoom_in: vec!["+".into(), "=".into()],
            ui_zoom_out: vec!["-".into(), "_".into()],
            export_ssh: vec!["Shift+E".into()],
            import_ssh: vec!["Shift+I".into()],
            import_termius: vec!["Shift+T".into()],
            groups_manage: vec!["Shift+G".into()],
            rename_group: vec!["Ctrl+G".into()],
            delete_group: vec!["Ctrl+Shift+G".into()],
            tab_hosts: vec!["h".into(), "1".into()],
            tab_sftp: vec!["2".into()],
            tab_tunnels: vec!["3".into()],
            tab_keys: vec!["i".into(), "4".into()],
            tab_audit: vec!["5".into()],
            identity_columns_inc: vec!["]".into()],
            identity_columns_dec: vec!["[".into()],
            add_to_agent: vec!["p".into()],
            remove_from_agent: vec!["r".into()],
            push_key: vec!["Shift+P".into()],
            tunnel_kill: vec!["x".into()],
            toggle_tunnel: vec!["Enter".into()],
            audit_filter: vec!["f".into()],
            audit_range: vec!["r".into()],
            session_new_tab: vec!["Ctrl+T".into()],
            session_close_tab: vec!["Ctrl+W".into()],
            session_tab_prev: vec!["Ctrl+[".into(), "Ctrl+PageUp".into()],
            session_tab_next: vec!["Ctrl+]".into(), "Ctrl+PageDown".into()],
            session_detach: vec!["Ctrl+D".into()],
            session_open_sftp: vec!["Ctrl+Shift+F".into()],
            local_shell: vec!["Ctrl+Shift+T".into()],
            session_focus: vec!["Ctrl+Shift+S".into()],
            session_switcher: vec!["Alt+S".into()],
            session_scroll_up: vec!["PageUp".into()],
            session_scroll_down: vec!["PageDown".into()],
            session_cancel: vec!["Esc".into()],
            session_toggle_log: vec!["Ctrl+O".into()],
            confirm_yes: vec!["y".into(), "Y".into(), "Enter".into()],
            confirm_no: vec!["n".into(), "N".into()],
            cancel: vec!["Esc".into()],
            toggle_panel_zoom: vec!["z".into(), "Alt+Enter".into()],
            focus_panel_left: vec!["Alt+Left".into()],
            focus_panel_right: vec!["Alt+Right".into()],
            focus_panel_up: vec!["Alt+Up".into()],
            focus_panel_down: vec!["Alt+Down".into()],
            broadcast: vec!["b".into()],
            broadcast_cancel: vec!["x".into()],
            known_hosts: vec!["H".into()],
            known_hosts_delete: vec!["Ctrl+D".into()],
            known_hosts_refresh: vec!["Ctrl+R".into()],
            logs_browser: vec!["Shift+L".into()],
            snippets_manage: vec!["Shift+S".into()],
            session_snippets: vec!["Ctrl+N".into()],
            profiles_manage: vec!["Alt+P".into()],
            ghost_accept: vec!["Ctrl+F".into()],
        }
    }
}

impl KeybindsConfig {
    fn default_for(action: KeyAction) -> Vec<String> {
        Self::default().binds(action).to_vec()
    }

    /// Restore one action's bindings to its built-in default.
    pub fn reset_action(&mut self, action: KeyAction) {
        self.set(action, Self::default_for(action));
    }

    /// First configured key for `action`, or `""` when unbound.
    pub fn primary(&self, action: KeyAction) -> &str {
        self.binds(action).first().map(String::as_str).unwrap_or("")
    }

    /// Right-aligned hints in the fullscreen session header.
    pub fn session_header_hints(&self, multi_tab: bool) -> String {
        let mut parts = Vec::new();
        if multi_tab {
            push_hint(&mut parts, self.primary(KeyAction::SessionNewTab), "new");
            push_hint(
                &mut parts,
                self.primary(KeyAction::SessionCloseTab),
                "close",
            );
            if let Some(tabs) = tab_switch_hint(self) {
                parts.push(tabs);
            }
            push_hint(&mut parts, self.primary(KeyAction::SessionDetach), "detach");
        } else {
            push_hint(
                &mut parts,
                self.primary(KeyAction::SessionNewTab),
                "new tab",
            );
            push_hint(&mut parts, self.primary(KeyAction::SessionDetach), "detach");
        }
        push_hint(
            &mut parts,
            self.primary(KeyAction::SessionSwitcher),
            "switch",
        );
        parts.join("  ")
    }

    /// Hint shown while a password / passphrase field is focused. A stored secret
    /// is masked, so without this the two binds that show or copy it are
    /// invisible exactly where they are needed. Empty when both are unbound.
    pub fn secret_field_hints(&self) -> String {
        let mut parts = Vec::new();
        let reveal = self.primary(KeyAction::RevealSecret);
        if !reveal.is_empty() {
            parts.push(format!("{reveal}: show + copy"));
        }
        let copy = self.primary(KeyAction::CopySecret);
        if !copy.is_empty() {
            parts.push(format!("{copy}: copy"));
        }
        parts.join(" \u{2502} ")
    }

    /// Dashboard footer hints when embedded sessions are running in background.
    ///
    /// `resume` comes first on purpose: with a session running somewhere behind
    /// the dashboard, getting back into it is the thing you want, and it used to
    /// be discoverable only through the help overlay.
    ///
    /// `detach` is intentionally absent: you are already on the dashboard, so
    /// there is nothing to detach from. It still appears in the in-session
    /// header via [`Self::session_header_hints`].
    pub fn session_footer_hints(&self) -> Vec<(String, &'static str)> {
        let mut out = Vec::new();
        push_footer_hint(&mut out, self.primary(KeyAction::SessionFocus), "resume");
        push_footer_hint(&mut out, self.primary(KeyAction::SessionOpenSftp), "sftp");
        if let Some(keys) = tab_switch_keys(self) {
            out.push((keys, "tabs"));
        }
        push_footer_hint(&mut out, self.primary(KeyAction::SessionSwitcher), "switch");
        push_footer_hint(&mut out, self.primary(KeyAction::SessionNewTab), "new tab");
        out
    }

    pub fn binds(&self, action: KeyAction) -> &[String] {
        match action {
            KeyAction::Save => &self.save,
            KeyAction::Quit => &self.quit,
            KeyAction::Help => &self.help,
            KeyAction::Search => &self.search,
            KeyAction::KeybindEditor => &self.keybind_editor,
            KeyAction::ForceQuit => &self.force_quit,
            KeyAction::Connect => &self.connect,
            KeyAction::AddHost => &self.add_host,
            KeyAction::GenerateKey => &self.generate_key,
            KeyAction::Edit => &self.edit,
            KeyAction::Delete => &self.delete,
            KeyAction::Duplicate => &self.duplicate,
            KeyAction::TagFilter => &self.tag_filter,
            KeyAction::Favorite => &self.favorite,
            KeyAction::ToggleGroup => &self.toggle_group,
            KeyAction::FoldGroupIn => &self.fold_group_in,
            KeyAction::FoldGroupOut => &self.fold_group_out,
            KeyAction::MoveUp => &self.move_up,
            KeyAction::MoveDown => &self.move_down,
            KeyAction::MoveLeft => &self.move_left,
            KeyAction::MoveRight => &self.move_right,
            KeyAction::MoveGroupUp => &self.move_group_up,
            KeyAction::MoveGroupDown => &self.move_group_down,
            KeyAction::MoveHostUp => &self.move_host_up,
            KeyAction::MoveHostDown => &self.move_host_down,
            KeyAction::CollapseAll => &self.collapse_all,
            KeyAction::DetailFocus => &self.detail_focus,
            KeyAction::ClearSshLog => &self.clear_ssh_log,
            KeyAction::SortCycle => &self.sort_cycle,
            KeyAction::YankLog => &self.yank_log,
            KeyAction::RevealSecret => &self.reveal_secret,
            KeyAction::CopySecret => &self.copy_secret,
            KeyAction::UiZoomIn => &self.ui_zoom_in,
            KeyAction::UiZoomOut => &self.ui_zoom_out,
            KeyAction::ExportSsh => &self.export_ssh,
            KeyAction::ImportSsh => &self.import_ssh,
            KeyAction::ImportTermius => &self.import_termius,
            KeyAction::GroupsManage => &self.groups_manage,
            KeyAction::RenameGroup => &self.rename_group,
            KeyAction::DeleteGroup => &self.delete_group,
            KeyAction::TabHosts => &self.tab_hosts,
            KeyAction::TabSftp => &self.tab_sftp,
            KeyAction::TabTunnels => &self.tab_tunnels,
            KeyAction::TabKeys => &self.tab_keys,
            KeyAction::TabAudit => &self.tab_audit,
            KeyAction::IdentityColumnsInc => &self.identity_columns_inc,
            KeyAction::IdentityColumnsDec => &self.identity_columns_dec,
            KeyAction::AddToAgent => &self.add_to_agent,
            KeyAction::RemoveFromAgent => &self.remove_from_agent,
            KeyAction::PushKey => &self.push_key,
            KeyAction::TunnelKill => &self.tunnel_kill,
            KeyAction::ToggleTunnel => &self.toggle_tunnel,
            KeyAction::AuditFilter => &self.audit_filter,
            KeyAction::AuditRange => &self.audit_range,
            KeyAction::SessionNewTab => &self.session_new_tab,
            KeyAction::SessionCloseTab => &self.session_close_tab,
            KeyAction::SessionTabPrev => &self.session_tab_prev,
            KeyAction::SessionTabNext => &self.session_tab_next,
            KeyAction::SessionDetach => &self.session_detach,
            KeyAction::SessionOpenSftp => &self.session_open_sftp,
            KeyAction::LocalShell => &self.local_shell,
            KeyAction::SessionFocus => &self.session_focus,
            KeyAction::SessionSwitcher => &self.session_switcher,
            KeyAction::SessionScrollUp => &self.session_scroll_up,
            KeyAction::SessionScrollDown => &self.session_scroll_down,
            KeyAction::SessionCancel => &self.session_cancel,
            KeyAction::SessionToggleLog => &self.session_toggle_log,
            KeyAction::ConfirmYes => &self.confirm_yes,
            KeyAction::ConfirmNo => &self.confirm_no,
            KeyAction::Cancel => &self.cancel,
            KeyAction::TogglePanelZoom => &self.toggle_panel_zoom,
            KeyAction::FocusPanelLeft => &self.focus_panel_left,
            KeyAction::FocusPanelRight => &self.focus_panel_right,
            KeyAction::FocusPanelUp => &self.focus_panel_up,
            KeyAction::FocusPanelDown => &self.focus_panel_down,
            KeyAction::Broadcast => &self.broadcast,
            KeyAction::BroadcastCancel => &self.broadcast_cancel,
            KeyAction::KnownHosts => &self.known_hosts,
            KeyAction::KnownHostsDelete => &self.known_hosts_delete,
            KeyAction::KnownHostsRefresh => &self.known_hosts_refresh,
            KeyAction::LogsBrowser => &self.logs_browser,
            KeyAction::SnippetsManage => &self.snippets_manage,
            KeyAction::SessionSnippets => &self.session_snippets,
            KeyAction::ProfilesManage => &self.profiles_manage,
            KeyAction::GhostAccept => &self.ghost_accept,
        }
    }

    pub fn set(&mut self, action: KeyAction, binds: Vec<String>) {
        match action {
            KeyAction::Save => self.save = binds,
            KeyAction::Quit => self.quit = binds,
            KeyAction::Help => self.help = binds,
            KeyAction::Search => self.search = binds,
            KeyAction::KeybindEditor => self.keybind_editor = binds,
            KeyAction::ForceQuit => self.force_quit = binds,
            KeyAction::Connect => self.connect = binds,
            KeyAction::AddHost => self.add_host = binds,
            KeyAction::GenerateKey => self.generate_key = binds,
            KeyAction::Edit => self.edit = binds,
            KeyAction::Delete => self.delete = binds,
            KeyAction::Duplicate => self.duplicate = binds,
            KeyAction::TagFilter => self.tag_filter = binds,
            KeyAction::Favorite => self.favorite = binds,
            KeyAction::ToggleGroup => self.toggle_group = binds,
            KeyAction::FoldGroupIn => self.fold_group_in = binds,
            KeyAction::FoldGroupOut => self.fold_group_out = binds,
            KeyAction::MoveUp => self.move_up = binds,
            KeyAction::MoveDown => self.move_down = binds,
            KeyAction::MoveLeft => self.move_left = binds,
            KeyAction::MoveRight => self.move_right = binds,
            KeyAction::MoveGroupUp => self.move_group_up = binds,
            KeyAction::MoveGroupDown => self.move_group_down = binds,
            KeyAction::MoveHostUp => self.move_host_up = binds,
            KeyAction::MoveHostDown => self.move_host_down = binds,
            KeyAction::CollapseAll => self.collapse_all = binds,
            KeyAction::DetailFocus => self.detail_focus = binds,
            KeyAction::ClearSshLog => self.clear_ssh_log = binds,
            KeyAction::SortCycle => self.sort_cycle = binds,
            KeyAction::YankLog => self.yank_log = binds,
            KeyAction::RevealSecret => self.reveal_secret = binds,
            KeyAction::CopySecret => self.copy_secret = binds,
            KeyAction::UiZoomIn => self.ui_zoom_in = binds,
            KeyAction::UiZoomOut => self.ui_zoom_out = binds,
            KeyAction::ExportSsh => self.export_ssh = binds,
            KeyAction::ImportSsh => self.import_ssh = binds,
            KeyAction::ImportTermius => self.import_termius = binds,
            KeyAction::GroupsManage => self.groups_manage = binds,
            KeyAction::RenameGroup => self.rename_group = binds,
            KeyAction::DeleteGroup => self.delete_group = binds,
            KeyAction::TabHosts => self.tab_hosts = binds,
            KeyAction::TabSftp => self.tab_sftp = binds,
            KeyAction::TabTunnels => self.tab_tunnels = binds,
            KeyAction::TabKeys => self.tab_keys = binds,
            KeyAction::TabAudit => self.tab_audit = binds,
            KeyAction::IdentityColumnsInc => self.identity_columns_inc = binds,
            KeyAction::IdentityColumnsDec => self.identity_columns_dec = binds,
            KeyAction::AddToAgent => self.add_to_agent = binds,
            KeyAction::RemoveFromAgent => self.remove_from_agent = binds,
            KeyAction::PushKey => self.push_key = binds,
            KeyAction::TunnelKill => self.tunnel_kill = binds,
            KeyAction::ToggleTunnel => self.toggle_tunnel = binds,
            KeyAction::AuditFilter => self.audit_filter = binds,
            KeyAction::AuditRange => self.audit_range = binds,
            KeyAction::SessionNewTab => self.session_new_tab = binds,
            KeyAction::SessionCloseTab => self.session_close_tab = binds,
            KeyAction::SessionTabPrev => self.session_tab_prev = binds,
            KeyAction::SessionTabNext => self.session_tab_next = binds,
            KeyAction::SessionDetach => self.session_detach = binds,
            KeyAction::SessionOpenSftp => self.session_open_sftp = binds,
            KeyAction::LocalShell => self.local_shell = binds,
            KeyAction::SessionFocus => self.session_focus = binds,
            KeyAction::SessionSwitcher => self.session_switcher = binds,
            KeyAction::SessionScrollUp => self.session_scroll_up = binds,
            KeyAction::SessionScrollDown => self.session_scroll_down = binds,
            KeyAction::SessionCancel => self.session_cancel = binds,
            KeyAction::SessionToggleLog => self.session_toggle_log = binds,
            KeyAction::ConfirmYes => self.confirm_yes = binds,
            KeyAction::ConfirmNo => self.confirm_no = binds,
            KeyAction::Cancel => self.cancel = binds,
            KeyAction::TogglePanelZoom => self.toggle_panel_zoom = binds,
            KeyAction::FocusPanelLeft => self.focus_panel_left = binds,
            KeyAction::FocusPanelRight => self.focus_panel_right = binds,
            KeyAction::FocusPanelUp => self.focus_panel_up = binds,
            KeyAction::FocusPanelDown => self.focus_panel_down = binds,
            KeyAction::Broadcast => self.broadcast = binds,
            KeyAction::BroadcastCancel => self.broadcast_cancel = binds,
            KeyAction::KnownHosts => self.known_hosts = binds,
            KeyAction::KnownHostsDelete => self.known_hosts_delete = binds,
            KeyAction::KnownHostsRefresh => self.known_hosts_refresh = binds,
            KeyAction::LogsBrowser => self.logs_browser = binds,
            KeyAction::SnippetsManage => self.snippets_manage = binds,
            KeyAction::SessionSnippets => self.session_snippets = binds,
            KeyAction::ProfilesManage => self.profiles_manage = binds,
            KeyAction::GhostAccept => self.ghost_accept = binds,
        }
    }

    /// One-time migration for configs written **before** the SFTP tab was
    /// inserted as tab #2. Those configs pin the old tab digits (tunnels=2,
    /// keys=3, audit=4) and have no `tab_sftp` key, so after the reindex the
    /// digits misroute (2 shadows tunnels, 3→keys, 4→audit) and tunnels becomes
    /// unreachable. We detect the pre-SFTP layout by the absence of a
    /// `tab_sftp` entry in the raw config text and shift each tab's digit up by
    /// one, preserving letter binds (`h`, `i`). Idempotent: once the config is
    /// re-saved with `tab_sftp` present, this is a no-op. Returns whether it
    /// changed anything.
    pub fn migrate_pre_sftp_tabs(&mut self, raw_config: &str) -> bool {
        // A config that already knows the SFTP tab, or never overrode any tab
        // binding, needs no migration (bare defaults are already correct).
        if raw_config.contains("tab_sftp")
            || !(raw_config.contains("tab_tunnels")
                || raw_config.contains("tab_keys")
                || raw_config.contains("tab_audit"))
        {
            return false;
        }
        let shift = |binds: &mut Vec<String>, from: &str, to: &str| {
            for b in binds.iter_mut() {
                if b == from {
                    *b = to.to_string();
                }
            }
        };
        shift(&mut self.tab_tunnels, "2", "3");
        shift(&mut self.tab_keys, "3", "4");
        shift(&mut self.tab_audit, "4", "5");
        if self.tab_sftp.is_empty() {
            self.tab_sftp = vec!["2".to_string()];
        }
        true
    }

    /// Free `H` / `Shift+H` from Help when upgrading to the known-hosts manager.
    ///
    /// Pre-PR installs persist `help = ["?", "Shift+H"]`. The new default
    /// `known_hosts = ["H"]` parses to the same keypress (`Shift` + `h`), and
    /// Help is matched first on the Keys tab — so without this strip the
    /// overlay is unreachable for every upgrading user. Detect the upgrade by
    /// the absence of a `known_hosts` key in the raw config text (same pattern
    /// as [`Self::migrate_pre_sftp_tabs`]). Idempotent once the config is
    /// re-saved with `known_hosts` present.
    pub fn migrate_help_frees_known_hosts(&mut self, raw_config: &str) -> bool {
        if raw_config.contains("known_hosts") {
            return false;
        }
        let before = self.help.clone();
        self.help.retain(|b| {
            let t = b.trim();
            !t.eq_ignore_ascii_case("Shift+H") && t != "H"
        });
        if self.help.is_empty() {
            self.help = vec!["?".to_string()];
        }
        self.help != before
    }

    /// Append `spec` to an action's bindings unless already present.
    pub fn add(&mut self, action: KeyAction, spec: String) {
        let mut binds = self.binds(action).to_vec();
        if !binds.iter().any(|b| b.eq_ignore_ascii_case(&spec)) {
            binds.push(spec);
            self.set(action, binds);
        }
    }
}

fn push_hint(parts: &mut Vec<String>, key: &str, label: &str) {
    if !key.is_empty() {
        parts.push(format!("{key} {label}"));
    }
}

fn push_footer_hint(out: &mut Vec<(String, &'static str)>, key: &str, label: &'static str) {
    if !key.is_empty() {
        out.push((key.to_string(), label));
    }
}

/// `Ctrl+[/]` style key from prev/next tab bindings.
fn tab_switch_keys(kb: &KeybindsConfig) -> Option<String> {
    let prev = kb.primary(KeyAction::SessionTabPrev);
    let next = kb.primary(KeyAction::SessionTabNext);
    match (prev.is_empty(), next.is_empty()) {
        (true, true) => None,
        (false, false) if prev != next => Some(format!("{prev}/{next}")),
        (false, _) => Some(prev.to_string()),
        (_, false) => Some(next.to_string()),
    }
}

/// `Ctrl+[/] tabs` style hint from prev/next tab bindings.
fn tab_switch_hint(kb: &KeybindsConfig) -> Option<String> {
    tab_switch_keys(kb).map(|keys| format!("{keys} tabs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_pre_sftp_shifts_tab_digits() {
        // A pre-SFTP config: old digits, no tab_sftp key.
        let raw =
            "[keybinds]\ntab_tunnels = [\"2\"]\ntab_keys = [\"i\", \"3\"]\ntab_audit = [\"4\"]\n";
        let mut kb = KeybindsConfig {
            tab_tunnels: vec!["2".into()],
            tab_keys: vec!["i".into(), "3".into()],
            tab_audit: vec!["4".into()],
            ..KeybindsConfig::default()
        };
        assert!(kb.migrate_pre_sftp_tabs(raw));
        assert_eq!(kb.tab_tunnels, vec!["3"]);
        assert_eq!(kb.tab_keys, vec!["i", "4"]);
        assert_eq!(kb.tab_audit, vec!["5"]);
        assert_eq!(kb.tab_sftp, vec!["2"]);
    }

    #[test]
    fn migrate_pre_sftp_is_noop_for_new_configs() {
        // Config already mentions tab_sftp → already migrated / new.
        let raw = "[keybinds]\ntab_sftp = [\"2\"]\ntab_tunnels = [\"3\"]\n";
        let mut kb = KeybindsConfig::default();
        let before = kb.tab_tunnels.clone();
        assert!(!kb.migrate_pre_sftp_tabs(raw));
        assert_eq!(kb.tab_tunnels, before);
    }

    #[test]
    fn migrate_help_strips_shift_h_on_upgrade() {
        let raw = "[keybinds]\nhelp = [\"?\", \"Shift+H\"]\n";
        let mut kb = KeybindsConfig {
            help: vec!["?".into(), "Shift+H".into()],
            ..KeybindsConfig::default()
        };
        assert!(kb.migrate_help_frees_known_hosts(raw));
        assert_eq!(kb.help, vec!["?"]);
    }

    #[test]
    fn migrate_help_is_noop_when_known_hosts_present() {
        let raw = "[keybinds]\nhelp = [\"?\", \"Shift+H\"]\nknown_hosts = [\"H\"]\n";
        let mut kb = KeybindsConfig {
            help: vec!["?".into(), "Shift+H".into()],
            known_hosts: vec!["H".into()],
            ..KeybindsConfig::default()
        };
        assert!(!kb.migrate_help_frees_known_hosts(raw));
        assert_eq!(kb.help, vec!["?", "Shift+H"]);
    }

    #[test]
    fn session_footer_hints_lead_with_resume() {
        let kb = KeybindsConfig::default();
        let hints = kb.session_footer_hints();
        assert_eq!(
            hints.first().map(|(keys, label)| (keys.as_str(), *label)),
            Some((kb.primary(KeyAction::SessionFocus), "resume")),
            "getting back into a running session comes first"
        );

        // The key is read from the config rather than hardcoded, so a rebind
        // shows up here and the old default stops being advertised.
        let mut custom = KeybindsConfig::default();
        custom.set(KeyAction::SessionFocus, vec!["F8".into()]);
        let hints = custom.session_footer_hints();
        assert_eq!(hints[0], ("F8".to_string(), "resume"));
        assert!(!hints.iter().any(|(keys, _)| keys == "Ctrl+Shift+S"));
        assert!(
            !hints.iter().any(|(_, label)| *label == "detach"),
            "detach belongs on the in-session header, not the dashboard footer"
        );
    }

    #[test]
    fn session_header_hints_use_configured_binds() {
        let kb = KeybindsConfig {
            session_new_tab: vec!["F9".into()],
            session_detach: vec!["Ctrl+Shift+D".into()],
            ..Default::default()
        };
        assert_eq!(
            kb.session_header_hints(false),
            "F9 new tab  Ctrl+Shift+D detach  Alt+S switch"
        );
    }

    #[test]
    fn session_header_hints_multi_tab_pair() {
        let kb = KeybindsConfig::default();
        let hints = kb.session_header_hints(true);
        assert!(hints.contains("Ctrl+T new"));
        assert!(hints.contains("Ctrl+D detach"));
        assert!(hints.contains("Ctrl+[/Ctrl+] tabs"));
    }

    #[test]
    fn an_empty_table_loads_every_default() {
        let cfg: KeybindsConfig = toml::from_str("").unwrap();
        let defaults = KeybindsConfig::default();
        for action in KeyAction::ALL {
            assert!(
                !defaults.binds(action).is_empty(),
                "{action:?} has no default"
            );
            assert_eq!(cfg.binds(action), defaults.binds(action), "{action:?}");
            assert_eq!(KeybindsConfig::default_for(action), defaults.binds(action));
        }
    }

    #[test]
    fn session_switcher_defaults_and_roundtrips() {
        // A config written before this action existed must still load, and pick
        // up the default rather than an empty bind list.
        let cfg: KeybindsConfig = toml::from_str("quit = [\"q\"]").unwrap();
        assert_eq!(cfg.session_switcher, vec!["Alt+S".to_string()]);
        assert!(KeyAction::ALL.contains(&KeyAction::SessionSwitcher));

        let mut custom = KeybindsConfig::default();
        custom.set(KeyAction::SessionSwitcher, vec!["F7".into()]);
        let text = toml::to_string(&custom).unwrap();
        let back: KeybindsConfig = toml::from_str(&text).unwrap();
        assert_eq!(back.session_switcher, vec!["F7".to_string()]);
    }

    #[test]
    fn alt_s_spec_matches_the_measured_event() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        // "Alt+S" must not pick up SHIFT: the uppercase-means-shift rule only
        // applies when no modifier was spelled out. The terminal reports
        // Char('s') + ALT, so that is what the spec has to parse to.
        let (code, mods) = crate::app::parse_keyspec("Alt+S").unwrap();
        assert_eq!(code, KeyCode::Char('s'));
        assert_eq!(mods, KeyModifiers::ALT);

        // The inverse uppercases letters that carry a modifier, so it round
        // trips to the canonical spelling rather than "Alt+s".
        let ev = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT);
        assert_eq!(crate::app::keyevent_to_spec(&ev).as_deref(), Some("Alt+S"));
    }

    #[test]
    fn switcher_hotkey_appears_in_both_hint_rows() {
        let mut kb = KeybindsConfig::default();
        kb.set(KeyAction::SessionSwitcher, vec!["F7".into()]);
        // Single tab and multi tab: the switcher is useful either way.
        assert!(kb.session_header_hints(false).contains("F7"));
        assert!(kb.session_header_hints(true).contains("F7"));
        assert!(kb
            .session_footer_hints()
            .iter()
            .any(|(keys, _)| keys.contains("F7")));
    }

    #[test]
    fn migrate_pre_sftp_is_noop_without_tab_overrides() {
        // A config that never customised tab binds needs nothing shifted.
        let raw = "[keybinds]\nquit = [\"q\"]\n";
        let mut kb = KeybindsConfig::default();
        assert!(!kb.migrate_pre_sftp_tabs(raw));
    }

    #[test]
    fn ghost_accept_defaults_and_roundtrips() {
        // Oracle: none exists (config defaults are sshub's own invention);
        // this pins the contract the session key handler relies on: one
        // action, still 85 total (84 + ProfilesManage), Ctrl+F out of the
        // box, and old `session_suggestions` keys ignored rather than fatal.
        assert_eq!(KeyAction::ALL.len(), 85, "picker swap must not add actions");
        assert!(KeyAction::ALL.contains(&KeyAction::GhostAccept));
        let kb = KeybindsConfig::default();
        assert_eq!(kb.primary(KeyAction::GhostAccept), "Ctrl+F");
        // A config written for the picker still loads (unknown keys ignored).
        let cfg: KeybindsConfig = toml::from_str("session_suggestions = [\"Ctrl+Space\"]").unwrap();
        assert_eq!(cfg.primary(KeyAction::GhostAccept), "Ctrl+F");
        // Round trip through TOML under the new field name.
        let mut custom = KeybindsConfig::default();
        custom.set(KeyAction::GhostAccept, vec!["F9".into()]);
        let text = toml::to_string(&custom).unwrap();
        let back: KeybindsConfig = toml::from_str(&text).unwrap();
        assert_eq!(back.ghost_accept, vec!["F9".to_string()]);
    }
}
