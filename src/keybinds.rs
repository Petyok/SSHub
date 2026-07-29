//! User-remappable keybindings.
//!
//! Each entry is a list of key specs so a user can add their own binding
//! without losing the defaults. Specs look like `"F2"`, `"Ctrl+S"`,
//! `"Alt+Enter"`, `"F10"` (parsed in [`crate::app::util::parse_keyspec`]).

use serde::{Deserialize, Serialize};

macro_rules! kb_defaults {
    ($($field:ident => [$($key:literal),* $(,)?]),* $(,)?) => {
        $(kb_defaults! { @fn $field $($key),* })*
    };
    (@fn save $($key:literal),* $(,)?) => {
        fn default_kb_save() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn quit $($key:literal),* $(,)?) => {
        fn default_kb_quit() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn help $($key:literal),* $(,)?) => {
        fn default_kb_help() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn search $($key:literal),* $(,)?) => {
        fn default_kb_search() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn keybind_editor $($key:literal),* $(,)?) => {
        fn default_kb_keybind_editor() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn force_quit $($key:literal),* $(,)?) => {
        fn default_kb_force_quit() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn connect $($key:literal),* $(,)?) => {
        fn default_kb_connect() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn add_host $($key:literal),* $(,)?) => {
        fn default_kb_add_host() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn generate_key $($key:literal),* $(,)?) => {
        fn default_kb_generate_key() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn edit $($key:literal),* $(,)?) => {
        fn default_kb_edit() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn delete $($key:literal),* $(,)?) => {
        fn default_kb_delete() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn duplicate $($key:literal),* $(,)?) => {
        fn default_kb_duplicate() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn tag_filter $($key:literal),* $(,)?) => {
        fn default_kb_tag_filter() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn favorite $($key:literal),* $(,)?) => {
        fn default_kb_favorite() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn toggle_group $($key:literal),* $(,)?) => {
        fn default_kb_toggle_group() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn fold_group_in $($key:literal),* $(,)?) => {
        fn default_kb_fold_group_in() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn fold_group_out $($key:literal),* $(,)?) => {
        fn default_kb_fold_group_out() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn move_up $($key:literal),* $(,)?) => {
        fn default_kb_move_up() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn move_down $($key:literal),* $(,)?) => {
        fn default_kb_move_down() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn move_left $($key:literal),* $(,)?) => {
        fn default_kb_move_left() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn move_right $($key:literal),* $(,)?) => {
        fn default_kb_move_right() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn move_group_up $($key:literal),* $(,)?) => {
        fn default_kb_move_group_up() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn move_group_down $($key:literal),* $(,)?) => {
        fn default_kb_move_group_down() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn move_host_up $($key:literal),* $(,)?) => {
        fn default_kb_move_host_up() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn move_host_down $($key:literal),* $(,)?) => {
        fn default_kb_move_host_down() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn collapse_all $($key:literal),* $(,)?) => {
        fn default_kb_collapse_all() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn detail_focus $($key:literal),* $(,)?) => {
        fn default_kb_detail_focus() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn clear_ssh_log $($key:literal),* $(,)?) => {
        fn default_kb_clear_ssh_log() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn sort_cycle $($key:literal),* $(,)?) => {
        fn default_kb_sort_cycle() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn yank_log $($key:literal),* $(,)?) => {
        fn default_kb_yank_log() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn reveal_secret $($key:literal),* $(,)?) => {
        fn default_kb_reveal_secret() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn copy_secret $($key:literal),* $(,)?) => {
        fn default_kb_copy_secret() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn ui_zoom_in $($key:literal),* $(,)?) => {
        fn default_kb_ui_zoom_in() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn ui_zoom_out $($key:literal),* $(,)?) => {
        fn default_kb_ui_zoom_out() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn export_ssh $($key:literal),* $(,)?) => {
        fn default_kb_export_ssh() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn import_ssh $($key:literal),* $(,)?) => {
        fn default_kb_import_ssh() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn import_termius $($key:literal),* $(,)?) => {
        fn default_kb_import_termius() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn groups_manage $($key:literal),* $(,)?) => {
        fn default_kb_groups_manage() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn rename_group $($key:literal),* $(,)?) => {
        fn default_kb_rename_group() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn delete_group $($key:literal),* $(,)?) => {
        fn default_kb_delete_group() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn tab_hosts $($key:literal),* $(,)?) => {
        fn default_kb_tab_hosts() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn tab_sftp $($key:literal),* $(,)?) => {
        fn default_kb_tab_sftp() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn tab_tunnels $($key:literal),* $(,)?) => {
        fn default_kb_tab_tunnels() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn tab_keys $($key:literal),* $(,)?) => {
        fn default_kb_tab_keys() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn tab_audit $($key:literal),* $(,)?) => {
        fn default_kb_tab_audit() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn identity_columns_inc $($key:literal),* $(,)?) => {
        fn default_kb_identity_columns_inc() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn identity_columns_dec $($key:literal),* $(,)?) => {
        fn default_kb_identity_columns_dec() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn add_to_agent $($key:literal),* $(,)?) => {
        fn default_kb_add_to_agent() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn remove_from_agent $($key:literal),* $(,)?) => {
        fn default_kb_remove_from_agent() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn push_key $($key:literal),* $(,)?) => {
        fn default_kb_push_key() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn tunnel_kill $($key:literal),* $(,)?) => {
        fn default_kb_tunnel_kill() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn toggle_tunnel $($key:literal),* $(,)?) => {
        fn default_kb_toggle_tunnel() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn audit_filter $($key:literal),* $(,)?) => {
        fn default_kb_audit_filter() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn audit_range $($key:literal),* $(,)?) => {
        fn default_kb_audit_range() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn session_new_tab $($key:literal),* $(,)?) => {
        fn default_kb_session_new_tab() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn session_close_tab $($key:literal),* $(,)?) => {
        fn default_kb_session_close_tab() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn session_tab_prev $($key:literal),* $(,)?) => {
        fn default_kb_session_tab_prev() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn session_tab_next $($key:literal),* $(,)?) => {
        fn default_kb_session_tab_next() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn session_detach $($key:literal),* $(,)?) => {
        fn default_kb_session_detach() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn session_open_sftp $($key:literal),* $(,)?) => {
        fn default_kb_session_open_sftp() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn local_shell $($key:literal),* $(,)?) => {
        fn default_kb_local_shell() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn session_focus $($key:literal),* $(,)?) => {
        fn default_kb_session_focus() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn session_scroll_up $($key:literal),* $(,)?) => {
        fn default_kb_session_scroll_up() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn session_scroll_down $($key:literal),* $(,)?) => {
        fn default_kb_session_scroll_down() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn session_cancel $($key:literal),* $(,)?) => {
        fn default_kb_session_cancel() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn session_toggle_log $($key:literal),* $(,)?) => {
        fn default_kb_session_toggle_log() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn confirm_yes $($key:literal),* $(,)?) => {
        fn default_kb_confirm_yes() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn confirm_no $($key:literal),* $(,)?) => {
        fn default_kb_confirm_no() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn cancel $($key:literal),* $(,)?) => {
        fn default_kb_cancel() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn toggle_panel_zoom $($key:literal),* $(,)?) => {
        fn default_kb_toggle_panel_zoom() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn focus_panel_left $($key:literal),* $(,)?) => {
        fn default_kb_focus_panel_left() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn focus_panel_right $($key:literal),* $(,)?) => {
        fn default_kb_focus_panel_right() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn focus_panel_up $($key:literal),* $(,)?) => {
        fn default_kb_focus_panel_up() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn focus_panel_down $($key:literal),* $(,)?) => {
        fn default_kb_focus_panel_down() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn broadcast $($key:literal),* $(,)?) => {
        fn default_kb_broadcast() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
    (@fn broadcast_cancel $($key:literal),* $(,)?) => {
        fn default_kb_broadcast_cancel() -> Vec<String> {
            vec![$($key.to_string()),*]
        }
    };
}

kb_defaults! {
    save => ["F2", "Ctrl+S"],
    quit => ["q"],
    help => ["?", "Shift+H"],
    search => ["/"],
    keybind_editor => ["Ctrl+K"],
    force_quit => ["Ctrl+C"],
    connect => ["Enter"],
    add_host => ["a"],
    generate_key => ["g"],
    edit => ["e"],
    delete => ["d"],
    duplicate => ["Shift+D"],
    tag_filter => ["#"],
    favorite => ["f"],
    toggle_group => ["Space"],
    fold_group_in => ["Left"],
    fold_group_out => ["Right"],
    move_up => ["k", "Up"],
    move_down => ["j", "Down"],
    move_left => ["Left"],
    move_right => ["l", "Right"],
    move_group_up => ["Shift+Up"],
    move_group_down => ["Shift+Down"],
    move_host_up => ["Ctrl+Up"],
    move_host_down => ["Ctrl+Down"],
    collapse_all => ["Shift+Z"],
    detail_focus => ["Tab"],
    clear_ssh_log => ["c"],
    sort_cycle => ["s"],
    yank_log => ["y"],
    reveal_secret => ["Ctrl+R"],
    copy_secret => ["Ctrl+Y"],
    ui_zoom_in => ["+", "="],
    ui_zoom_out => ["-", "_"],
    export_ssh => ["Shift+E"],
    import_ssh => ["Shift+I"],
    import_termius => ["Shift+T"],
    groups_manage => ["Shift+G"],
    rename_group => ["Ctrl+G"],
    delete_group => ["Ctrl+Shift+G"],
    tab_hosts => ["h", "1"],
    tab_sftp => ["2"],
    tab_tunnels => ["3"],
    tab_keys => ["i", "4"],
    tab_audit => ["5"],
    identity_columns_inc => ["]"],
    identity_columns_dec => ["["],
    add_to_agent => ["p"],
    remove_from_agent => ["r"],
    push_key => ["Shift+P"],
    tunnel_kill => ["x"],
    toggle_tunnel => ["Enter"],
    audit_filter => ["f"],
    audit_range => ["r"],
    session_new_tab => ["Ctrl+T"],
    session_close_tab => ["Ctrl+W"],
    session_tab_prev => ["Ctrl+[", "Ctrl+PageUp"],
    session_tab_next => ["Ctrl+]", "Ctrl+PageDown"],
    session_detach => ["Ctrl+D"],
    session_open_sftp => ["Ctrl+Shift+F"],
    local_shell => ["Ctrl+Shift+T"],
    session_focus => ["Ctrl+Shift+S"],
    session_scroll_up => ["PageUp"],
    session_scroll_down => ["PageDown"],
    session_cancel => ["Esc"],
    session_toggle_log => ["Ctrl+O"],
    confirm_yes => ["y", "Y", "Enter"],
    confirm_no => ["n", "N"],
    cancel => ["Esc"],
    toggle_panel_zoom => ["z", "Alt+Enter"],
    focus_panel_left => ["Alt+Left"],
    focus_panel_right => ["Alt+Right"],
    focus_panel_up => ["Alt+Up"],
    focus_panel_down => ["Alt+Down"],
    broadcast => ["b"],
    broadcast_cancel => ["x"],
}
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
}

impl KeyAction {
    /// All editable actions, in display order.
    pub const ALL: [KeyAction; 76] = [
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
        }
    }
}

/// User-remappable keybindings. Each field is a list of key specs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindsConfig {
    #[serde(default = "default_kb_save")]
    pub save: Vec<String>,
    #[serde(default = "default_kb_quit")]
    pub quit: Vec<String>,
    #[serde(default = "default_kb_help")]
    pub help: Vec<String>,
    #[serde(default = "default_kb_search")]
    pub search: Vec<String>,
    #[serde(default = "default_kb_keybind_editor")]
    pub keybind_editor: Vec<String>,
    #[serde(default = "default_kb_force_quit")]
    pub force_quit: Vec<String>,
    #[serde(default = "default_kb_connect")]
    pub connect: Vec<String>,
    #[serde(default = "default_kb_add_host")]
    pub add_host: Vec<String>,
    #[serde(default = "default_kb_generate_key")]
    pub generate_key: Vec<String>,
    #[serde(default = "default_kb_edit")]
    pub edit: Vec<String>,
    #[serde(default = "default_kb_delete")]
    pub delete: Vec<String>,
    #[serde(default = "default_kb_duplicate")]
    pub duplicate: Vec<String>,
    #[serde(default = "default_kb_tag_filter")]
    pub tag_filter: Vec<String>,
    #[serde(default = "default_kb_favorite")]
    pub favorite: Vec<String>,
    #[serde(default = "default_kb_toggle_group")]
    pub toggle_group: Vec<String>,
    #[serde(default = "default_kb_fold_group_in")]
    pub fold_group_in: Vec<String>,
    #[serde(default = "default_kb_fold_group_out")]
    pub fold_group_out: Vec<String>,
    #[serde(default = "default_kb_move_up")]
    pub move_up: Vec<String>,
    #[serde(default = "default_kb_move_down")]
    pub move_down: Vec<String>,
    #[serde(default = "default_kb_move_left")]
    pub move_left: Vec<String>,
    #[serde(default = "default_kb_move_right")]
    pub move_right: Vec<String>,
    #[serde(default = "default_kb_move_group_up")]
    pub move_group_up: Vec<String>,
    #[serde(default = "default_kb_move_group_down")]
    pub move_group_down: Vec<String>,
    #[serde(default = "default_kb_move_host_up")]
    pub move_host_up: Vec<String>,
    #[serde(default = "default_kb_move_host_down")]
    pub move_host_down: Vec<String>,
    #[serde(default = "default_kb_collapse_all")]
    pub collapse_all: Vec<String>,
    #[serde(default = "default_kb_detail_focus")]
    pub detail_focus: Vec<String>,
    #[serde(default = "default_kb_clear_ssh_log")]
    pub clear_ssh_log: Vec<String>,
    #[serde(default = "default_kb_sort_cycle")]
    pub sort_cycle: Vec<String>,
    #[serde(default = "default_kb_yank_log")]
    pub yank_log: Vec<String>,
    #[serde(default = "default_kb_reveal_secret")]
    pub reveal_secret: Vec<String>,
    #[serde(default = "default_kb_copy_secret")]
    pub copy_secret: Vec<String>,
    #[serde(default = "default_kb_ui_zoom_in")]
    pub ui_zoom_in: Vec<String>,
    #[serde(default = "default_kb_ui_zoom_out")]
    pub ui_zoom_out: Vec<String>,
    #[serde(default = "default_kb_export_ssh")]
    pub export_ssh: Vec<String>,
    #[serde(default = "default_kb_import_ssh")]
    pub import_ssh: Vec<String>,
    #[serde(default = "default_kb_import_termius")]
    pub import_termius: Vec<String>,
    #[serde(default = "default_kb_groups_manage")]
    pub groups_manage: Vec<String>,
    #[serde(default = "default_kb_rename_group")]
    pub rename_group: Vec<String>,
    #[serde(default = "default_kb_delete_group")]
    pub delete_group: Vec<String>,
    #[serde(default = "default_kb_tab_hosts")]
    pub tab_hosts: Vec<String>,
    #[serde(default = "default_kb_tab_sftp")]
    pub tab_sftp: Vec<String>,
    #[serde(default = "default_kb_tab_tunnels")]
    pub tab_tunnels: Vec<String>,
    #[serde(default = "default_kb_tab_keys")]
    pub tab_keys: Vec<String>,
    #[serde(default = "default_kb_tab_audit")]
    pub tab_audit: Vec<String>,
    #[serde(default = "default_kb_identity_columns_inc")]
    pub identity_columns_inc: Vec<String>,
    #[serde(default = "default_kb_identity_columns_dec")]
    pub identity_columns_dec: Vec<String>,
    #[serde(default = "default_kb_add_to_agent")]
    pub add_to_agent: Vec<String>,
    #[serde(default = "default_kb_remove_from_agent")]
    pub remove_from_agent: Vec<String>,
    #[serde(default = "default_kb_push_key")]
    pub push_key: Vec<String>,
    #[serde(default = "default_kb_tunnel_kill")]
    pub tunnel_kill: Vec<String>,
    #[serde(default = "default_kb_toggle_tunnel")]
    pub toggle_tunnel: Vec<String>,
    #[serde(default = "default_kb_audit_filter")]
    pub audit_filter: Vec<String>,
    #[serde(default = "default_kb_audit_range")]
    pub audit_range: Vec<String>,
    #[serde(default = "default_kb_session_new_tab")]
    pub session_new_tab: Vec<String>,
    #[serde(default = "default_kb_session_close_tab")]
    pub session_close_tab: Vec<String>,
    #[serde(default = "default_kb_session_tab_prev")]
    pub session_tab_prev: Vec<String>,
    #[serde(default = "default_kb_session_tab_next")]
    pub session_tab_next: Vec<String>,
    #[serde(default = "default_kb_session_detach")]
    pub session_detach: Vec<String>,
    #[serde(default = "default_kb_session_open_sftp")]
    pub session_open_sftp: Vec<String>,
    #[serde(default = "default_kb_local_shell")]
    pub local_shell: Vec<String>,
    #[serde(default = "default_kb_session_focus")]
    pub session_focus: Vec<String>,
    #[serde(default = "default_kb_session_scroll_up")]
    pub session_scroll_up: Vec<String>,
    #[serde(default = "default_kb_session_scroll_down")]
    pub session_scroll_down: Vec<String>,
    #[serde(default = "default_kb_session_cancel")]
    pub session_cancel: Vec<String>,
    #[serde(default = "default_kb_session_toggle_log")]
    pub session_toggle_log: Vec<String>,
    #[serde(default = "default_kb_confirm_yes")]
    pub confirm_yes: Vec<String>,
    #[serde(default = "default_kb_confirm_no")]
    pub confirm_no: Vec<String>,
    #[serde(default = "default_kb_cancel")]
    pub cancel: Vec<String>,
    #[serde(default = "default_kb_toggle_panel_zoom")]
    pub toggle_panel_zoom: Vec<String>,
    #[serde(default = "default_kb_focus_panel_left")]
    pub focus_panel_left: Vec<String>,
    #[serde(default = "default_kb_focus_panel_right")]
    pub focus_panel_right: Vec<String>,
    #[serde(default = "default_kb_focus_panel_up")]
    pub focus_panel_up: Vec<String>,
    #[serde(default = "default_kb_focus_panel_down")]
    pub focus_panel_down: Vec<String>,
    #[serde(default = "default_kb_broadcast")]
    pub broadcast: Vec<String>,
    #[serde(default = "default_kb_broadcast_cancel")]
    pub broadcast_cancel: Vec<String>,
}

impl Default for KeybindsConfig {
    fn default() -> Self {
        Self {
            save: default_kb_save(),
            quit: default_kb_quit(),
            help: default_kb_help(),
            search: default_kb_search(),
            keybind_editor: default_kb_keybind_editor(),
            force_quit: default_kb_force_quit(),
            connect: default_kb_connect(),
            add_host: default_kb_add_host(),
            generate_key: default_kb_generate_key(),
            edit: default_kb_edit(),
            delete: default_kb_delete(),
            duplicate: default_kb_duplicate(),
            tag_filter: default_kb_tag_filter(),
            favorite: default_kb_favorite(),
            toggle_group: default_kb_toggle_group(),
            fold_group_in: default_kb_fold_group_in(),
            fold_group_out: default_kb_fold_group_out(),
            move_up: default_kb_move_up(),
            move_down: default_kb_move_down(),
            move_left: default_kb_move_left(),
            move_right: default_kb_move_right(),
            move_group_up: default_kb_move_group_up(),
            move_group_down: default_kb_move_group_down(),
            move_host_up: default_kb_move_host_up(),
            move_host_down: default_kb_move_host_down(),
            collapse_all: default_kb_collapse_all(),
            detail_focus: default_kb_detail_focus(),
            clear_ssh_log: default_kb_clear_ssh_log(),
            sort_cycle: default_kb_sort_cycle(),
            yank_log: default_kb_yank_log(),
            reveal_secret: default_kb_reveal_secret(),
            copy_secret: default_kb_copy_secret(),
            ui_zoom_in: default_kb_ui_zoom_in(),
            ui_zoom_out: default_kb_ui_zoom_out(),
            export_ssh: default_kb_export_ssh(),
            import_ssh: default_kb_import_ssh(),
            import_termius: default_kb_import_termius(),
            groups_manage: default_kb_groups_manage(),
            rename_group: default_kb_rename_group(),
            delete_group: default_kb_delete_group(),
            tab_hosts: default_kb_tab_hosts(),
            tab_sftp: default_kb_tab_sftp(),
            tab_tunnels: default_kb_tab_tunnels(),
            tab_keys: default_kb_tab_keys(),
            tab_audit: default_kb_tab_audit(),
            identity_columns_inc: default_kb_identity_columns_inc(),
            identity_columns_dec: default_kb_identity_columns_dec(),
            add_to_agent: default_kb_add_to_agent(),
            remove_from_agent: default_kb_remove_from_agent(),
            push_key: default_kb_push_key(),
            tunnel_kill: default_kb_tunnel_kill(),
            toggle_tunnel: default_kb_toggle_tunnel(),
            audit_filter: default_kb_audit_filter(),
            audit_range: default_kb_audit_range(),
            session_new_tab: default_kb_session_new_tab(),
            session_close_tab: default_kb_session_close_tab(),
            session_tab_prev: default_kb_session_tab_prev(),
            session_tab_next: default_kb_session_tab_next(),
            session_detach: default_kb_session_detach(),
            session_open_sftp: default_kb_session_open_sftp(),
            local_shell: default_kb_local_shell(),
            session_focus: default_kb_session_focus(),
            session_scroll_up: default_kb_session_scroll_up(),
            session_scroll_down: default_kb_session_scroll_down(),
            session_cancel: default_kb_session_cancel(),
            session_toggle_log: default_kb_session_toggle_log(),
            confirm_yes: default_kb_confirm_yes(),
            confirm_no: default_kb_confirm_no(),
            cancel: default_kb_cancel(),
            toggle_panel_zoom: default_kb_toggle_panel_zoom(),
            focus_panel_left: default_kb_focus_panel_left(),
            focus_panel_right: default_kb_focus_panel_right(),
            focus_panel_up: default_kb_focus_panel_up(),
            focus_panel_down: default_kb_focus_panel_down(),
            broadcast: default_kb_broadcast(),
            broadcast_cancel: default_kb_broadcast_cancel(),
        }
    }
}

impl KeybindsConfig {
    fn default_for(action: KeyAction) -> Vec<String> {
        match action {
            KeyAction::Save => default_kb_save(),
            KeyAction::Quit => default_kb_quit(),
            KeyAction::Help => default_kb_help(),
            KeyAction::Search => default_kb_search(),
            KeyAction::KeybindEditor => default_kb_keybind_editor(),
            KeyAction::ForceQuit => default_kb_force_quit(),
            KeyAction::Connect => default_kb_connect(),
            KeyAction::AddHost => default_kb_add_host(),
            KeyAction::GenerateKey => default_kb_generate_key(),
            KeyAction::Edit => default_kb_edit(),
            KeyAction::Delete => default_kb_delete(),
            KeyAction::Duplicate => default_kb_duplicate(),
            KeyAction::TagFilter => default_kb_tag_filter(),
            KeyAction::Favorite => default_kb_favorite(),
            KeyAction::ToggleGroup => default_kb_toggle_group(),
            KeyAction::FoldGroupIn => default_kb_fold_group_in(),
            KeyAction::FoldGroupOut => default_kb_fold_group_out(),
            KeyAction::MoveUp => default_kb_move_up(),
            KeyAction::MoveDown => default_kb_move_down(),
            KeyAction::MoveLeft => default_kb_move_left(),
            KeyAction::MoveRight => default_kb_move_right(),
            KeyAction::MoveGroupUp => default_kb_move_group_up(),
            KeyAction::MoveGroupDown => default_kb_move_group_down(),
            KeyAction::MoveHostUp => default_kb_move_host_up(),
            KeyAction::MoveHostDown => default_kb_move_host_down(),
            KeyAction::CollapseAll => default_kb_collapse_all(),
            KeyAction::DetailFocus => default_kb_detail_focus(),
            KeyAction::ClearSshLog => default_kb_clear_ssh_log(),
            KeyAction::SortCycle => default_kb_sort_cycle(),
            KeyAction::YankLog => default_kb_yank_log(),
            KeyAction::RevealSecret => default_kb_reveal_secret(),
            KeyAction::CopySecret => default_kb_copy_secret(),
            KeyAction::UiZoomIn => default_kb_ui_zoom_in(),
            KeyAction::UiZoomOut => default_kb_ui_zoom_out(),
            KeyAction::ExportSsh => default_kb_export_ssh(),
            KeyAction::ImportSsh => default_kb_import_ssh(),
            KeyAction::ImportTermius => default_kb_import_termius(),
            KeyAction::GroupsManage => default_kb_groups_manage(),
            KeyAction::RenameGroup => default_kb_rename_group(),
            KeyAction::DeleteGroup => default_kb_delete_group(),
            KeyAction::TabHosts => default_kb_tab_hosts(),
            KeyAction::TabSftp => default_kb_tab_sftp(),
            KeyAction::TabTunnels => default_kb_tab_tunnels(),
            KeyAction::TabKeys => default_kb_tab_keys(),
            KeyAction::TabAudit => default_kb_tab_audit(),
            KeyAction::IdentityColumnsInc => default_kb_identity_columns_inc(),
            KeyAction::IdentityColumnsDec => default_kb_identity_columns_dec(),
            KeyAction::AddToAgent => default_kb_add_to_agent(),
            KeyAction::RemoveFromAgent => default_kb_remove_from_agent(),
            KeyAction::PushKey => default_kb_push_key(),
            KeyAction::TunnelKill => default_kb_tunnel_kill(),
            KeyAction::ToggleTunnel => default_kb_toggle_tunnel(),
            KeyAction::AuditFilter => default_kb_audit_filter(),
            KeyAction::AuditRange => default_kb_audit_range(),
            KeyAction::SessionNewTab => default_kb_session_new_tab(),
            KeyAction::SessionCloseTab => default_kb_session_close_tab(),
            KeyAction::SessionTabPrev => default_kb_session_tab_prev(),
            KeyAction::SessionTabNext => default_kb_session_tab_next(),
            KeyAction::SessionDetach => default_kb_session_detach(),
            KeyAction::SessionOpenSftp => default_kb_session_open_sftp(),
            KeyAction::LocalShell => default_kb_local_shell(),
            KeyAction::SessionFocus => default_kb_session_focus(),
            KeyAction::SessionScrollUp => default_kb_session_scroll_up(),
            KeyAction::SessionScrollDown => default_kb_session_scroll_down(),
            KeyAction::SessionCancel => default_kb_session_cancel(),
            KeyAction::SessionToggleLog => default_kb_session_toggle_log(),
            KeyAction::ConfirmYes => default_kb_confirm_yes(),
            KeyAction::ConfirmNo => default_kb_confirm_no(),
            KeyAction::Cancel => default_kb_cancel(),
            KeyAction::TogglePanelZoom => default_kb_toggle_panel_zoom(),
            KeyAction::FocusPanelLeft => default_kb_focus_panel_left(),
            KeyAction::FocusPanelRight => default_kb_focus_panel_right(),
            KeyAction::FocusPanelUp => default_kb_focus_panel_up(),
            KeyAction::FocusPanelDown => default_kb_focus_panel_down(),
            KeyAction::Broadcast => default_kb_broadcast(),
            KeyAction::BroadcastCancel => default_kb_broadcast_cancel(),
        }
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
            "F9 new tab  Ctrl+Shift+D detach"
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
    fn migrate_pre_sftp_is_noop_without_tab_overrides() {
        // A config that never customised tab binds needs nothing shifted.
        let raw = "[keybinds]\nquit = [\"q\"]\n";
        let mut kb = KeybindsConfig::default();
        assert!(!kb.migrate_pre_sftp_tabs(raw));
    }
}
