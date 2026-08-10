//! The frozen V1 role catalogue.
//!
//! Every public component role, its value type and its semantic fallback are
//! declared exactly once, in the [`role_catalog!`] invocation at the bottom of
//! this file. The typed enums, their `COUNT` constants and [`ROLE_SPECS`] are
//! all generated from that single declaration, so validator, resolver and
//! documentation can never drift apart from what the renderers actually index.

/// One slot of the fixed semantic core of schema version 1.
///
/// The 25 slots are the only names a component fallback may reference; a theme
/// that overrides one of them re-tints everything inheriting from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SemanticSlot {
    Background,
    Canvas,
    Surface,
    SurfaceRaised,
    Border,
    BorderFocus,
    BorderPopup,
    Text,
    TextBright,
    TextHighlight,
    TextMuted,
    TextDim,
    TextInverse,
    Accent,
    SelectionBg,
    SelectionFg,
    Success,
    Warning,
    Error,
    Info,
    Connecting,
    Exited,
    Unknown,
    /// The ground painted under the embedded remote grid, and the default
    /// foreground written into it. Appended after the original 23 so every
    /// existing `as usize` index keeps its value.
    PtyBackground,
    PtyForeground,
}

impl SemanticSlot {
    /// The name used in `[semantic]` tables and in diagnostics.
    pub fn key(self) -> &'static str {
        SEMANTIC_SPECS[self as usize].key
    }
}

/// A semantic slot used as the fallback of a `Color` role.
pub type SemanticColor = SemanticSlot;

/// A semantic slot used as the fallback of a `Paint` role.
pub type SemanticPaint = SemanticSlot;

/// The fallback of a `Style` role.
///
/// Style fallbacks are recipes rather than single slots, because several roles
/// inherit a foreground *and* a background (and occasionally a modifier) as one
/// unit — `text_inverse on text_bright` has to stay a pair to keep contrast.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SemanticStyle {
    /// `text`
    Text,
    /// `text_bright`
    TextBright,
    /// `text_bright` with bold
    TextBrightBold,
    /// `text_bright` with underline and bold
    TextBrightUnderlinedBold,
    /// `text_highlight`
    TextHighlight,
    /// `text_muted`
    TextMuted,
    /// `text_dim`
    TextDim,
    /// `text` on `surface_raised`
    TextOnSurfaceRaised,
    /// `text_highlight` on `selection_bg`
    HighlightOnSelection,
    /// `selection_fg` on `selection_bg`
    Selection,
    /// `text_inverse` on `text_bright`
    Inverse,
    /// `text_inverse` on `warning`
    InverseOnWarning,
    /// `accent`
    Accent,
    /// `info`
    Info,
    /// `success`
    Success,
    /// `warning`
    Warning,
    /// `error`
    Error,
}

/// The fallback of a `Tint` role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SemanticTint {
    /// Keep the asset's own colours untouched.
    Native,
    Color(SemanticSlot),
}

/// A component role of any kind, used where the four typed enums must be
/// handled uniformly (catalogue metadata, diagnostics, lookup by path).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoleRef {
    Color(ColorRole),
    Style(StyleRole),
    Paint(PaintRole),
    Tint(TintRole),
}

/// The semantic fallback of a role, tagged with the role kind it belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoleFallback {
    Color(SemanticColor),
    Style(SemanticStyle),
    Paint(SemanticPaint),
    Tint(SemanticTint),
}

impl RoleFallback {
    /// Whether this fallback can actually supply a value for `role`.
    ///
    /// Guards the catalogue itself: a `Style` role whose fallback is a bare
    /// colour slot would resolve to a value the renderer cannot use.
    pub fn is_type_compatible(&self, role: RoleRef) -> bool {
        matches!(
            (self, role),
            (RoleFallback::Color(_), RoleRef::Color(_))
                | (RoleFallback::Style(_), RoleRef::Style(_))
                | (RoleFallback::Paint(_), RoleRef::Paint(_))
                | (RoleFallback::Tint(_), RoleRef::Tint(_))
        )
    }
}

/// Catalogue entry of one component role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoleSpec {
    /// Full literal path as written in a theme file, e.g.
    /// `components.dashboard.host_list.border`.
    pub path: &'static str,
    pub role: RoleRef,
    pub fallback: RoleFallback,
    /// Whether the role paints a closed frame. Only closed frames may use the
    /// `perimeter` gradient direction, which has to run a seamless ring.
    pub closed_frame: bool,
}

/// Catalogue entry of one semantic slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SemanticSpec {
    pub key: &'static str,
    pub slot: SemanticSlot,
}

/// The semantic core, in declaration order. `SemanticSlot as usize` indexes it.
pub static SEMANTIC_SPECS: &[SemanticSpec] = &[
    SemanticSpec {
        key: "background",
        slot: SemanticSlot::Background,
    },
    SemanticSpec {
        key: "canvas",
        slot: SemanticSlot::Canvas,
    },
    SemanticSpec {
        key: "surface",
        slot: SemanticSlot::Surface,
    },
    SemanticSpec {
        key: "surface_raised",
        slot: SemanticSlot::SurfaceRaised,
    },
    SemanticSpec {
        key: "border",
        slot: SemanticSlot::Border,
    },
    SemanticSpec {
        key: "border_focus",
        slot: SemanticSlot::BorderFocus,
    },
    SemanticSpec {
        key: "border_popup",
        slot: SemanticSlot::BorderPopup,
    },
    SemanticSpec {
        key: "text",
        slot: SemanticSlot::Text,
    },
    SemanticSpec {
        key: "text_bright",
        slot: SemanticSlot::TextBright,
    },
    SemanticSpec {
        key: "text_highlight",
        slot: SemanticSlot::TextHighlight,
    },
    SemanticSpec {
        key: "text_muted",
        slot: SemanticSlot::TextMuted,
    },
    SemanticSpec {
        key: "text_dim",
        slot: SemanticSlot::TextDim,
    },
    SemanticSpec {
        key: "text_inverse",
        slot: SemanticSlot::TextInverse,
    },
    SemanticSpec {
        key: "accent",
        slot: SemanticSlot::Accent,
    },
    SemanticSpec {
        key: "selection_bg",
        slot: SemanticSlot::SelectionBg,
    },
    SemanticSpec {
        key: "selection_fg",
        slot: SemanticSlot::SelectionFg,
    },
    SemanticSpec {
        key: "success",
        slot: SemanticSlot::Success,
    },
    SemanticSpec {
        key: "warning",
        slot: SemanticSlot::Warning,
    },
    SemanticSpec {
        key: "error",
        slot: SemanticSlot::Error,
    },
    SemanticSpec {
        key: "info",
        slot: SemanticSlot::Info,
    },
    SemanticSpec {
        key: "connecting",
        slot: SemanticSlot::Connecting,
    },
    SemanticSpec {
        key: "exited",
        slot: SemanticSlot::Exited,
    },
    SemanticSpec {
        key: "unknown",
        slot: SemanticSlot::Unknown,
    },
    SemanticSpec {
        key: "pty_background",
        slot: SemanticSlot::PtyBackground,
    },
    SemanticSpec {
        key: "pty_foreground",
        slot: SemanticSlot::PtyForeground,
    },
];

/// Generate the four typed role enums plus the flat [`ROLE_SPECS`] table from
/// one declaration per role: `Variant => ("path", fallback, closed_frame)`.
macro_rules! role_catalog {
    (
        color { $($cvar:ident => ($cpath:literal, $cfb:expr, $cframe:literal)),* $(,)? }
        style { $($svar:ident => ($spath:literal, $sfb:expr, $sframe:literal)),* $(,)? }
        paint { $($pvar:ident => ($ppath:literal, $pfb:expr, $pframe:literal)),* $(,)? }
        tint  { $($tvar:ident => ($tpath:literal, $tfb:expr, $tframe:literal)),* $(,)? }
    ) => {
        /// Roles resolving to a single colour.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(usize)]
        pub enum ColorRole { $($cvar),* }

        /// Roles resolving to a full text style.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(usize)]
        pub enum StyleRole { $($svar),* }

        /// Roles resolving to a colour or a gradient.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(usize)]
        pub enum PaintRole { $($pvar),* }

        /// Roles recolouring embedded assets.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(usize)]
        pub enum TintRole { $($tvar),* }

        impl ColorRole {
            pub const COUNT: usize = [$(stringify!($cvar)),*].len();
        }
        impl StyleRole {
            pub const COUNT: usize = [$(stringify!($svar)),*].len();
        }
        impl PaintRole {
            pub const COUNT: usize = [$(stringify!($pvar)),*].len();
        }
        impl TintRole {
            pub const COUNT: usize = [$(stringify!($tvar)),*].len();
        }

        /// Every component role of the V1 contract, grouped by kind and in
        /// enum-discriminant order within each kind.
        pub static ROLE_SPECS: &[RoleSpec] = &[
            $(RoleSpec {
                path: $cpath,
                role: RoleRef::Color(ColorRole::$cvar),
                fallback: RoleFallback::Color($cfb),
                closed_frame: $cframe,
            },)*
            $(RoleSpec {
                path: $spath,
                role: RoleRef::Style(StyleRole::$svar),
                fallback: RoleFallback::Style($sfb),
                closed_frame: $sframe,
            },)*
            $(RoleSpec {
                path: $ppath,
                role: RoleRef::Paint(PaintRole::$pvar),
                fallback: RoleFallback::Paint($pfb),
                closed_frame: $pframe,
            },)*
            $(RoleSpec {
                path: $tpath,
                role: RoleRef::Tint(TintRole::$tvar),
                fallback: RoleFallback::Tint($tfb),
                closed_frame: $tframe,
            },)*
        ];
    };
}

role_catalog! {
    color {
        StatusSuccess => ("components.status.success", SemanticColor::Success, false),
        StatusWarning => ("components.status.warning", SemanticColor::Warning, false),
        StatusError => ("components.status.error", SemanticColor::Error, false),
        StatusInfo => ("components.status.info", SemanticColor::Info, false),
        StatusUnknown => ("components.status.unknown", SemanticColor::Unknown, false),

        HeaderSessionSuccess => ("components.header.session_success", SemanticColor::Success, false),
        HeaderSessionWarning => ("components.header.session_warning", SemanticColor::Warning, false),
        HeaderSessionError => ("components.header.session_error", SemanticColor::Error, false),
        SessionConnecting => ("components.session.connecting", SemanticColor::Connecting, false),
        SessionExited => ("components.session.exited", SemanticColor::Exited, false),

        DashboardMetricsSparklineLow => ("components.dashboard.metrics.sparkline_low", SemanticColor::Success, false),
        DashboardMetricsSparklineMedium => ("components.dashboard.metrics.sparkline_medium", SemanticColor::Warning, false),
        DashboardMetricsSparklineHigh => ("components.dashboard.metrics.sparkline_high", SemanticColor::Error, false),

        PickerBadgeSuccess => ("components.picker.badge_success", SemanticColor::Success, false),
        PickerBadgeWarning => ("components.picker.badge_warning", SemanticColor::Warning, false),
        PickerBadgeError => ("components.picker.badge_error", SemanticColor::Error, false),

        TunnelRunning => ("components.tunnel.running", SemanticColor::Success, false),
        TunnelStopped => ("components.tunnel.stopped", SemanticColor::Error, false),
        TunnelRetrying => ("components.tunnel.retrying", SemanticColor::Warning, false),
        TunnelConnecting => ("components.tunnel.connecting", SemanticColor::Connecting, false),
        TunnelUnknown => ("components.tunnel.unknown", SemanticColor::Unknown, false),

        IdentitiesCardLoaded => ("components.identities.card.loaded", SemanticColor::Success, false),
        IdentitiesCardMissing => ("components.identities.card.missing", SemanticColor::Unknown, false),
        IdentitiesCardCredential => ("components.identities.card.credential", SemanticColor::Warning, false),

        AuditSuccess => ("components.audit.success", SemanticColor::Success, false),
        AuditWarning => ("components.audit.warning", SemanticColor::Warning, false),
        AuditError => ("components.audit.error", SemanticColor::Error, false),
        AuditUnknown => ("components.audit.unknown", SemanticColor::Unknown, false),

        // `pending` is muted text, not the `unknown` status colour: a host that
        // has not been reached yet is quiet, not in an unknown state.
        BroadcastPending => ("components.broadcast.pending", SemanticColor::TextMuted, false),
        BroadcastRunning => ("components.broadcast.running", SemanticColor::Warning, false),
        BroadcastSuccess => ("components.broadcast.success", SemanticColor::Success, false),
        BroadcastError => ("components.broadcast.error", SemanticColor::Error, false),

    }

    style {
        TextPrimary => ("components.text.primary", SemanticStyle::Text, false),
        TextBright => ("components.text.bright", SemanticStyle::TextBright, false),
        TextMuted => ("components.text.muted", SemanticStyle::TextMuted, false),
        TextDim => ("components.text.dim", SemanticStyle::TextDim, false),
        SelectionActive => ("components.selection.active", SemanticStyle::Selection, false),
        SelectionInactive => ("components.selection.inactive", SemanticStyle::TextOnSurfaceRaised, false),
        FocusIndicator => ("components.focus.indicator", SemanticStyle::Accent, false),

        HeaderBrand => ("components.header.brand", SemanticStyle::Inverse, false),
        HeaderStatsLabel => ("components.header.stats_label", SemanticStyle::TextMuted, false),
        HeaderStatsValue => ("components.header.stats_value", SemanticStyle::Text, false),
        HeaderSessionActive => ("components.header.session_active", SemanticStyle::Inverse, false),
        HeaderSessionInactive => ("components.header.session_inactive", SemanticStyle::TextMuted, false),
        HeaderSessionMore => ("components.header.session_more", SemanticStyle::TextMuted, false),

        SessionTitle => ("components.session.title", SemanticStyle::Inverse, false),
        SessionScrollback => ("components.session.scrollback", SemanticStyle::Warning, false),
        SessionDebugTail => ("components.session.debug_tail", SemanticStyle::TextDim, false),

        DashboardHostListTitle => ("components.dashboard.host_list.title", SemanticStyle::TextBright, false),
        DashboardHostListCount => ("components.dashboard.host_list.count", SemanticStyle::TextDim, false),
        DashboardHostListGroup => ("components.dashboard.host_list.group", SemanticStyle::Info, false),
        DashboardHostListHost => ("components.dashboard.host_list.host", SemanticStyle::Text, false),
        DashboardHostListHostSelected => ("components.dashboard.host_list.host_selected", SemanticStyle::HighlightOnSelection, false),
        DashboardHostListMatch => ("components.dashboard.host_list.match", SemanticStyle::Warning, false),

        DashboardDetailsTitle => ("components.dashboard.details.title", SemanticStyle::TextBright, false),
        DashboardDetailsLabel => ("components.dashboard.details.label", SemanticStyle::Info, false),
        DashboardDetailsValue => ("components.dashboard.details.value", SemanticStyle::Text, false),
        DashboardDetailsMetadata => ("components.dashboard.details.metadata", SemanticStyle::TextMuted, false),
        // The `> ` cursor in front of the detail panel's active edit field.
        // Deliberately *not* `focus.indicator`: that role is the global marker
        // for the two form popups whose marker clung to a directly-ANSI label.
        // This one belongs to the detail panel, so a theme can move the
        // dashboard's own cursor without moving the popups'.
        DashboardDetailsFieldMarker => ("components.dashboard.details.field_marker", SemanticStyle::Accent, false),

        DashboardSshLogTitle => ("components.dashboard.ssh_log.title", SemanticStyle::TextBright, false),
        DashboardAgentTitle => ("components.dashboard.agent.title", SemanticStyle::TextBright, false),
        DashboardLatencyTitle => ("components.dashboard.latency.title", SemanticStyle::TextBright, false),
        DashboardRecentTitle => ("components.dashboard.recent.title", SemanticStyle::TextBright, false),
        DashboardAuthTitle => ("components.dashboard.auth.title", SemanticStyle::TextBright, false),
        DashboardPingTitle => ("components.dashboard.ping.title", SemanticStyle::TextBright, false),

        FooterKey => ("components.footer.key", SemanticStyle::TextBright, false),
        FooterLabel => ("components.footer.label", SemanticStyle::TextMuted, false),
        // The floating chip that stands in for the status-bar notice while a
        // panel is zoomed. Its own role because it is the one notice surface
        // that inverts itself instead of writing into the bar.
        StatusBarToast => ("components.status_bar.toast", SemanticStyle::Info, false),

        PopupTitle => ("components.popup.title", SemanticStyle::TextBright, false),
        PopupHint => ("components.popup.hint", SemanticStyle::TextDim, false),
        PopupLegend => ("components.popup.legend", SemanticStyle::TextMuted, false),
        PopupError => ("components.popup.error", SemanticStyle::Error, false),
        PopupWarning => ("components.popup.warning", SemanticStyle::Warning, false),

        PickerQuery => ("components.picker.query", SemanticStyle::TextBright, false),
        PickerMatch => ("components.picker.match", SemanticStyle::Accent, false),
        PickerRow => ("components.picker.row", SemanticStyle::Text, false),
        PickerRowSelected => ("components.picker.row_selected", SemanticStyle::Selection, false),
        PickerMarker => ("components.picker.marker", SemanticStyle::Selection, false),
        CommandPaletteQuery => ("components.command_palette.query", SemanticStyle::TextHighlight, false),
        CommandPaletteRowSelected => ("components.command_palette.row_selected", SemanticStyle::HighlightOnSelection, false),
        SettingsRowSelected => ("components.settings.row_selected", SemanticStyle::HighlightOnSelection, false),
        SettingsMarker => ("components.settings.marker", SemanticStyle::HighlightOnSelection, false),

        GroupFormLabel => ("components.group_form.label", SemanticStyle::TextMuted, false),
        GroupFormLabelFocused => ("components.group_form.label_focused", SemanticStyle::TextBrightBold, false),
        GroupFormValue => ("components.group_form.value", SemanticStyle::Text, false),
        GroupFormValueFocused => ("components.group_form.value_focused", SemanticStyle::TextBrightBold, false),
        GroupFormMarker => ("components.group_form.marker", SemanticStyle::TextBrightBold, false),

        FormLabel => ("components.form.label", SemanticStyle::TextDim, false),
        FormLabelFocused => ("components.form.label_focused", SemanticStyle::Info, false),
        FormLabelEditing => ("components.form.label_editing", SemanticStyle::Warning, false),
        FormValue => ("components.form.value", SemanticStyle::Text, false),
        FormInput => ("components.form.input", SemanticStyle::TextBright, false),
        FormInputFocused => ("components.form.input_focused", SemanticStyle::TextBright, false),
        FormInputEditing => ("components.form.input_editing", SemanticStyle::TextBrightUnderlinedBold, false),
        FormHelp => ("components.form.help", SemanticStyle::TextDim, false),
        FormError => ("components.form.error", SemanticStyle::Error, false),

        // The tunnel form is its own family rather than a second reader of
        // `form.*` or `group_form.*`. Its focus idiom matches neither: it
        // brightens the label without bolding it, and marks the field being
        // edited with underlined highlight text rather than bold. Folding it
        // onto a neighbour would have silently restyled that neighbour's cells.
        //
        // The title is `accent`, not `popup.title`: the form's title was a bare
        // `Block::title(..)` over an accent frame, and ratatui draws an
        // unstyled title in the border style — so the cell was accent with no
        // weight. It gets its own role rather than staying unstyled, because
        // implicit inheritance is exactly what made the deviation invisible.
        TunnelFormTitle => ("components.tunnel_form.title", SemanticStyle::Accent, false),
        TunnelFormLabel => ("components.tunnel_form.label", SemanticStyle::TextMuted, false),
        TunnelFormLabelFocused => ("components.tunnel_form.label_focused", SemanticStyle::TextBright, false),
        TunnelFormValue => ("components.tunnel_form.value", SemanticStyle::Text, false),
        TunnelFormValueFocused => ("components.tunnel_form.value_focused", SemanticStyle::TextBright, false),
        TunnelFormValueEditing => ("components.tunnel_form.value_editing", SemanticStyle::TextHighlight, false),
        TunnelFormMarker => ("components.tunnel_form.marker", SemanticStyle::Success, false),
        TunnelFormHelp => ("components.tunnel_form.help", SemanticStyle::TextDim, false),

        TableRow => ("components.table.row", SemanticStyle::Text, false),
        TableRowSelected => ("components.table.row_selected", SemanticStyle::Selection, false),

        HelpSection => ("components.help.section", SemanticStyle::TextBright, false),
        HelpKey => ("components.help.key", SemanticStyle::TextBright, false),
        HelpDescription => ("components.help.description", SemanticStyle::Text, false),

        KeybindRow => ("components.keybind.row", SemanticStyle::Text, false),
        KeybindRowSelected => ("components.keybind.row_selected", SemanticStyle::HighlightOnSelection, false),
        KeybindMarker => ("components.keybind.marker", SemanticStyle::HighlightOnSelection, false),
        KeybindValue => ("components.keybind.value", SemanticStyle::TextMuted, false),
        KeybindValueBound => ("components.keybind.value_bound", SemanticStyle::Success, false),
        KeybindValueCapturing => ("components.keybind.value_capturing", SemanticStyle::Warning, false),


        TabsActive => ("components.tabs.active", SemanticStyle::Inverse, false),
        TabsInactive => ("components.tabs.inactive", SemanticStyle::TextMuted, false),

        TunnelsSummary => ("components.tunnels.summary", SemanticStyle::TextMuted, false),
        TunnelsTableHeader => ("components.tunnels.table_header", SemanticStyle::TextBrightBold, false),
        TunnelsRow => ("components.tunnels.row", SemanticStyle::Text, false),
        TunnelsRowSelected => ("components.tunnels.row_selected", SemanticStyle::Selection, false),
        TunnelsDirection => ("components.tunnels.direction", SemanticStyle::Info, false),
        TunnelsRemote => ("components.tunnels.remote", SemanticStyle::TextMuted, false),
        TunnelsMetadata => ("components.tunnels.metadata", SemanticStyle::TextDim, false),
        TunnelsNotice => ("components.tunnels.notice", SemanticStyle::Warning, false),
        TunnelsError => ("components.tunnels.error", SemanticStyle::Error, false),
        TunnelsEmpty => ("components.tunnels.empty", SemanticStyle::TextDim, false),

        SftpLocal => ("components.sftp.local", SemanticStyle::Info, false),
        SftpRemote => ("components.sftp.remote", SemanticStyle::Info, false),
        SftpSelection => ("components.sftp.selection", SemanticStyle::Selection, false),
        SftpSearch => ("components.sftp.search", SemanticStyle::InverseOnWarning, false),
        SftpQueueDownload => ("components.sftp.queue_download", SemanticStyle::Success, false),
        SftpQueueUpload => ("components.sftp.queue_upload", SemanticStyle::Warning, false),
        SftpProgress => ("components.sftp.progress", SemanticStyle::Warning, false),
        SftpProgressComplete => ("components.sftp.progress_complete", SemanticStyle::Success, false),
        SftpProgressRemaining => ("components.sftp.progress_remaining", SemanticStyle::TextDim, false),
        SftpNotice => ("components.sftp.notice", SemanticStyle::Warning, false),
        SftpQueueHeader => ("components.sftp.queue_header", SemanticStyle::TextBrightBold, false),
        SftpPanelTitle => ("components.sftp.panel.title", SemanticStyle::TextBright, false),
        SftpPanelCount => ("components.sftp.panel.count", SemanticStyle::TextDim, false),

        IdentitiesEmpty => ("components.identities.empty", SemanticStyle::TextDim, false),
        IdentitiesCardSelection => ("components.identities.card.selection", SemanticStyle::Selection, false),
        IdentitiesCardName => ("components.identities.card.name", SemanticStyle::TextBrightBold, false),
        IdentitiesCardText => ("components.identities.card.text", SemanticStyle::Text, false),
        IdentitiesCardMetadata => ("components.identities.card.metadata", SemanticStyle::TextDim, false),
        IdentitiesCardKeyType => ("components.identities.card.key_type", SemanticStyle::TextMuted, false),
        IdentitiesAgentLabel => ("components.identities.agent.label", SemanticStyle::TextMuted, false),
        IdentitiesAgentValue => ("components.identities.agent.value", SemanticStyle::Text, false),
        IdentitiesAgentCount => ("components.identities.agent.count", SemanticStyle::TextBright, false),
        IdentitiesNotice => ("components.identities.notice", SemanticStyle::Warning, false),

        // The key generator popup. `label_focused` and `value_focused` are one
        // pair rather than the tunnel form's three, because the keygen form has
        // no separate "selected but not editing" state: reaching a field is
        // editing it. The `\u{25b8}` marker shares `label_focused` for the same
        // reason — it marks exactly the row that label belongs to.
        KeygenTitle => ("components.keygen.title", SemanticStyle::TextBrightBold, false),
        KeygenLabel => ("components.keygen.label", SemanticStyle::TextDim, false),
        KeygenLabelFocused => ("components.keygen.label_focused", SemanticStyle::Warning, false),
        KeygenValue => ("components.keygen.value", SemanticStyle::Text, false),
        KeygenValueFocused => ("components.keygen.value_focused", SemanticStyle::TextBrightUnderlinedBold, false),
        KeygenHelp => ("components.keygen.help", SemanticStyle::TextDim, false),

        AuditFilterActive => ("components.audit.filter_active", SemanticStyle::Inverse, false),
        AuditFilterInactive => ("components.audit.filter_inactive", SemanticStyle::TextDim, false),
        AuditNote => ("components.audit.note", SemanticStyle::TextMuted, false),
        // Bold like its sibling `tunnels.table_header`: both column headers have
        // always been drawn with the same `theme::heading()` call.
        AuditTableHeader => ("components.audit.table_header", SemanticStyle::TextBrightBold, false),
        AuditRow => ("components.audit.row", SemanticStyle::Text, false),
        AuditRowSelected => ("components.audit.row_selected", SemanticStyle::Selection, false),

        BroadcastStdout => ("components.broadcast.stdout", SemanticStyle::TextMuted, false),
        BroadcastStderr => ("components.broadcast.stderr", SemanticStyle::Error, false),
        BroadcastDetail => ("components.broadcast.detail", SemanticStyle::TextDim, false),
        BroadcastCountdown => ("components.broadcast.countdown", SemanticStyle::Info, false),
        BroadcastPanelTitle => ("components.broadcast.panel.title", SemanticStyle::TextBright, false),
        BroadcastPanelCount => ("components.broadcast.panel.count", SemanticStyle::TextDim, false),

        AnimationNode => ("components.animation.node", SemanticStyle::Success, false),
        AnimationNodeLabel => ("components.animation.node_label", SemanticStyle::Text, false),
        AnimationSpoke => ("components.animation.spoke", SemanticStyle::TextDim, false),
        AnimationHubEarly => ("components.animation.hub_early", SemanticStyle::Success, false),
        // Bright *and* bold, like `wordmark`: the assembled hub glyph and the
        // wordmark were the same `theme::heading()`-weight cell, and the plain
        // `text_bright` fallback would have quietly dropped the weight.
        AnimationHubReady => ("components.animation.hub_ready", SemanticStyle::TextBrightBold, false),
        AnimationHubFlash => ("components.animation.hub_flash", SemanticStyle::Warning, false),
        // The word "hub" while the hub is still assembling. `hub_flash` is the
        // same word once the animation is done and it starts pulsing; the two
        // were never the same colour, so they cannot be the same role.
        AnimationHubLabel => ("components.animation.hub_label", SemanticStyle::TextMuted, false),
        AnimationWordmark => ("components.animation.wordmark", SemanticStyle::TextBrightBold, false),
        AnimationWordmarkAccent => ("components.animation.wordmark_accent", SemanticStyle::Warning, false),
        AnimationTagline => ("components.animation.tagline", SemanticStyle::TextMuted, false),
        AnimationTaglineAccent => ("components.animation.tagline_accent", SemanticStyle::Warning, false),
        AnimationQuip => ("components.animation.quip", SemanticStyle::TextDim, false),
        AnimationPromptKey => ("components.animation.prompt_key", SemanticStyle::TextBright, false),
        AnimationPromptText => ("components.animation.prompt_text", SemanticStyle::TextMuted, false),
        AnimationCursor => ("components.animation.cursor", SemanticStyle::Success, false),
    }

    paint {
        AppBackground => ("components.app.background", SemanticPaint::Background, false),
        SeparatorPrimary => ("components.separator.primary", SemanticPaint::Border, false),
        SeparatorSecondary => ("components.separator.secondary", SemanticPaint::TextDim, false),

        HeaderBackground => ("components.header.background", SemanticPaint::SurfaceRaised, false),
        HeaderSeparator => ("components.header.separator", SemanticPaint::TextDim, false),
        SessionBackground => ("components.session.background", SemanticPaint::Background, false),
        SessionBorder => ("components.session.border", SemanticPaint::BorderPopup, true),

        DashboardHostListBorder => ("components.dashboard.host_list.border", SemanticPaint::Border, true),
        DashboardHostListBorderFocused => ("components.dashboard.host_list.border_focused", SemanticPaint::BorderFocus, true),
        DashboardHostListBackground => ("components.dashboard.host_list.background", SemanticPaint::Surface, false),
        DashboardDetailsBorder => ("components.dashboard.details.border", SemanticPaint::Border, true),
        DashboardDetailsBorderFocused => ("components.dashboard.details.border_focused", SemanticPaint::BorderFocus, true),
        DashboardDetailsBackground => ("components.dashboard.details.background", SemanticPaint::Surface, false),

        DashboardSshLogBorder => ("components.dashboard.ssh_log.border", SemanticPaint::Border, true),
        DashboardSshLogBorderFocused => ("components.dashboard.ssh_log.border_focused", SemanticPaint::BorderFocus, true),
        DashboardSshLogBackground => ("components.dashboard.ssh_log.background", SemanticPaint::Surface, false),
        DashboardAgentBorder => ("components.dashboard.agent.border", SemanticPaint::Border, true),
        DashboardAgentBorderFocused => ("components.dashboard.agent.border_focused", SemanticPaint::BorderFocus, true),
        DashboardAgentBackground => ("components.dashboard.agent.background", SemanticPaint::Surface, false),
        DashboardLatencyBorder => ("components.dashboard.latency.border", SemanticPaint::Border, true),
        DashboardLatencyBorderFocused => ("components.dashboard.latency.border_focused", SemanticPaint::BorderFocus, true),
        DashboardLatencyBackground => ("components.dashboard.latency.background", SemanticPaint::Surface, false),
        DashboardRecentBorder => ("components.dashboard.recent.border", SemanticPaint::Border, true),
        DashboardRecentBorderFocused => ("components.dashboard.recent.border_focused", SemanticPaint::BorderFocus, true),
        DashboardRecentBackground => ("components.dashboard.recent.background", SemanticPaint::Surface, false),
        DashboardAuthBorder => ("components.dashboard.auth.border", SemanticPaint::Border, true),
        DashboardAuthBorderFocused => ("components.dashboard.auth.border_focused", SemanticPaint::BorderFocus, true),
        DashboardAuthBackground => ("components.dashboard.auth.background", SemanticPaint::Surface, false),
        DashboardPingBorder => ("components.dashboard.ping.border", SemanticPaint::Border, true),
        DashboardPingBorderFocused => ("components.dashboard.ping.border_focused", SemanticPaint::BorderFocus, true),
        DashboardPingBackground => ("components.dashboard.ping.background", SemanticPaint::Surface, false),

        FooterBackground => ("components.footer.background", SemanticPaint::SurfaceRaised, false),
        FooterSeparator => ("components.footer.separator", SemanticPaint::TextDim, false),

        PopupBackground => ("components.popup.background", SemanticPaint::Surface, false),
        PopupBorder => ("components.popup.border", SemanticPaint::BorderPopup, true),
        PickerBorder => ("components.picker.border", SemanticPaint::Accent, true),

        TabsSeparator => ("components.tabs.separator", SemanticPaint::TextDim, false),
        TunnelsSeparator => ("components.tunnels.separator", SemanticPaint::TextDim, false),
        TunnelFormBorder => ("components.tunnel_form.border", SemanticPaint::Accent, true),

        SftpPanelBorder => ("components.sftp.panel.border", SemanticPaint::Border, true),
        SftpPanelBorderFocused => ("components.sftp.panel.border_focused", SemanticPaint::BorderFocus, true),
        SftpPanelBackground => ("components.sftp.panel.background", SemanticPaint::Surface, false),

        IdentitiesCardBorder => ("components.identities.card.border", SemanticPaint::Border, true),
        IdentitiesCardBorderSelected => ("components.identities.card.border_selected", SemanticPaint::Accent, true),
        IdentitiesAgentSeparator => ("components.identities.agent.separator", SemanticPaint::TextDim, false),

        BroadcastPanelBorder => ("components.broadcast.panel.border", SemanticPaint::Border, true),
        BroadcastPanelBorderFocused => ("components.broadcast.panel.border_focused", SemanticPaint::BorderFocus, true),
        BroadcastPanelBackground => ("components.broadcast.panel.background", SemanticPaint::Surface, false),

        AnimationBackground => ("components.animation.background", SemanticPaint::Background, false),
        AnimationHalo => ("components.animation.halo", SemanticPaint::SelectionBg, false),
    }

    tint {
        OsLogoTint => ("components.os_logo.tint", SemanticTint::Native, false),
    }
}

// ---------------------------------------------------------------------------
// Documentation renderer
// ---------------------------------------------------------------------------

/// The public role catalogue as Markdown, for `docs/theme-system.md`.
///
/// Generated, never written by hand: the guide's table sits between the stable
/// `THEME_ROLES:START` / `THEME_ROLES:END` markers and a test compares the
/// checked-in block with this function byte for byte. A role added to
/// [`ROLE_SPECS`] therefore fails the build until the guide is regenerated,
/// which is the only way a 234-row table stays true.
///
/// Grouped by *surface* rather than by [`ROLE_SPECS`] order, because a reader
/// arrives knowing which part of the screen they want to recolour, not which
/// value type it happens to have.
pub fn role_catalog_markdown() -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write;

    let mut families: BTreeMap<&'static str, Vec<&RoleSpec>> = BTreeMap::new();
    for spec in ROLE_SPECS {
        families
            .entry(role_family(spec.path))
            .or_default()
            .push(spec);
    }

    let mut out = String::new();
    for (family, mut specs) in families {
        specs.sort_by_key(|spec| spec.path);
        let _ = writeln!(out, "#### `components.{family}`\n");
        let _ = writeln!(out, "| Role | Type | Falls back to | Closed frame |");
        let _ = writeln!(out, "| --- | --- | --- | --- |");
        for spec in specs {
            let _ = writeln!(
                out,
                "| `{}` | {} | {} | {} |",
                spec.path,
                role_kind(spec.role),
                fallback_markdown(spec.fallback),
                if spec.closed_frame { "yes" } else { "no" },
            );
        }
        out.push('\n');
    }
    out
}

/// The surface a role path belongs to: the segment after `components.`.
fn role_family(path: &'static str) -> &'static str {
    path.strip_prefix("components.")
        .and_then(|rest| rest.split('.').next())
        // Unreachable while the catalogue guard holds; documenting a role under
        // its own path is still better than panicking in a doc renderer.
        .unwrap_or(path)
}

fn role_kind(role: RoleRef) -> &'static str {
    match role {
        RoleRef::Color(_) => "color",
        RoleRef::Style(_) => "style",
        RoleRef::Paint(_) => "paint",
        RoleRef::Tint(_) => "tint",
    }
}

/// What a role inherits when a theme does not set it, in prose.
///
/// The style recipes are **read out of the resolver** rather than transcribed:
/// [`crate::theme::model::semantic_style`] is run against a probe core whose
/// slots all carry distinct colours, and the resulting `Style` is translated
/// back into the slot names it used. A recipe that changes therefore changes
/// this table, instead of leaving the documentation quietly wrong.
fn fallback_markdown(fallback: RoleFallback) -> String {
    match fallback {
        RoleFallback::Color(slot) | RoleFallback::Paint(slot) => {
            format!("`semantic.{}`", slot.key())
        }
        RoleFallback::Tint(SemanticTint::Native) => {
            "`native` (the asset's own colours)".to_string()
        }
        RoleFallback::Tint(SemanticTint::Color(slot)) => format!("`semantic.{}`", slot.key()),
        RoleFallback::Style(recipe) => style_recipe_markdown(recipe),
    }
}

/// A probe semantic core: every slot a colour that identifies only that slot.
fn probe_semantic() -> crate::theme::model::ResolvedSemantic {
    let mut slots = [ratatui::style::Color::Reset; crate::theme::model::SEMANTIC_SLOT_COUNT];
    for (index, slot) in slots.iter_mut().enumerate() {
        *slot = ratatui::style::Color::Rgb(0, 0, index as u8);
    }
    crate::theme::model::ResolvedSemantic::from_slots(slots)
}

/// The slot a probe colour came from, if any.
fn probe_slot(color: ratatui::style::Color) -> Option<SemanticSlot> {
    match color {
        ratatui::style::Color::Rgb(0, 0, index) => {
            SEMANTIC_SPECS.get(index as usize).map(|spec| spec.slot)
        }
        _ => None,
    }
}

fn style_recipe_markdown(recipe: SemanticStyle) -> String {
    use std::fmt::Write;

    let style = crate::theme::model::semantic_style(&probe_semantic(), recipe);
    let name = |color: Option<ratatui::style::Color>| {
        color
            .and_then(probe_slot)
            .map(|slot| format!("`semantic.{}`", slot.key()))
    };
    let mut out = name(style.fg).unwrap_or_else(|| "`terminal`".to_string());
    if let Some(background) = name(style.bg) {
        let _ = write!(out, " on {background}");
    }
    let modifiers: Vec<&str> = crate::theme::model::MODIFIER_KEYS
        .iter()
        .filter(|key| {
            crate::theme::model::modifier_from_key(key)
                .is_some_and(|modifier| style.add_modifier.contains(modifier))
        })
        .copied()
        .collect();
    if !modifiers.is_empty() {
        let _ = write!(out, " + {}", modifiers.join(" + "));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guide, as it is checked in.
    const THEME_GUIDE: &str = include_str!("../../docs/theme-system.md");
    const ROLES_START: &str = "<!-- THEME_ROLES:START -->";
    const ROLES_END: &str = "<!-- THEME_ROLES:END -->";

    /// The block between the two markers, without the marker lines themselves.
    fn checked_in_role_block() -> &'static str {
        let start = THEME_GUIDE
            .find(ROLES_START)
            .expect("docs/theme-system.md must carry the THEME_ROLES:START marker")
            + ROLES_START.len();
        let end = THEME_GUIDE
            .find(ROLES_END)
            .expect("docs/theme-system.md must carry the THEME_ROLES:END marker");
        // One newline belongs to each marker line; the table itself is what
        // lies between them.
        THEME_GUIDE[start..end]
            .strip_prefix('\n')
            .expect("the start marker must sit on a line of its own")
    }

    /// The single-source contract: the guide's role table *is*
    /// [`role_catalog_markdown`], byte for byte.
    #[test]
    fn the_guide_role_table_matches_the_generated_catalogue() {
        let generated = role_catalog_markdown();
        let checked_in = checked_in_role_block();
        assert_eq!(
            checked_in, generated,
            "docs/theme-system.md is out of date with ROLE_SPECS. Regenerate the \
             block between the THEME_ROLES markers with:\n  cargo test --lib \
             theme::catalog::tests::print_role_catalog_markdown -- --ignored --nocapture"
        );
    }

    /// Every published role has to appear in the guide, under its own path.
    ///
    /// The byte-for-byte test above already implies this, but only while the
    /// generator is right; this one reads the checked-in file directly, so a
    /// generator that started dropping rows fails here too.
    #[test]
    fn the_guide_lists_every_published_role() {
        let checked_in = checked_in_role_block();
        for spec in ROLE_SPECS {
            assert!(
                checked_in.contains(&format!("| `{}` |", spec.path)),
                "{} is missing from the guide's role catalogue",
                spec.path
            );
        }
    }

    /// Every complete theme the guide prints, as `(id, toml)`.
    ///
    /// Extracted from the checked-in Markdown rather than duplicated here: a
    /// copy would be the one thing this test exists to rule out.
    fn guide_examples() -> Vec<(String, String)> {
        const START: &str = "<!-- THEME_EXAMPLE:START id=";
        const END: &str = "<!-- THEME_EXAMPLE:END -->";
        let mut out = Vec::new();
        let mut rest = THEME_GUIDE;
        while let Some(at) = rest.find(START) {
            rest = &rest[at + START.len()..];
            let (id, after) = rest
                .split_once(" -->\n")
                .expect("a well-formed start marker");
            let (block, after) = after.split_once(END).expect("an unterminated example");
            let toml = block
                .trim()
                .strip_prefix("```toml")
                .and_then(|body| body.trim_end().strip_suffix("```"))
                .unwrap_or_else(|| panic!("example `{id}` is not a ```toml fence"))
                .trim_start_matches('\n');
            out.push((id.to_string(), toml.to_string()));
            rest = after;
        }
        out
    }

    /// The guide's complete example themes have to *work*.
    ///
    /// Each one goes through the real strict pipeline — the same
    /// [`ThemeRegistry::load_check_target`] that `sshub theme check` runs — and
    /// has to validate without a single diagnostic and resolve to a runtime
    /// theme. A documentation example that does not work is worse than none.
    #[test]
    fn every_guide_example_passes_the_real_strict_pipeline() {
        let examples = guide_examples();
        let ids: Vec<&str> = examples.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["mint", "harbour"],
            "the guide's worked examples changed; keep this list in step so a \
             silently dropped example cannot pass as a green run"
        );

        for (id, toml) in &examples {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(format!("{id}.toml"));
            std::fs::write(&path, toml).unwrap();

            let registry = crate::theme::registry::ThemeRegistry::load_check_target(
                &path,
                crate::theme::model::ValidationMode::Strict,
            )
            .unwrap_or_else(|e| panic!("example `{id}` could not be read: {e}"));
            let record = registry
                .get(id)
                .unwrap_or_else(|| panic!("example `{id}` is not registered"));
            assert!(
                record.diagnostics.is_empty(),
                "example `{id}` is not clean: {:#?}",
                record.diagnostics
            );
            assert!(record.is_valid(), "example `{id}` does not validate");
            assert!(
                record.resolved().is_some(),
                "example `{id}` validates but does not resolve"
            );
        }
    }

    /// The substantial example has to demonstrate what its prose claims: an
    /// inherited parent, a multi-stop ring on a closed frame, and an `"auto"`
    /// reset that really lands back on the semantic fallback.
    #[test]
    fn the_substantial_guide_example_demonstrates_what_it_promises() {
        use crate::theme::model::{ResolvedPaint, ValidationMode};

        let (id, toml) = guide_examples()
            .into_iter()
            .find(|(id, _)| id == "harbour")
            .expect("the substantial example");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("{id}.toml"));
        std::fs::write(&path, &toml).unwrap();
        let registry =
            crate::theme::registry::ThemeRegistry::load_check_target(&path, ValidationMode::Strict)
                .unwrap();
        let record = registry.get(&id).unwrap();
        let theme = record.resolved().expect("it resolves").clone();

        assert_eq!(
            record.inheritance_chain().last().map(|id| id.as_str()),
            Some("default"),
            "the chain must run through the built-in root"
        );
        assert!(
            record
                .inheritance_chain()
                .iter()
                .any(|id| id.as_str() == "aqua"),
            "the example claims to extend `aqua`"
        );

        // The perimeter ring: a gradient, on a closed frame, with four stops
        // whose ends meet.
        let ring = match theme.paint(PaintRole::DashboardHostListBorderFocused) {
            ResolvedPaint::Gradient(id) => theme.gradient(*id).expect("the ring resolves"),
            other => panic!("the focused frame is not a gradient: {other:?}"),
        };
        assert!(
            ROLE_SPECS.iter().any(|spec| spec.role
                == RoleRef::Paint(PaintRole::DashboardHostListBorderFocused)
                && spec.closed_frame),
            "the role the example rings must be a closed frame"
        );
        assert_eq!(ring.stops().len(), 4);
        assert_eq!(
            ring.stops().first().unwrap().color(),
            ring.stops().last().unwrap().color(),
            "a perimeter ring has to close on itself"
        );

        // The `"auto"` reset really goes back to the semantic fallback — and
        // the parent really did override that role, so the reset is visible
        // rather than a no-op dressed up as documentation.
        let aqua = crate::theme::registry::ThemeRegistry::builtins(ValidationMode::Strict)
            .unwrap()
            .resolved(&crate::theme::model::ThemeId::parse("aqua").unwrap())
            .unwrap();
        assert!(
            matches!(
                aqua.paint(PaintRole::PopupBorder),
                ResolvedPaint::Gradient(_)
            ),
            "`aqua` no longer rings the popup frame, so the example's `auto` \
             reset would demonstrate nothing"
        );
        assert_eq!(
            *theme.paint(PaintRole::PopupBorder),
            ResolvedPaint::Solid(theme.semantic().border_popup),
            "`border = \"auto\"` must land back on semantic.border_popup"
        );
    }

    /// Regeneration helper: prints exactly what belongs between the markers.
    #[test]
    #[ignore = "prints the generated role table for docs/theme-system.md"]
    fn print_role_catalog_markdown() {
        print!("{}", role_catalog_markdown());
    }

    /// The style recipes in the table are read out of the resolver, so this
    /// pins the translation rather than the recipes: a fallback that is a pair
    /// must name both slots, and a modifier must survive into the prose.
    #[test]
    fn the_generated_fallback_prose_names_slots_and_modifiers() {
        assert_eq!(
            fallback_markdown(RoleFallback::Style(SemanticStyle::Selection)),
            "`semantic.selection_fg` on `semantic.selection_bg`"
        );
        assert_eq!(
            fallback_markdown(RoleFallback::Style(SemanticStyle::TextBrightUnderlinedBold)),
            "`semantic.text_bright` + bold + underlined"
        );
        assert_eq!(
            fallback_markdown(RoleFallback::Color(SemanticSlot::Accent)),
            "`semantic.accent`"
        );
        assert_eq!(
            fallback_markdown(RoleFallback::Tint(SemanticTint::Native)),
            "`native` (the asset's own colours)"
        );
    }

    #[test]
    fn v1_catalog_is_complete_unique_and_typed() {
        assert_eq!(SEMANTIC_SPECS.len(), 25);
        let paths: std::collections::BTreeSet<_> =
            ROLE_SPECS.iter().map(|spec| spec.path).collect();
        assert_eq!(paths.len(), ROLE_SPECS.len());
        assert!(ROLE_SPECS
            .iter()
            .all(|spec| spec.path.starts_with("components.")));
        assert!(ROLE_SPECS
            .iter()
            .all(|spec| spec.fallback.is_type_compatible(spec.role)));
    }

    /// The frozen role → semantic-fallback matrix, one line per role.
    ///
    /// The default-parity test compares each role against the fallback its own
    /// catalogue row names, which proves resolution honours the catalogue but
    /// cannot notice a row whose *mapping* was changed — flipping
    /// `TunnelRunning` from `Success` to `Error` would still pass. This snapshot
    /// is that guard: any edit to the frozen V1 matrix shows up as an explicit
    /// diff to be reviewed against the spec's role tables.
    ///
    /// Regenerate it deliberately when the contract really changes, never to
    /// silence a failing test.
    const FROZEN_ROLE_MATRIX: &str = include_str!("role_matrix.snapshot");

    #[test]
    fn the_role_to_fallback_matrix_matches_its_frozen_snapshot() {
        let actual: Vec<String> = ROLE_SPECS
            .iter()
            .map(|spec| format!("{} = {:?}", spec.path, spec.fallback))
            .collect();
        let frozen: Vec<&str> = FROZEN_ROLE_MATRIX.lines().collect();

        let changed = matrix_diff(&frozen, &actual);
        assert!(
            changed.is_empty(),
            "the frozen V1 role \u{2192} fallback matrix changed ({} rows frozen, {} now).\n{}\n\
             Check every difference against the spec's role catalogue before \
             updating src/theme/role_matrix.snapshot.",
            frozen.len(),
            actual.len(),
            changed.join("\n")
        );
    }

    /// The frozen-vs-actual comparison, as a pure function so it can be tested
    /// on inputs the real snapshot cannot produce.
    ///
    /// Diffing is by role path, not by line position: a pairwise zip turns a
    /// single *removal* into a cascade of dozens of bogus "changed" rows, which
    /// buries exactly the review this snapshot exists to enable. But a map keyed
    /// by path also *collapses duplicates*, so the row counts are checked
    /// explicitly — otherwise a snapshot with a repeated line would compare
    /// equal to a catalogue that is genuinely one row shorter.
    fn matrix_diff(frozen: &[&str], actual: &[String]) -> Vec<String> {
        let key = |row: &str| row.split(" = ").next().unwrap_or(row).to_string();
        let was: std::collections::BTreeMap<String, &str> =
            frozen.iter().map(|row| (key(row), *row)).collect();
        let now: std::collections::BTreeMap<String, &str> =
            actual.iter().map(|row| (key(row), row.as_str())).collect();

        let mut changed: Vec<String> = Vec::new();

        // Duplicate paths would silently vanish into the maps below, taking the
        // length protection with them.
        if was.len() != frozen.len() {
            changed.push(format!(
                "! the frozen snapshot has {} rows but only {} distinct role paths \u{2014} \
                 it contains duplicates",
                frozen.len(),
                was.len()
            ));
        }
        if now.len() != actual.len() {
            changed.push(format!(
                "! the catalogue has {} rows but only {} distinct role paths \u{2014} \
                 it contains duplicates",
                actual.len(),
                now.len()
            ));
        }
        if frozen.len() != actual.len() {
            changed.push(format!(
                "! row count moved: {} frozen, {} now",
                frozen.len(),
                actual.len()
            ));
        }

        for (path, row) in &was {
            match now.get(path) {
                None => changed.push(format!("- removed:  {row}")),
                Some(current) if current != row => {
                    changed.push(format!("~ remapped: {row}  ->  {current}"))
                }
                Some(_) => {}
            }
        }
        for (path, row) in &now {
            if !was.contains_key(path) {
                changed.push(format!("+ added:    {row}"));
            }
        }
        changed
    }

    /// Every diagnostic class `matrix_diff` can emit, with the **complete**
    /// report pinned line for line.
    ///
    /// Substring matching would let an unwanted extra classification, or a
    /// half-mangled message, pass green — so each case states the whole report.
    /// Note that a length change can never appear alone: with distinct paths on
    /// both sides it always implies a removal or an addition, so the only inputs
    /// that isolate the row-count line are the two duplicate cases.
    #[test]
    fn every_diagnostic_class_is_reported() {
        struct Case {
            name: &'static str,
            frozen: &'static [&'static str],
            actual: &'static [&'static str],
            report: &'static [&'static str],
        }

        const A: &str = "components.a = Style(Text)";
        const B: &str = "components.b = Style(Text)";

        const CASES: &[Case] = &[
            Case {
                name: "duplicate in the frozen snapshot",
                frozen: &[A, A, B],
                actual: &[A, B],
                report: &[
                    "! the frozen snapshot has 3 rows but only 2 distinct role paths \u{2014} it contains duplicates",
                    "! row count moved: 3 frozen, 2 now",
                ],
            },
            Case {
                name: "duplicate in the catalogue",
                frozen: &[A, B],
                actual: &[A, A, B],
                report: &[
                    "! the catalogue has 3 rows but only 2 distinct role paths \u{2014} it contains duplicates",
                    "! row count moved: 2 frozen, 3 now",
                ],
            },
            Case {
                name: "removal, with the row count it implies",
                frozen: &[A, B],
                actual: &[A],
                report: &[
                    "! row count moved: 2 frozen, 1 now",
                    "- removed:  components.b = Style(Text)",
                ],
            },
            Case {
                name: "addition, with the row count it implies",
                frozen: &[A],
                actual: &[A, B],
                report: &[
                    "! row count moved: 1 frozen, 2 now",
                    "+ added:    components.b = Style(Text)",
                ],
            },
            Case {
                name: "remap, which moves no row count",
                frozen: &[A],
                actual: &["components.a = Style(Error)"],
                report: &[
                    "~ remapped: components.a = Style(Text)  ->  components.a = Style(Error)",
                ],
            },
        ];

        for case in CASES {
            let actual: Vec<String> = case.actual.iter().map(|row| row.to_string()).collect();
            assert_eq!(
                matrix_diff(case.frozen, &actual),
                case.report,
                "`{}` reported something other than its expected lines",
                case.name
            );
        }
    }

    /// The regression by name: keying by role path collapses duplicates, so
    /// without a length check a snapshot listing one row twice compared equal to
    /// a catalogue genuinely one row shorter.
    #[test]
    fn a_duplicated_frozen_row_cannot_hide_a_removal() {
        // Same two paths either side, but the snapshot repeats one of them in
        // place of a role the catalogue no longer has.
        let frozen = vec![
            "components.a = Style(Text)",
            "components.a = Style(Text)",
            "components.b = Style(Text)",
        ];
        let actual = vec![
            "components.a = Style(Text)".to_string(),
            "components.b = Style(Text)".to_string(),
        ];

        let changed = matrix_diff(&frozen, &actual);
        assert!(
            !changed.is_empty(),
            "a duplicated frozen row hid a length change: {changed:?}"
        );
        assert!(
            changed.iter().any(|row| row.contains("duplicates")),
            "the report should name the duplication: {changed:?}"
        );
    }

    /// The happy path still reports nothing, so the guard above cannot be
    /// satisfied by simply always failing.
    #[test]
    fn an_unchanged_matrix_reports_no_differences() {
        let frozen = vec!["components.a = Style(Text)", "components.b = Paint(Border)"];
        let actual = vec![
            "components.a = Style(Text)".to_string(),
            "components.b = Paint(Border)".to_string(),
        ];
        assert!(matrix_diff(&frozen, &actual).is_empty());
    }

    /// The two counts the guide states in prose are the ones `ROLE_SPECS`
    /// actually has.
    ///
    /// The generated table between the `THEME_ROLES` markers is checked byte
    /// for byte elsewhere, but the numbers in the surrounding sentences are
    /// hand-written and sit far away from it — they went stale silently when
    /// roles were added. Deriving both from `ROLE_SPECS` and searching the
    /// guide for them ties the prose to the catalogue.
    #[test]
    fn the_guide_states_the_role_counts_the_catalogue_has() {
        let total = ROLE_SPECS.len();
        // Every role but one falls back to a semantic slot; `os_logo.tint`
        // defaults to the asset's own colours instead.
        let native: Vec<&str> = ROLE_SPECS
            .iter()
            .filter(|spec| matches!(spec.fallback, RoleFallback::Tint(SemanticTint::Native)))
            .map(|spec| spec.path)
            .collect();
        assert_eq!(native, ["components.os_logo.tint"]);
        let semantic = total - native.len();
        assert_eq!((total, semantic), (234, 233));

        for phrase in [
            format!("The {total} component roles"),
            format!("{semantic} of them fall back to a semantic slot"),
            format!("{semantic} of SSHub's {total} component roles fall back"),
            format!("{total} roles, grouped by the surface they paint"),
        ] {
            assert!(
                THEME_GUIDE.contains(&phrase),
                "docs/theme-system.md no longer states `{phrase}`"
            );
        }
    }

    #[test]
    fn role_counts_cover_every_spec() {
        assert_eq!(
            ColorRole::COUNT + StyleRole::COUNT + PaintRole::COUNT + TintRole::COUNT,
            ROLE_SPECS.len()
        );
    }

    #[test]
    fn semantic_specs_are_indexed_by_their_slot() {
        for (index, spec) in SEMANTIC_SPECS.iter().enumerate() {
            assert_eq!(spec.slot as usize, index, "{}", spec.key);
        }
    }

    #[test]
    fn only_closed_frames_may_use_perimeter_gradients() {
        // `perimeter` has to run a seamless ring, so a role that never draws a
        // closed frame must not be flagged for it — and only paints draw frames.
        assert!(ROLE_SPECS
            .iter()
            .all(|spec| !spec.closed_frame || matches!(spec.role, RoleRef::Paint(_))));
    }
}
