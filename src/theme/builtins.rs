//! The five reference themes compiled into the binary.
//!
//! The assets are real `.toml` files under `assets/themes/`, embedded with
//! [`include_str!`] rather than hand-written Rust structures. That is what
//! guarantees built-ins and user themes go through exactly the same parser,
//! validator and resolver — a built-in cannot express anything a user file
//! could not, and `sshub theme show <id>` can print a copyable original.

/// One embedded reference theme.
pub struct BuiltInTheme {
    /// Reserved theme id; also the asset's file stem.
    pub id: &'static str,
    /// The asset verbatim, as `theme show` prints it.
    pub source: &'static str,
}

/// The built-ins in registration order.
///
/// `default` must stay first: it is the root every other theme inherits from,
/// and the order is what the theme picker and `theme list` present.
pub static BUILT_IN_THEMES: &[BuiltInTheme] = &[
    BuiltInTheme {
        id: "default",
        source: include_str!("../../assets/themes/default.toml"),
    },
    BuiltInTheme {
        id: "summer",
        source: include_str!("../../assets/themes/summer.toml"),
    },
    BuiltInTheme {
        id: "aqua",
        source: include_str!("../../assets/themes/aqua.toml"),
    },
    BuiltInTheme {
        id: "fire",
        source: include_str!("../../assets/themes/fire.toml"),
    },
    BuiltInTheme {
        id: "high-contrast",
        source: include_str!("../../assets/themes/high-contrast.toml"),
    },
];

/// Whether `id` names a built-in, i.e. whether it is reserved for one.
pub fn is_reserved(id: &str) -> bool {
    BUILT_IN_THEMES.iter().any(|theme| theme.id == id)
}

/// The embedded source of a built-in.
pub fn source(id: &str) -> Option<&'static str> {
    BUILT_IN_THEMES
        .iter()
        .find(|theme| theme.id == id)
        .map(|theme| theme.source)
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Modifier, Style};

    use crate::theme::catalog::{ColorRole, PaintRole};
    use crate::theme::catalog::{
        RoleFallback, RoleRef, SemanticSlot, SemanticStyle, SemanticTint, StyleRole, ROLE_SPECS,
        SEMANTIC_SPECS,
    };
    use crate::theme::model::{
        GradientDirection, ResolvedPaint, ResolvedTheme, ResolvedTint, ThemeId, ValidationMode,
    };
    use crate::theme::registry::{ThemeRegistry, ThemeStatus};
    use crate::tui::theme as legacy;

    #[test]
    fn all_builtins_pass_the_public_strict_pipeline() {
        let registry = ThemeRegistry::builtins(ValidationMode::Strict).unwrap();
        let ids: Vec<_> = registry.records().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["default", "summer", "aqua", "fire", "high-contrast"]);
        assert!(
            registry
                .records()
                .iter()
                .all(|r| r.status == ThemeStatus::Valid),
            "{:#?}",
            registry
                .records()
                .iter()
                .flat_map(|r| r.diagnostics.iter())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn default_matches_every_frozen_legacy_fallback() {
        let default = ThemeRegistry::builtins(ValidationMode::Strict)
            .unwrap()
            .resolved(&ThemeId::parse("default").unwrap())
            .unwrap();
        assert_default_role_parity(&default);
    }

    #[test]
    fn each_builtin_demonstrates_the_features_it_documents() {
        let registry = ThemeRegistry::builtins(ValidationMode::Strict).unwrap();
        let resolved = |id: &str| {
            registry
                .resolved(&ThemeId::parse(id).unwrap())
                .unwrap_or_else(|| panic!("{id} resolves"))
        };

        // `default` is the compatibility baseline: transparent, undecorated.
        let default = resolved("default");
        assert!(default.gradients.is_empty());
        assert_eq!(default.semantic.background, Color::Reset);

        // `summer` paints a light ground, which is also what makes `opacity`
        // usable: the implicit mixing ground is `semantic.background`.
        let summer = resolved("summer");
        assert!(!summer.gradients.is_empty());
        assert_eq!(summer.semantic.background, Color::Rgb(0xff, 0xf8, 0xe7));
        assert!(matches!(
            *summer.paint(PaintRole::SeparatorPrimary),
            ResolvedPaint::Gradient(_)
        ));
        // #f5b301 at 22% over #fff8e7, per channel in sRGB and rounded.
        assert_eq!(
            *summer.paint(PaintRole::FooterBackground),
            ResolvedPaint::Solid(Color::Rgb(253, 233, 180))
        );

        // `aqua` demonstrates the perimeter ring, whose ends have to meet.
        let aqua = resolved("aqua");
        let directions: Vec<_> = aqua.gradients.iter().map(|g| g.direction).collect();
        assert!(directions.contains(&GradientDirection::Horizontal));
        assert!(directions.contains(&GradientDirection::Perimeter));
        let ring = aqua
            .paint_gradient(PaintRole::PopupBorder)
            .expect("aqua rings its popup border");
        assert_eq!(ring.direction, GradientDirection::Perimeter);
        assert_eq!(
            ring.stops.first().unwrap().color,
            ring.stops.last().unwrap().color,
            "a perimeter ring must close without a seam"
        );

        // `fire` demonstrates both diagonals, and keeps its statuses out of the
        // decorative red/orange/gold ramp so a failure never reads as chrome.
        let fire = resolved("fire");
        let directions: Vec<_> = fire.gradients.iter().map(|g| g.direction).collect();
        assert!(directions.contains(&GradientDirection::DiagonalUp));
        assert!(directions.contains(&GradientDirection::DiagonalDown));
        let statuses = [
            fire.semantic.success,
            fire.semantic.warning,
            fire.semantic.error,
            fire.semantic.info,
            fire.semantic.accent,
            fire.semantic.border_focus,
        ];
        for (index, left) in statuses.iter().enumerate() {
            for right in &statuses[index + 1..] {
                assert_ne!(left, right, "fire's signal colours must stay distinct");
            }
        }

        // `high-contrast` proves gradients are entirely optional.
        let high_contrast = resolved("high-contrast");
        assert!(high_contrast.gradients.is_empty());
        assert_eq!(high_contrast.semantic.background, Color::Rgb(0, 0, 0));
        assert_eq!(high_contrast.semantic.text, Color::Rgb(0xff, 0xff, 0xff));
    }

    #[test]
    fn every_builtin_describes_itself_the_right_way_round() {
        let registry = ThemeRegistry::builtins(ValidationMode::Strict).unwrap();
        for record in registry.records() {
            assert!(!record.name.is_empty(), "{} has no name", record.id);
            let description = record
                .description
                .as_deref()
                .unwrap_or_else(|| panic!("{} has no description", record.id));
            // The description is the only prose the picker and `theme list`
            // show, so a theme that describes itself backwards misinforms the
            // user at exactly the moment they are choosing.
            assert!(
                !description.is_empty(),
                "{} has an empty description",
                record.id
            );

            // Pin the polarity of the one description that states a
            // foreground-on-background pair explicitly.
            if record.id.as_str() == "high-contrast" {
                let theme = record.resolved().expect("high-contrast resolves");
                assert_eq!(theme.semantic.background, Color::Rgb(0, 0, 0));
                assert_eq!(theme.semantic.text, Color::Rgb(0xff, 0xff, 0xff));
                assert!(
                    description.starts_with("White on pure black"),
                    "the description states the pair the wrong way round: {description}"
                );
            }
        }
    }

    /// The colour the pre-theme-system renderers used for each semantic slot.
    ///
    /// Every right-hand side is a frozen constant of `src/tui/theme.rs`; nothing
    /// here may be derived from the asset it verifies, otherwise the parity
    /// check would only prove the asset is self-consistent.
    fn legacy_semantic(slot: SemanticSlot) -> Color {
        match slot {
            // The app never painted its own ground: unstyled cells kept the
            // emulator's default, which is what `"terminal"` resolves to.
            SemanticSlot::Background => Color::Reset,
            SemanticSlot::Canvas => legacy::BG,
            SemanticSlot::Surface | SemanticSlot::SurfaceRaised => Color::Reset,
            SemanticSlot::Border => legacy::BORDER,
            SemanticSlot::BorderFocus => legacy::CYAN,
            SemanticSlot::BorderPopup => legacy::MUTE,
            SemanticSlot::Text => legacy::TEXT,
            SemanticSlot::TextBright => legacy::BRIGHT,
            SemanticSlot::TextHighlight => legacy::WHITE,
            SemanticSlot::TextMuted => legacy::MUTE,
            SemanticSlot::TextDim => legacy::DIM,
            SemanticSlot::TextInverse => legacy::BG_DEEP,
            SemanticSlot::Accent => legacy::ACCENT,
            SemanticSlot::SelectionBg => legacy::SEL_BG,
            SemanticSlot::SelectionFg => legacy::SEL_FG,
            SemanticSlot::Success => legacy::GREEN,
            SemanticSlot::Warning | SemanticSlot::Connecting => legacy::AMBER,
            SemanticSlot::Error | SemanticSlot::Exited => legacy::RED,
            SemanticSlot::Info => legacy::CYAN,
            SemanticSlot::Unknown => legacy::DIM,
        }
    }

    /// The [`Style`] the legacy helpers produced for each fallback recipe.
    fn legacy_style(recipe: SemanticStyle) -> Style {
        match recipe {
            SemanticStyle::Text => legacy::text(),
            SemanticStyle::TextBright => legacy::bright(),
            SemanticStyle::TextBrightBold => legacy::heading(),
            SemanticStyle::TextBrightUnderlinedBold => {
                legacy::bright().add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
            }
            SemanticStyle::TextHighlight => legacy::white(),
            SemanticStyle::TextMuted => legacy::mute(),
            SemanticStyle::TextDim => legacy::dim(),
            SemanticStyle::TextOnSurfaceRaised => {
                Style::default().fg(legacy::TEXT).bg(Color::Reset)
            }
            SemanticStyle::HighlightOnSelection => {
                Style::default().fg(legacy::WHITE).bg(legacy::SEL_BG)
            }
            SemanticStyle::Selection => legacy::selected(),
            SemanticStyle::Inverse => legacy::inv(),
            SemanticStyle::InverseOnWarning => {
                Style::default().fg(legacy::BG_DEEP).bg(legacy::AMBER)
            }
            SemanticStyle::Accent => Style::default().fg(legacy::ACCENT),
            SemanticStyle::Info => legacy::cyan(),
            SemanticStyle::Success => legacy::green(),
            SemanticStyle::Warning => legacy::amber(),
            SemanticStyle::Error => legacy::red(),
        }
    }

    /// The roles `default` overrides on purpose, because their semantic
    /// fallback does not reproduce what SSHub drew. Skipped by the blanket
    /// fallback loop and asserted individually instead — the list is short by
    /// design, and every entry has to earn its place.
    const DEFAULT_PARITY_OVERRIDES: &[&str] = &[
        "components.dashboard.host_list.host_selected",
        "components.dashboard.host_list.group",
        "components.popup.title",
        "components.help.section",
    ];

    /// Assert that `theme` reproduces the frozen pre-theme-system appearance for
    /// the whole semantic core and for **every** row of [`ROLE_SPECS`].
    fn assert_default_role_parity(theme: &ResolvedTheme) {
        for spec in SEMANTIC_SPECS {
            assert_eq!(
                theme.semantic.slot(spec.slot),
                legacy_semantic(spec.slot),
                "semantic.{}",
                spec.key
            );
        }

        for spec in ROLE_SPECS {
            // `default` carries a handful of deliberate component overrides,
            // asserted explicitly below against the legacy surface each one
            // reproduces. Their semantic fallback is *not* what SSHub drew,
            // which is precisely why the override exists.
            if DEFAULT_PARITY_OVERRIDES.contains(&spec.path) {
                continue;
            }
            match (spec.role, spec.fallback) {
                (RoleRef::Color(role), RoleFallback::Color(slot)) => {
                    assert_eq!(theme.color(role), legacy_semantic(slot), "{}", spec.path)
                }
                (RoleRef::Style(role), RoleFallback::Style(recipe)) => {
                    assert_eq!(theme.style(role), legacy_style(recipe), "{}", spec.path)
                }
                (RoleRef::Paint(role), RoleFallback::Paint(slot)) => assert_eq!(
                    *theme.paint(role),
                    ResolvedPaint::Solid(legacy_semantic(slot)),
                    "{}",
                    spec.path
                ),
                (RoleRef::Tint(role), RoleFallback::Tint(tint)) => {
                    let expected = match tint {
                        SemanticTint::Native => ResolvedTint::Native,
                        SemanticTint::Color(slot) => ResolvedTint::Color(legacy_semantic(slot)),
                    };
                    assert_eq!(*theme.tint(role), expected, "{}", spec.path);
                }
                _ => panic!("{} pairs a fallback of a different kind", spec.path),
            }
        }

        // The cases the loop would still pass if two slots were swapped, spelled
        // out against the exact legacy helper each one replaces.

        // `theme::inv()` — deep background on bright text, not the other way round.
        assert_eq!(theme.style(StyleRole::TextInverse), legacy::inv());
        assert_eq!(theme.style(StyleRole::HeaderBrand), legacy::inv());
        assert_eq!(theme.style(StyleRole::TabsActive), legacy::inv());
        assert_eq!(theme.style(StyleRole::StatusBarMode), legacy::inv());
        assert_eq!(theme.semantic.text_inverse, legacy::BG_DEEP);
        assert_ne!(theme.semantic.text_inverse, theme.semantic.text_bright);

        // The documented `default` overrides, against the exact legacy
        // helper each replaces. The selected *host name* was `theme::selected()`
        // (SEL_FG); `text_highlight` (WHITE) is the selected *group* label, and
        // the two are not interchangeable.
        assert_ne!(legacy::WHITE, legacy::SEL_FG);
        assert_eq!(
            theme.style(StyleRole::DashboardHostListHostSelected),
            legacy::selected(),
            "the selected host name"
        );
        assert_eq!(
            theme.style(StyleRole::DashboardHostListGroup),
            legacy::white(),
            "the group label"
        );

        // Popup titles were `theme::heading()` — bright *and* bold. The
        // catalogue fallback is plain `text_bright`, so comparing this role
        // against its own fallback (as the loop above does) would have frozen
        // the wrong assumption instead of the legacy cell.
        assert_eq!(
            theme.style(StyleRole::PopupTitle),
            legacy::heading(),
            "a popup title"
        );
        assert!(theme
            .style(StyleRole::PopupTitle)
            .add_modifier
            .contains(Modifier::BOLD));
        assert_ne!(
            theme.style(StyleRole::PopupTitle),
            legacy::bright(),
            "the plain fallback would drop the weight"
        );

        assert_eq!(
            theme.style(StyleRole::CommandPaletteRowSelected),
            Style::default().fg(legacy::WHITE).bg(legacy::SEL_BG)
        );
        assert_eq!(
            theme.style(StyleRole::PickerRowSelected),
            legacy::selected()
        );

        // Focused panel borders are cyan, popup borders are the brighter mute —
        // `theme::border()` vs `theme::popup_border()`.
        assert_eq!(
            *theme.paint(PaintRole::DashboardHostListBorder),
            ResolvedPaint::Solid(legacy::BORDER)
        );
        assert_eq!(
            *theme.paint(PaintRole::DashboardHostListBorderFocused),
            ResolvedPaint::Solid(legacy::CYAN)
        );
        assert_eq!(
            *theme.paint(PaintRole::PopupBorder),
            ResolvedPaint::Solid(legacy::MUTE)
        );
        assert_ne!(legacy::CYAN, legacy::MUTE);

        // Surfaces stayed transparent: panels never painted a background.
        for role in [
            PaintRole::AppBackground,
            PaintRole::SessionBackground,
            PaintRole::PopupBackground,
            PaintRole::HeaderBackground,
            PaintRole::FooterBackground,
            PaintRole::StatusBarBackground,
            PaintRole::DashboardHostListBackground,
            PaintRole::DashboardDetailsBackground,
            PaintRole::SftpPanelBackground,
            PaintRole::BroadcastPanelBackground,
        ] {
            assert_eq!(
                *theme.paint(role),
                ResolvedPaint::Solid(Color::Reset),
                "{role:?} must stay transparent"
            );
        }

        // Two selection idioms coexist and must not converge: the tunnels tab,
        // the picker and the group list highlight with SEL_FG, while the
        // settings, palette and keybind rows brighten to WHITE.
        assert_eq!(
            theme.style(StyleRole::TunnelsRowSelected),
            legacy::selected()
        );
        assert_eq!(theme.style(StyleRole::TableRowSelected), legacy::selected());
        assert_ne!(
            theme.style(StyleRole::TunnelsRowSelected),
            theme.style(StyleRole::SettingsRowSelected)
        );
        assert_eq!(
            theme.style(StyleRole::TunnelsTableHeader),
            legacy::heading()
        );
        assert_eq!(theme.color(ColorRole::TunnelRunning), legacy::GREEN);
        assert_eq!(theme.color(ColorRole::TunnelStopped), legacy::RED);
        assert_eq!(theme.color(ColorRole::TunnelRetrying), legacy::AMBER);
        assert_eq!(theme.color(ColorRole::TunnelUnknown), legacy::DIM);

        // Identity cards select with SEL_FG and mark the selected card with the
        // accent border rather than the focus border.
        assert_eq!(
            theme.style(StyleRole::IdentitiesCardSelection),
            legacy::selected()
        );
        assert_eq!(
            *theme.paint(PaintRole::IdentitiesCardBorderSelected),
            ResolvedPaint::Solid(legacy::ACCENT)
        );
        assert_eq!(
            *theme.paint(PaintRole::IdentitiesCardBorder),
            ResolvedPaint::Solid(legacy::BORDER)
        );
        assert_eq!(
            theme.style(StyleRole::IdentitiesCardName),
            legacy::heading()
        );

        assert_task14_legacy_cells(theme);
    }
    // ── Per-renderer legacy inventory for the Task 14 surfaces ──
    //
    // The blanket loop above is circular *as a class*: it derives its expected
    // value from `ROLE_SPECS[*].fallback`, the same source the productive role
    // resolves from. A fallback that was mis-assigned when it was written is
    // therefore wrong on both sides and the loop stays green — which is how
    // four overlay regressions, and then the five field markers, survived
    // review.
    //
    // A witness keyed on *role paths alone* is not enough either: a path that
    // is forgotten in both the witness and its guard is invisible, and a cell
    // whose appearance does not follow its role at all — the palette's opaque
    // background — cannot be expressed as a role value.
    //
    // So the inventory below has **one row per `(renderer, role)` pair**: where
    // the role is drawn, every legacy source that pair replaced, and what
    // `default` must produce. Two things keep it honest, and both are machine
    // -checked: the values are hand-written from the `crate::tui::theme` calls
    // they replaced, never from `ROLE_SPECS`; and the *set* of pairs is derived
    // from the renderers' own source, so a role a renderer reads without an
    // inventory row fails.
    //
    // **What this does and does not prove.** It proves per-renderer *role*
    // coverage: no Task 14 renderer reads a role this table has not accounted
    // for, and every accounted role reproduces its legacy value under
    // `default`. It does **not** prove per-cell coverage — a renderer that
    // draws a role it already uses at one more place adds no new row, because
    // the check is textual and deduplicates identifiers per file. Proving that
    // would need an AST pass with source spans, and this crate may not take the
    // dependency. Cell-level parity is instead proved where it belongs: by the
    // productive goldens in `crate::tui::tests`, which render the real frame
    // and read named cells back — `the_overlays_reproduce_their_legacy_cells_
    // under_default`, `the_field_markers_reproduce_their_legacy_cells_under_
    // default`, `the_forms_uncoloured_cells_keep_their_documented_roles` and
    // the per-surface marker proofs.
    //
    // The textual scan is also blind to `use StyleRole::*`, to a role reached
    // through a macro or a helper in another file, to a role named only in a
    // comment or a string, and to a renderer left out of `TASK14_RENDERERS`
    // altogether. Adding a Task 14 surface therefore still requires adding it
    // to that list by hand.
    //
    // Roles whose legacy source was a *direct ANSI colour* carry
    // [`Task14Expect::Ansi`]: the spec allows those onto the semantic core and
    // there is no `theme.rs` cell to be faithful to. Cells that carried no
    // colour at all are **not** that exception and carry
    // [`Task14Expect::Unstyled`] with their own recorded reasoning.

    /// What `default` must produce for one migrated role use.
    #[derive(Clone, Copy)]
    enum Task14Expect {
        /// The cells follow the role, and the role must equal this legacy value.
        Style(Style),
        Color(Color),
        Paint(Color),
        /// The legacy source really was a direct ANSI colour, which the spec's
        /// one parity exception allows onto the semantic core. Carries the
        /// colour itself, and the guard rejects anything that is not a named
        /// ANSI colour — the exception may not be stretched over cells that
        /// simply had no colour.
        Ansi(Color),
        /// The legacy cells carried **no colour at all** — the terminal's own
        /// foreground, sometimes with a bare `DIM` modifier. That is not the
        /// ANSI exception, so each one is a deliberate deviation recorded in
        /// the spec, with its reasoning, rather than absorbed silently.
        Unstyled {
            was: &'static str,
            why: &'static str,
        },
        /// The cells deliberately do *not* wear the role's `default` value;
        /// the renderer documents a substitution. Names the reason and the
        /// productive test that proves them instead.
        Context {
            why: &'static str,
            proof: &'static str,
        },
    }

    /// One `(renderer, role)` pair this task migrated.
    ///
    /// **Not one cell.** A renderer usually draws several cells from the same
    /// role — `src/tui/mod.rs` puts `PopupTitle` on three popup frames — and
    /// this row covers all of them together; `was` therefore lists every legacy
    /// source that pair replaced. What the guard below proves is that no role a
    /// Task 14 renderer reads is missing from this table, not that every
    /// individual `set_string` has its own row. Cell-level parity is proved by
    /// the productive goldens in `crate::tui::tests`, which render the real
    /// frame and read named cells back.
    #[derive(Clone, Copy)]
    struct Task14RoleUse {
        /// Stable `<surface>.<role>` id, used in failure messages.
        id: &'static str,
        /// The renderer that draws it today, relative to the crate root.
        renderer: &'static str,
        /// Every pre-migration source this pair replaced, verbatim.
        was: &'static str,
        /// The role's Rust identifier, as the renderer spells it.
        ident: &'static str,
        role: RoleRef,
        expect: Task14Expect,
    }

    const MOD: &str = "src/tui/mod.rs";
    const PALETTE: &str = "src/tui/screens/palette.rs";
    const FIELD_PICKER: &str = "src/tui/screens/field_picker.rs";
    const GROUP_FORM: &str = "src/tui/screens/group_form.rs";
    const GROUP_MANAGE: &str = "src/tui/screens/group_manage.rs";
    const HOST_FORM: &str = "src/tui/screens/host_form.rs";
    const TAG_FILTER: &str = "src/tui/screens/tag_filter.rs";
    const SESSION_PICKER: &str = "src/tui/screens/session_picker.rs";
    const SETTINGS: &str = "src/tui/screens/settings.rs";
    const KEYBIND: &str = "src/tui/screens/keybind_editor.rs";
    const KEYCHAIN: &str = "src/tui/screens/keychain.rs";
    const KEYS: &str = "src/tui/screens/keys.rs";
    const HELP: &str = "src/tui/screens/help.rs";
    const TUNNEL_RECONNECT: &str = "src/tui/screens/tunnel_reconnect.rs";

    /// Every renderer whose roles this task owns. The completeness guard reads
    /// these files, so a call site added to one of them without an inventory
    /// row fails.
    const TASK14_RENDERERS: &[&str] = &[
        MOD,
        PALETTE,
        FIELD_PICKER,
        GROUP_FORM,
        GROUP_MANAGE,
        HOST_FORM,
        TAG_FILTER,
        SESSION_PICKER,
        SETTINGS,
        KEYBIND,
        KEYCHAIN,
        KEYS,
        HELP,
        TUNNEL_RECONNECT,
    ];

    /// Roles read by a Task 14 renderer that belong to an earlier task.
    ///
    /// `src/tui/mod.rs` is the whole frame, so it also draws the shared chrome
    /// Task 12 owns. Anything not listed here has to appear in the inventory,
    /// which is what makes a newly added call site fail rather than pass
    /// unnoticed.
    const NOT_TASK14: &[(&str, &str)] = &[
        (MOD, "AppBackground"),
        (MOD, "HeaderSeparator"),
        (MOD, "FooterSeparator"),
        (MOD, "TabsSeparator"),
        (MOD, "StatusBarBackground"),
    ];

    /// The role uses, grouped by the surface that draws them.
    fn task14_role_uses() -> Vec<Task14RoleUse> {
        use Task14Expect::{Ansi, Color as C, Context, Paint, Unstyled};
        let sel_bg = legacy::SEL_BG;
        vec![
            // ── Generic popup chrome, `src/tui/mod.rs` ──────────
            Task14RoleUse {
                id: "popup.title",
                renderer: MOD,
                was: "theme::heading() on every popup Block title",
                ident: "PopupTitle",
                role: RoleRef::Style(StyleRole::PopupTitle),
                expect: Task14Expect::Style(legacy::heading()),
            },
            Task14RoleUse {
                id: "popup.hint",
                renderer: MOD,
                was: "theme::dim() on the help footer and the prompt legends",
                ident: "PopupHint",
                role: RoleRef::Style(StyleRole::PopupHint),
                expect: Task14Expect::Style(legacy::dim()),
            },
            Task14RoleUse {
                id: "popup.border",
                renderer: MOD,
                was: "theme::popup_border()",
                ident: "PopupBorder",
                role: RoleRef::Paint(PaintRole::PopupBorder),
                expect: Paint(legacy::MUTE),
            },
            Task14RoleUse {
                id: "popup.background",
                renderer: MOD,
                was: "no fill at all: `Clear` left the cells at the terminal ground",
                ident: "PopupBackground",
                role: RoleRef::Paint(PaintRole::PopupBackground),
                expect: Paint(Color::Reset),
            },
            Task14RoleUse {
                id: "confirm.error",
                renderer: MOD,
                was: "Style::default().fg(Color::Red)",
                ident: "PopupError",
                role: RoleRef::Style(StyleRole::PopupError),
                expect: Ansi(Color::Red),
            },
            Task14RoleUse {
                id: "confirm.warning",
                renderer: MOD,
                was: "Style::default().fg(Color::Yellow)",
                ident: "PopupWarning",
                role: RoleRef::Style(StyleRole::PopupWarning),
                expect: Ansi(Color::Yellow),
            },
            Task14RoleUse {
                id: "form_popup.notice",
                renderer: MOD,
                was: "Style::default().fg(Color::Red)",
                ident: "FormError",
                role: RoleRef::Style(StyleRole::FormError),
                expect: Ansi(Color::Red),
            },
            Task14RoleUse {
                id: "prompt.label",
                renderer: MOD,
                was: "theme::text() on the SFTP and Termius prompt lines",
                ident: "TextPrimary",
                role: RoleRef::Style(StyleRole::TextPrimary),
                expect: Task14Expect::Style(legacy::text()),
            },
            Task14RoleUse {
                id: "prompt.value",
                renderer: MOD,
                was: "theme::bright() on the typed path or name",
                ident: "FormInput",
                role: RoleRef::Style(StyleRole::FormInput),
                expect: Task14Expect::Style(legacy::bright()),
            },
            // ── Fuzzy palette ──────────────────────────────────
            Task14RoleUse {
                id: "palette.background",
                renderer: PALETTE,
                was: "Block::style(Style::default().bg(theme::BG))",
                ident: "PopupBackground",
                role: RoleRef::Paint(PaintRole::PopupBackground),
                expect: Context {
                    why: "the palette is the one overlay that has always been opaque, \
                          while `popup.background` is transparent in `default` like \
                          every surface role; where the role resolves to the terminal \
                          ground the palette substitutes `semantic.canvas`, which under \
                          `default` is literally the former `theme::BG`",
                    proof: "tui::tests::palette_popup_interior_filled_with_theme_bg",
                },
            },
            Task14RoleUse {
                id: "palette.title",
                renderer: PALETTE,
                was: "theme::heading()",
                ident: "PopupTitle",
                role: RoleRef::Style(StyleRole::PopupTitle),
                expect: Task14Expect::Style(legacy::heading()),
            },
            Task14RoleUse {
                id: "palette.query",
                renderer: PALETTE,
                was: "theme::white()",
                ident: "CommandPaletteQuery",
                role: RoleRef::Style(StyleRole::CommandPaletteQuery),
                expect: Task14Expect::Style(legacy::white()),
            },
            Task14RoleUse {
                id: "palette.row_selected",
                renderer: PALETTE,
                was: "theme::white().bg(theme::SEL_BG)",
                ident: "CommandPaletteRowSelected",
                role: RoleRef::Style(StyleRole::CommandPaletteRowSelected),
                expect: Task14Expect::Style(legacy::white().bg(sel_bg)),
            },
            Task14RoleUse {
                id: "palette.row_name",
                renderer: PALETTE,
                was: "theme::bright() on an unselected host name",
                ident: "TextBright",
                role: RoleRef::Style(StyleRole::TextBright),
                expect: Task14Expect::Style(legacy::bright()),
            },
            Task14RoleUse {
                id: "palette.detail_key",
                renderer: PALETTE,
                was: "theme::mute() on the counter, group, hint and detail keys",
                ident: "PopupLegend",
                role: RoleRef::Style(StyleRole::PopupLegend),
                expect: Task14Expect::Style(legacy::mute()),
            },
            Task14RoleUse {
                id: "palette.detail_value",
                renderer: PALETTE,
                was: "theme::text() on a detail value",
                ident: "TextPrimary",
                role: RoleRef::Style(StyleRole::TextPrimary),
                expect: Task14Expect::Style(legacy::text()),
            },
            Task14RoleUse {
                id: "palette.user_column",
                renderer: PALETTE,
                was: "theme::dim() on the user column",
                ident: "PopupHint",
                role: RoleRef::Style(StyleRole::PopupHint),
                expect: Task14Expect::Style(legacy::dim()),
            },
            Task14RoleUse {
                id: "palette.rule",
                renderer: PALETTE,
                was: "theme::border() on the two inner rules",
                ident: "SeparatorPrimary",
                role: RoleRef::Paint(PaintRole::SeparatorPrimary),
                expect: Paint(legacy::BORDER),
            },
            Task14RoleUse {
                id: "palette.prompt_caret",
                renderer: PALETTE,
                was: "theme::green() on the prompt marker, caret and selection arrow",
                ident: "StatusSuccess",
                role: RoleRef::Color(ColorRole::StatusSuccess),
                expect: C(legacy::GREEN),
            },
            // ── Field picker ───────────────────────────────────
            Task14RoleUse {
                id: "field_picker.title",
                renderer: FIELD_PICKER,
                was: "theme::heading()",
                ident: "PopupTitle",
                role: RoleRef::Style(StyleRole::PopupTitle),
                expect: Task14Expect::Style(legacy::heading()),
            },
            Task14RoleUse {
                id: "field_picker.row",
                renderer: FIELD_PICKER,
                was: "theme::text()",
                ident: "PickerRow",
                role: RoleRef::Style(StyleRole::PickerRow),
                expect: Task14Expect::Style(legacy::text()),
            },
            Task14RoleUse {
                id: "field_picker.row_selected",
                renderer: FIELD_PICKER,
                was: "theme::selected()",
                ident: "PickerRowSelected",
                role: RoleRef::Style(StyleRole::PickerRowSelected),
                expect: Task14Expect::Style(legacy::selected()),
            },
            Task14RoleUse {
                id: "field_picker.marker",
                renderer: FIELD_PICKER,
                was: "the marker inside the selected row's own theme::selected() label",
                ident: "PickerMarker",
                role: RoleRef::Style(StyleRole::PickerMarker),
                expect: Task14Expect::Style(legacy::selected()),
            },
            Task14RoleUse {
                id: "field_picker.create_row",
                renderer: FIELD_PICKER,
                was: "theme::green() on the `+ New group` row",
                ident: "StatusSuccess",
                role: RoleRef::Color(ColorRole::StatusSuccess),
                expect: C(legacy::GREEN),
            },
            Task14RoleUse {
                id: "field_picker.inline_input",
                renderer: FIELD_PICKER,
                was: "theme::bright() on the inline new-group name",
                ident: "FormInput",
                role: RoleRef::Style(StyleRole::FormInput),
                expect: Task14Expect::Style(legacy::bright()),
            },
            // ── Group form and its dropdown ────────────────────
            Task14RoleUse {
                id: "group_form.title",
                renderer: GROUP_FORM,
                was: "theme::heading()",
                ident: "PopupTitle",
                role: RoleRef::Style(StyleRole::PopupTitle),
                expect: Task14Expect::Style(legacy::heading()),
            },
            Task14RoleUse {
                id: "group_form.label",
                renderer: GROUP_FORM,
                was: "theme::mute()",
                ident: "GroupFormLabel",
                role: RoleRef::Style(StyleRole::GroupFormLabel),
                expect: Task14Expect::Style(legacy::mute()),
            },
            Task14RoleUse {
                id: "group_form.label_focused",
                renderer: GROUP_FORM,
                was: "theme::heading()",
                ident: "GroupFormLabelFocused",
                role: RoleRef::Style(StyleRole::GroupFormLabelFocused),
                expect: Task14Expect::Style(legacy::heading()),
            },
            Task14RoleUse {
                id: "group_form.value",
                renderer: GROUP_FORM,
                was: "theme::text()",
                ident: "GroupFormValue",
                role: RoleRef::Style(StyleRole::GroupFormValue),
                expect: Task14Expect::Style(legacy::text()),
            },
            Task14RoleUse {
                id: "group_form.value_focused",
                renderer: GROUP_FORM,
                was: "theme::bright().add_modifier(Modifier::BOLD)",
                ident: "GroupFormValueFocused",
                role: RoleRef::Style(StyleRole::GroupFormValueFocused),
                expect: Task14Expect::Style(legacy::bright().add_modifier(Modifier::BOLD)),
            },
            Task14RoleUse {
                id: "group_form.marker",
                renderer: GROUP_FORM,
                was: "the marker inside the focused theme::heading() label",
                ident: "GroupFormMarker",
                role: RoleRef::Style(StyleRole::GroupFormMarker),
                expect: Task14Expect::Style(legacy::heading()),
            },
            Task14RoleUse {
                id: "group_form.hint",
                renderer: GROUP_FORM,
                was: "theme::dim() on the key hints and `Enter to choose`",
                ident: "FormHelp",
                role: RoleRef::Style(StyleRole::FormHelp),
                expect: Task14Expect::Style(legacy::dim()),
            },
            Task14RoleUse {
                id: "group_form.picker_row",
                renderer: GROUP_FORM,
                was: "theme::text() on a dropdown option",
                ident: "PickerRow",
                role: RoleRef::Style(StyleRole::PickerRow),
                expect: Task14Expect::Style(legacy::text()),
            },
            Task14RoleUse {
                id: "group_form.picker_row_selected",
                renderer: GROUP_FORM,
                was: "List::highlight_style(theme::selected())",
                ident: "PickerRowSelected",
                role: RoleRef::Style(StyleRole::PickerRowSelected),
                expect: Task14Expect::Style(legacy::selected()),
            },
            Task14RoleUse {
                id: "group_form.picker_none",
                renderer: GROUP_FORM,
                was: "theme::mute() on the `(none)` row",
                ident: "PopupLegend",
                role: RoleRef::Style(StyleRole::PopupLegend),
                expect: Task14Expect::Style(legacy::mute()),
            },
            // ── Group management ───────────────────────────────
            Task14RoleUse {
                id: "group_manage.title",
                renderer: GROUP_MANAGE,
                was: "theme::heading()",
                ident: "PopupTitle",
                role: RoleRef::Style(StyleRole::PopupTitle),
                expect: Task14Expect::Style(legacy::heading()),
            },
            Task14RoleUse {
                id: "group_manage.row",
                renderer: GROUP_MANAGE,
                was: "theme::text() on a group name",
                ident: "TableRow",
                role: RoleRef::Style(StyleRole::TableRow),
                expect: Task14Expect::Style(legacy::text()),
            },
            Task14RoleUse {
                id: "group_manage.row_selected",
                renderer: GROUP_MANAGE,
                was: "List::highlight_style(theme::selected())",
                ident: "TableRowSelected",
                role: RoleRef::Style(StyleRole::TableRowSelected),
                expect: Task14Expect::Style(legacy::selected()),
            },
            Task14RoleUse {
                id: "group_manage.indent",
                renderer: GROUP_MANAGE,
                was: "theme::mute() on the indent, the count and the empty state",
                ident: "PopupLegend",
                role: RoleRef::Style(StyleRole::PopupLegend),
                expect: Task14Expect::Style(legacy::mute()),
            },
            Task14RoleUse {
                id: "group_manage.hint",
                renderer: GROUP_MANAGE,
                was: "theme::dim() on the action hint",
                ident: "PopupHint",
                role: RoleRef::Style(StyleRole::PopupHint),
                expect: Task14Expect::Style(legacy::dim()),
            },
            // ── Host form (direct ANSI throughout) ─────────────
            Task14RoleUse {
                id: "host_form.title",
                renderer: HOST_FORM,
                was: "Block::title(title) with no style of its own",
                ident: "PopupTitle",
                role: RoleRef::Style(StyleRole::PopupTitle),
                expect: Unstyled {
                    was: "an unstyled Block title, i.e. the terminal's own foreground",
                    why: "every other overlay titles itself with `popup.title`; leaving \
                          these two unstyled would put an unthemeable title on a themed \
                          popup ground and make the role unreachable from them",
                },
            },
            Task14RoleUse {
                id: "host_form.label",
                renderer: HOST_FORM,
                was: "Style::default().fg(Color::DarkGray)",
                ident: "FormLabel",
                role: RoleRef::Style(StyleRole::FormLabel),
                expect: Ansi(Color::DarkGray),
            },
            Task14RoleUse {
                id: "host_form.label_focused",
                renderer: HOST_FORM,
                was: "Style::default().fg(Color::Cyan).add_modifier(BOLD)",
                ident: "FormLabelFocused",
                role: RoleRef::Style(StyleRole::FormLabelFocused),
                expect: Ansi(Color::Cyan),
            },
            Task14RoleUse {
                id: "host_form.label_editing",
                renderer: HOST_FORM,
                was: "Style::default().fg(Color::Yellow).add_modifier(BOLD)",
                ident: "FormLabelEditing",
                role: RoleRef::Style(StyleRole::FormLabelEditing),
                expect: Ansi(Color::Yellow),
            },
            Task14RoleUse {
                id: "host_form.value",
                renderer: HOST_FORM,
                was: "Style::default() — an idle value had no style at all",
                ident: "FormValue",
                role: RoleRef::Style(StyleRole::FormValue),
                expect: Unstyled {
                    was: "Style::default(), i.e. the terminal's own foreground",
                    why: "an idle value that carries no colour cannot be themed at all; \
                          `form.value` gives it the body-text role the rest of the app \
                          already uses for exactly this kind of text",
                },
            },
            Task14RoleUse {
                id: "host_form.value_focused",
                renderer: HOST_FORM,
                was: "Style::default().fg(Color::White).add_modifier(BOLD)",
                ident: "FormInputFocused",
                role: RoleRef::Style(StyleRole::FormInputFocused),
                expect: Ansi(Color::White),
            },
            Task14RoleUse {
                id: "host_form.value_editing",
                renderer: HOST_FORM,
                was: "Style::default().fg(Color::White).add_modifier(BOLD | UNDERLINED)",
                ident: "FormInputEditing",
                role: RoleRef::Style(StyleRole::FormInputEditing),
                expect: Ansi(Color::White),
            },
            Task14RoleUse {
                id: "host_form.hint",
                renderer: HOST_FORM,
                was: "Style::default().add_modifier(Modifier::DIM)",
                ident: "FormHelp",
                role: RoleRef::Style(StyleRole::FormHelp),
                expect: Unstyled {
                    was: "Style::default().add_modifier(Modifier::DIM), no colour",
                    why: "the bare DIM modifier is terminal-dependent and several \
                          emulators ignore it; `form.help` states the same intent as a \
                          colour the theme controls",
                },
            },
            Task14RoleUse {
                id: "host_form.marker",
                renderer: HOST_FORM,
                was: "the marker inside the ANSI-coloured label span",
                ident: "FocusIndicator",
                role: RoleRef::Style(StyleRole::FocusIndicator),
                expect: Ansi(Color::Cyan),
            },
            // ── Identity form (the same ANSI shape) ────────────
            Task14RoleUse {
                id: "identity_form.title",
                renderer: KEYCHAIN,
                was: "Block::title(\"Identity\") with no style of its own",
                ident: "PopupTitle",
                role: RoleRef::Style(StyleRole::PopupTitle),
                expect: Unstyled {
                    was: "an unstyled Block title, i.e. the terminal's own foreground",
                    why: "every other overlay titles itself with `popup.title`; leaving \
                          these two unstyled would put an unthemeable title on a themed \
                          popup ground and make the role unreachable from them",
                },
            },
            Task14RoleUse {
                id: "identity_form.label",
                renderer: KEYCHAIN,
                was: "Style::default().fg(Color::DarkGray)",
                ident: "FormLabel",
                role: RoleRef::Style(StyleRole::FormLabel),
                expect: Ansi(Color::DarkGray),
            },
            Task14RoleUse {
                id: "identity_form.label_focused",
                renderer: KEYCHAIN,
                was: "Style::default().fg(Color::Cyan).add_modifier(BOLD)",
                ident: "FormLabelFocused",
                role: RoleRef::Style(StyleRole::FormLabelFocused),
                expect: Ansi(Color::Cyan),
            },
            Task14RoleUse {
                id: "identity_form.label_editing",
                renderer: KEYCHAIN,
                was: "Style::default().fg(Color::Yellow).add_modifier(BOLD)",
                ident: "FormLabelEditing",
                role: RoleRef::Style(StyleRole::FormLabelEditing),
                expect: Ansi(Color::Yellow),
            },
            Task14RoleUse {
                id: "identity_form.value",
                renderer: KEYCHAIN,
                was: "Style::default()",
                ident: "FormValue",
                role: RoleRef::Style(StyleRole::FormValue),
                expect: Unstyled {
                    was: "Style::default(), i.e. the terminal's own foreground",
                    why: "an idle value that carries no colour cannot be themed at all; \
                          `form.value` gives it the body-text role the rest of the app \
                          already uses for exactly this kind of text",
                },
            },
            Task14RoleUse {
                id: "identity_form.value_focused",
                renderer: KEYCHAIN,
                was: "Style::default().fg(Color::White).add_modifier(BOLD)",
                ident: "FormInputFocused",
                role: RoleRef::Style(StyleRole::FormInputFocused),
                expect: Ansi(Color::White),
            },
            Task14RoleUse {
                id: "identity_form.value_editing",
                renderer: KEYCHAIN,
                was: "Style::default().fg(Color::White).add_modifier(BOLD | UNDERLINED)",
                ident: "FormInputEditing",
                role: RoleRef::Style(StyleRole::FormInputEditing),
                expect: Ansi(Color::White),
            },
            Task14RoleUse {
                id: "identity_form.hint",
                renderer: KEYCHAIN,
                was: "Style::default().add_modifier(Modifier::DIM)",
                ident: "FormHelp",
                role: RoleRef::Style(StyleRole::FormHelp),
                expect: Unstyled {
                    was: "Style::default().add_modifier(Modifier::DIM), no colour",
                    why: "the bare DIM modifier is terminal-dependent and several \
                          emulators ignore it; `form.help` states the same intent as a \
                          colour the theme controls",
                },
            },
            Task14RoleUse {
                id: "identity_form.marker",
                renderer: KEYCHAIN,
                was: "the marker inside the ANSI-coloured label span",
                ident: "FocusIndicator",
                role: RoleRef::Style(StyleRole::FocusIndicator),
                expect: Ansi(Color::Cyan),
            },
            // ── Tag filter ─────────────────────────────────────
            Task14RoleUse {
                id: "tag_filter.title",
                renderer: TAG_FILTER,
                was: "theme::heading()",
                ident: "PopupTitle",
                role: RoleRef::Style(StyleRole::PopupTitle),
                expect: Task14Expect::Style(legacy::heading()),
            },
            Task14RoleUse {
                id: "tag_filter.query",
                renderer: TAG_FILTER,
                was: "theme::bright()",
                ident: "PickerQuery",
                role: RoleRef::Style(StyleRole::PickerQuery),
                expect: Task14Expect::Style(legacy::bright()),
            },
            Task14RoleUse {
                id: "tag_filter.row",
                renderer: TAG_FILTER,
                was: "theme::text()",
                ident: "PickerRow",
                role: RoleRef::Style(StyleRole::PickerRow),
                expect: Task14Expect::Style(legacy::text()),
            },
            Task14RoleUse {
                id: "tag_filter.row_selected",
                renderer: TAG_FILTER,
                was: "theme::selected()",
                ident: "PickerRowSelected",
                role: RoleRef::Style(StyleRole::PickerRowSelected),
                expect: Task14Expect::Style(legacy::selected()),
            },
            Task14RoleUse {
                id: "tag_filter.marker",
                renderer: TAG_FILTER,
                was: "the marker inside the selected row's own theme::selected() label",
                ident: "PickerMarker",
                role: RoleRef::Style(StyleRole::PickerMarker),
                expect: Task14Expect::Style(legacy::selected()),
            },
            Task14RoleUse {
                id: "tag_filter.legend",
                renderer: TAG_FILTER,
                was: "theme::mute() on the hint and the empty note",
                ident: "PopupLegend",
                role: RoleRef::Style(StyleRole::PopupLegend),
                expect: Task14Expect::Style(legacy::mute()),
            },
            // ── Session picker ─────────────────────────────────
            Task14RoleUse {
                id: "session_picker.border",
                renderer: SESSION_PICKER,
                was: "Style::default().fg(theme::ACCENT)",
                ident: "PickerBorder",
                role: RoleRef::Paint(PaintRole::PickerBorder),
                expect: Paint(legacy::ACCENT),
            },
            Task14RoleUse {
                id: "session_picker.title",
                renderer: SESSION_PICKER,
                was: "theme::heading()",
                ident: "PopupTitle",
                role: RoleRef::Style(StyleRole::PopupTitle),
                expect: Task14Expect::Style(legacy::heading()),
            },
            Task14RoleUse {
                id: "session_picker.query",
                renderer: SESSION_PICKER,
                was: "theme::bright()",
                ident: "PickerQuery",
                role: RoleRef::Style(StyleRole::PickerQuery),
                expect: Task14Expect::Style(legacy::bright()),
            },
            Task14RoleUse {
                id: "session_picker.rule",
                renderer: SESSION_PICKER,
                was: "theme::dim() on the separator row",
                ident: "PopupHint",
                role: RoleRef::Style(StyleRole::PopupHint),
                expect: Task14Expect::Style(legacy::dim()),
            },
            Task14RoleUse {
                id: "session_picker.row",
                renderer: SESSION_PICKER,
                was: "theme::text()",
                ident: "PickerRow",
                role: RoleRef::Style(StyleRole::PickerRow),
                expect: Task14Expect::Style(legacy::text()),
            },
            Task14RoleUse {
                id: "session_picker.row_selected",
                renderer: SESSION_PICKER,
                was: "theme::selected()",
                ident: "PickerRowSelected",
                role: RoleRef::Style(StyleRole::PickerRowSelected),
                expect: Task14Expect::Style(legacy::selected()),
            },
            Task14RoleUse {
                id: "session_picker.legend",
                renderer: SESSION_PICKER,
                was: "theme::mute() on the empty state, the hint and `current`",
                ident: "PopupLegend",
                role: RoleRef::Style(StyleRole::PopupLegend),
                expect: Task14Expect::Style(legacy::mute()),
            },
            Task14RoleUse {
                id: "session_picker.badge_up",
                renderer: SESSION_PICKER,
                was: "theme::green()",
                ident: "PickerBadgeSuccess",
                role: RoleRef::Color(ColorRole::PickerBadgeSuccess),
                expect: C(legacy::GREEN),
            },
            Task14RoleUse {
                id: "session_picker.badge_connecting",
                renderer: SESSION_PICKER,
                was: "theme::amber()",
                ident: "PickerBadgeWarning",
                role: RoleRef::Color(ColorRole::PickerBadgeWarning),
                expect: C(legacy::AMBER),
            },
            Task14RoleUse {
                id: "session_picker.badge_exited",
                renderer: SESSION_PICKER,
                was: "theme::red()",
                ident: "PickerBadgeError",
                role: RoleRef::Color(ColorRole::PickerBadgeError),
                expect: C(legacy::RED),
            },
            // ── Settings ───────────────────────────────────────
            Task14RoleUse {
                id: "settings.title",
                renderer: SETTINGS,
                was: "theme::heading()",
                ident: "PopupTitle",
                role: RoleRef::Style(StyleRole::PopupTitle),
                expect: Task14Expect::Style(legacy::heading()),
            },
            Task14RoleUse {
                id: "settings.row",
                renderer: SETTINGS,
                was: "theme::text() on an unselected label",
                ident: "TableRow",
                role: RoleRef::Style(StyleRole::TableRow),
                expect: Task14Expect::Style(legacy::text()),
            },
            Task14RoleUse {
                id: "settings.row_selected",
                renderer: SETTINGS,
                was: "theme::white().bg(theme::SEL_BG)",
                ident: "SettingsRowSelected",
                role: RoleRef::Style(StyleRole::SettingsRowSelected),
                expect: Task14Expect::Style(legacy::white().bg(sel_bg)),
            },
            Task14RoleUse {
                id: "settings.theme_value",
                renderer: SETTINGS,
                was: "Style::default().fg(theme::ACCENT)",
                ident: "PickerMatch",
                role: RoleRef::Style(StyleRole::PickerMatch),
                expect: Task14Expect::Style(Style::default().fg(legacy::ACCENT)),
            },
            Task14RoleUse {
                id: "settings.checkbox_on",
                renderer: SETTINGS,
                was: "theme::green() on a ticked box",
                ident: "StatusSuccess",
                role: RoleRef::Color(ColorRole::StatusSuccess),
                expect: C(legacy::GREEN),
            },
            Task14RoleUse {
                id: "settings.legend",
                renderer: SETTINGS,
                was: "theme::mute() on the unticked box and the key legend",
                ident: "PopupLegend",
                role: RoleRef::Style(StyleRole::PopupLegend),
                expect: Task14Expect::Style(legacy::mute()),
            },
            Task14RoleUse {
                id: "settings.hint",
                renderer: SETTINGS,
                was: "theme::dim() on the per-row hint",
                ident: "PopupHint",
                role: RoleRef::Style(StyleRole::PopupHint),
                expect: Task14Expect::Style(legacy::dim()),
            },
            // ── Keybind editor ─────────────────────────────────
            Task14RoleUse {
                id: "keybind.title",
                renderer: KEYBIND,
                was: "theme::heading()",
                ident: "PopupTitle",
                role: RoleRef::Style(StyleRole::PopupTitle),
                expect: Task14Expect::Style(legacy::heading()),
            },
            Task14RoleUse {
                id: "keybind.row",
                renderer: KEYBIND,
                was: "theme::text()",
                ident: "KeybindRow",
                role: RoleRef::Style(StyleRole::KeybindRow),
                expect: Task14Expect::Style(legacy::text()),
            },
            Task14RoleUse {
                id: "keybind.row_selected",
                renderer: KEYBIND,
                was: "theme::white().bg(theme::SEL_BG)",
                ident: "KeybindRowSelected",
                role: RoleRef::Style(StyleRole::KeybindRowSelected),
                expect: Task14Expect::Style(legacy::white().bg(sel_bg)),
            },
            Task14RoleUse {
                id: "keybind.marker",
                renderer: KEYBIND,
                was: "the marker inside the selected white-on-SEL_BG label",
                ident: "KeybindMarker",
                role: RoleRef::Style(StyleRole::KeybindMarker),
                expect: Task14Expect::Style(legacy::white().bg(sel_bg)),
            },
            Task14RoleUse {
                id: "keybind.value",
                renderer: KEYBIND,
                was: "theme::mute()",
                ident: "KeybindValue",
                role: RoleRef::Style(StyleRole::KeybindValue),
                expect: Task14Expect::Style(legacy::mute()),
            },
            Task14RoleUse {
                id: "keybind.value_bound",
                renderer: KEYBIND,
                was: "theme::green().bg(theme::SEL_BG); the bar is now painted separately",
                ident: "KeybindValueBound",
                role: RoleRef::Style(StyleRole::KeybindValueBound),
                expect: Task14Expect::Style(legacy::green()),
            },
            Task14RoleUse {
                id: "keybind.value_capturing",
                renderer: KEYBIND,
                was: "theme::amber().bg(theme::SEL_BG); the bar is now painted separately",
                ident: "KeybindValueCapturing",
                role: RoleRef::Style(StyleRole::KeybindValueCapturing),
                expect: Task14Expect::Style(legacy::amber()),
            },
            Task14RoleUse {
                id: "keybind.hint",
                renderer: KEYBIND,
                was: "theme::dim()",
                ident: "PopupHint",
                role: RoleRef::Style(StyleRole::PopupHint),
                expect: Task14Expect::Style(legacy::dim()),
            },
            // ── Tunnel reconnect ───────────────────────────────
            Task14RoleUse {
                id: "tunnel_reconnect.title",
                renderer: TUNNEL_RECONNECT,
                was: "theme::heading()",
                ident: "PopupTitle",
                role: RoleRef::Style(StyleRole::PopupTitle),
                expect: Task14Expect::Style(legacy::heading()),
            },
            Task14RoleUse {
                id: "tunnel_reconnect.row",
                renderer: TUNNEL_RECONNECT,
                was: "theme::text() on an unselected label",
                ident: "TableRow",
                role: RoleRef::Style(StyleRole::TableRow),
                expect: Task14Expect::Style(legacy::text()),
            },
            Task14RoleUse {
                id: "tunnel_reconnect.row_selected",
                renderer: TUNNEL_RECONNECT,
                was: "theme::white().bg(theme::SEL_BG)",
                ident: "SettingsRowSelected",
                role: RoleRef::Style(StyleRole::SettingsRowSelected),
                expect: Task14Expect::Style(legacy::white().bg(sel_bg)),
            },
            Task14RoleUse {
                id: "tunnel_reconnect.marker",
                renderer: TUNNEL_RECONNECT,
                was: "the marker inside the selected white-on-SEL_BG label",
                ident: "SettingsMarker",
                role: RoleRef::Style(StyleRole::SettingsMarker),
                expect: Task14Expect::Style(legacy::white().bg(sel_bg)),
            },
            Task14RoleUse {
                id: "tunnel_reconnect.value_selected",
                renderer: TUNNEL_RECONNECT,
                was: "theme::green().bg(theme::SEL_BG); the bar is now painted separately",
                ident: "StatusSuccess",
                role: RoleRef::Color(ColorRole::StatusSuccess),
                expect: C(legacy::GREEN),
            },
            Task14RoleUse {
                id: "tunnel_reconnect.legend",
                renderer: TUNNEL_RECONNECT,
                was: "theme::mute() on an unselected value and the key legend",
                ident: "PopupLegend",
                role: RoleRef::Style(StyleRole::PopupLegend),
                expect: Task14Expect::Style(legacy::mute()),
            },
            Task14RoleUse {
                id: "tunnel_reconnect.hint",
                renderer: TUNNEL_RECONNECT,
                was: "theme::dim() on the header line and the per-row hint",
                ident: "PopupHint",
                role: RoleRef::Style(StyleRole::PopupHint),
                expect: Task14Expect::Style(legacy::dim()),
            },
            // ── Help sheet ─────────────────────────────────────
            Task14RoleUse {
                id: "help.section",
                renderer: HELP,
                was: "theme::heading()",
                ident: "HelpSection",
                role: RoleRef::Style(StyleRole::HelpSection),
                expect: Task14Expect::Style(legacy::heading()),
            },
            Task14RoleUse {
                id: "help.key",
                renderer: HELP,
                was: "theme::bright()",
                ident: "HelpKey",
                role: RoleRef::Style(StyleRole::HelpKey),
                expect: Task14Expect::Style(legacy::bright()),
            },
            Task14RoleUse {
                id: "help.description",
                renderer: HELP,
                was: "theme::text()",
                ident: "HelpDescription",
                role: RoleRef::Style(StyleRole::HelpDescription),
                expect: Task14Expect::Style(legacy::text()),
            },
            // ── Identity cards ─────────────────────────────────
            Task14RoleUse {
                id: "identities.empty",
                renderer: KEYS,
                was: "theme::dim() on the empty state and the missing-agent note",
                ident: "IdentitiesEmpty",
                role: RoleRef::Style(StyleRole::IdentitiesEmpty),
                expect: Task14Expect::Style(legacy::dim()),
            },
            Task14RoleUse {
                id: "identities.notice",
                renderer: KEYS,
                was: "theme::amber()",
                ident: "IdentitiesNotice",
                role: RoleRef::Style(StyleRole::IdentitiesNotice),
                expect: Task14Expect::Style(legacy::amber()),
            },
            Task14RoleUse {
                id: "identities.card_border",
                renderer: KEYS,
                was: "theme::border()",
                ident: "IdentitiesCardBorder",
                role: RoleRef::Paint(PaintRole::IdentitiesCardBorder),
                expect: Paint(legacy::BORDER),
            },
            Task14RoleUse {
                id: "identities.card_border_selected",
                renderer: KEYS,
                was: "Style::default().fg(theme::ACCENT)",
                ident: "IdentitiesCardBorderSelected",
                role: RoleRef::Paint(PaintRole::IdentitiesCardBorderSelected),
                expect: Paint(legacy::ACCENT),
            },
            Task14RoleUse {
                id: "identities.card_selection",
                renderer: KEYS,
                was: "theme::selected()",
                ident: "IdentitiesCardSelection",
                role: RoleRef::Style(StyleRole::IdentitiesCardSelection),
                expect: Task14Expect::Style(legacy::selected()),
            },
            Task14RoleUse {
                id: "identities.card_name",
                renderer: KEYS,
                was: "theme::heading()",
                ident: "IdentitiesCardName",
                role: RoleRef::Style(StyleRole::IdentitiesCardName),
                expect: Task14Expect::Style(legacy::heading()),
            },
            Task14RoleUse {
                id: "identities.card_text",
                renderer: KEYS,
                was: "theme::text()",
                ident: "IdentitiesCardText",
                role: RoleRef::Style(StyleRole::IdentitiesCardText),
                expect: Task14Expect::Style(legacy::text()),
            },
            Task14RoleUse {
                id: "identities.card_metadata",
                renderer: KEYS,
                was: "theme::dim() on the fingerprint and the key path",
                ident: "IdentitiesCardMetadata",
                role: RoleRef::Style(StyleRole::IdentitiesCardMetadata),
                expect: Task14Expect::Style(legacy::dim()),
            },
            Task14RoleUse {
                id: "identities.card_key_type",
                renderer: KEYS,
                was: "theme::mute()",
                ident: "IdentitiesCardKeyType",
                role: RoleRef::Style(StyleRole::IdentitiesCardKeyType),
                expect: Task14Expect::Style(legacy::mute()),
            },
            Task14RoleUse {
                id: "identities.card_loaded",
                renderer: KEYS,
                was: "theme::GREEN",
                ident: "IdentitiesCardLoaded",
                role: RoleRef::Color(ColorRole::IdentitiesCardLoaded),
                expect: C(legacy::GREEN),
            },
            Task14RoleUse {
                id: "identities.card_missing",
                renderer: KEYS,
                was: "theme::DIM",
                ident: "IdentitiesCardMissing",
                role: RoleRef::Color(ColorRole::IdentitiesCardMissing),
                expect: C(legacy::DIM),
            },
            Task14RoleUse {
                id: "identities.card_credential",
                renderer: KEYS,
                was: "theme::AMBER",
                ident: "IdentitiesCardCredential",
                role: RoleRef::Color(ColorRole::IdentitiesCardCredential),
                expect: C(legacy::AMBER),
            },
            Task14RoleUse {
                id: "identities.agent_separator",
                renderer: KEYS,
                was: "theme::dim() on the rule above the agent block",
                ident: "IdentitiesAgentSeparator",
                role: RoleRef::Paint(PaintRole::IdentitiesAgentSeparator),
                expect: Paint(legacy::DIM),
            },
            Task14RoleUse {
                id: "identities.agent_label",
                renderer: KEYS,
                was: "theme::mute()",
                ident: "IdentitiesAgentLabel",
                role: RoleRef::Style(StyleRole::IdentitiesAgentLabel),
                expect: Task14Expect::Style(legacy::mute()),
            },
            Task14RoleUse {
                id: "identities.agent_value",
                renderer: KEYS,
                was: "theme::text()",
                ident: "IdentitiesAgentValue",
                role: RoleRef::Style(StyleRole::IdentitiesAgentValue),
                expect: Task14Expect::Style(legacy::text()),
            },
            Task14RoleUse {
                id: "identities.agent_count",
                renderer: KEYS,
                was: "theme::bright()",
                ident: "IdentitiesAgentCount",
                role: RoleRef::Style(StyleRole::IdentitiesAgentCount),
                expect: Task14Expect::Style(legacy::bright()),
            },
        ]
    }

    /// The path a role is published under.
    fn role_path(role: RoleRef) -> &'static str {
        ROLE_SPECS
            .iter()
            .find(|spec| spec.role == role)
            .map(|spec| spec.path)
            .unwrap_or("<unpublished>")
    }

    /// Compare every inventoried cell that follows its role against the
    /// hand-written legacy value, not against the role's own fallback.
    fn assert_task14_legacy_cells(theme: &ResolvedTheme) {
        for cell in task14_role_uses() {
            let path = role_path(cell.role);
            let context = format!(
                "{} ({}) via {path}, was {}",
                cell.id, cell.renderer, cell.was
            );
            match (cell.expect, cell.role) {
                (Task14Expect::Style(expected), RoleRef::Style(role)) => {
                    assert_eq!(theme.style(role), expected, "{context}")
                }
                (Task14Expect::Color(expected), RoleRef::Color(role)) => {
                    assert_eq!(theme.color(role), expected, "{context}")
                }
                (Task14Expect::Paint(expected), RoleRef::Paint(role)) => {
                    assert_eq!(
                        *theme.paint(role),
                        ResolvedPaint::Solid(expected),
                        "{context}"
                    )
                }
                (Task14Expect::Ansi(_), _)
                | (Task14Expect::Unstyled { .. }, _)
                | (Task14Expect::Context { .. }, _) => {}
                _ => panic!("{context}: the expectation is of a different kind than the role"),
            }
        }
    }

    /// The productive part of a renderer: its `#[cfg(test)]` child does not
    /// draw anything and must not be mistaken for a call site.
    fn renderer_source(path: &str) -> String {
        let full = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
        let text = std::fs::read_to_string(&full)
            .unwrap_or_else(|e| panic!("{path} is inventoried but cannot be read: {e}"));
        match text.find("\n#[cfg(test)]\nmod tests {") {
            Some(cut) => text[..cut].to_string(),
            None => text,
        }
    }

    /// The body of `fn <name>` in `src/tui/mod.rs`, or `None` if there is no
    /// such test. Used to check that a cited proof really exercises the branch
    /// it is cited for.
    fn test_body(qualified: &str) -> Option<String> {
        let name = qualified.rsplit("::").next()?;
        let source =
            std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(MOD))
                .ok()?;
        let at = source.find(&format!("fn {name}("))?;
        let open = source[at..].find('{')? + at;
        let mut depth = 0usize;
        for (offset, ch) in source[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(source[open..open + offset].to_string());
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The `<Kind>Role::<Ident>` references a renderer really makes.
    fn roles_read_by(path: &str) -> Vec<String> {
        let source = renderer_source(path);
        let mut found = Vec::new();
        for kind in ["StyleRole::", "PaintRole::", "ColorRole::"] {
            let mut rest = source.as_str();
            while let Some(at) = rest.find(kind) {
                rest = &rest[at + kind.len()..];
                let end = rest
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .unwrap_or(rest.len());
                let ident = &rest[..end];
                if !ident.is_empty() && !found.iter().any(|f| f == ident) {
                    found.push(ident.to_string());
                }
            }
        }
        found
    }

    /// The inventory must match the roles the renderers actually read — both
    /// ways.
    ///
    /// The forward direction catches an invented row; the reverse direction is
    /// the one that matters, because it derives the required set from the
    /// renderers' own source. A role that nobody remembered to inventory fails
    /// here naming its file, which is how the palette's opaque background was
    /// found missing.
    ///
    /// The claim is deliberately per *renderer and role*, not per cell — see
    /// the module comment above for what that does and does not cover.
    #[test]
    fn the_inventory_covers_every_role_each_task14_renderer_reads() {
        let cells = task14_role_uses();

        let mut ids: Vec<&str> = cells.iter().map(|c| c.id).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(total, ids.len(), "two rows share an id");
        // Pinned so the number quoted in reports cannot drift from the table.
        assert_eq!(
            total, 115,
            "the inventory changed size; add or remove the row deliberately"
        );

        // Forward: every inventoried cell names a role its renderer really
        // reads, and a role the catalogue really publishes.
        for cell in &cells {
            assert!(
                ROLE_SPECS.iter().any(|spec| spec.role == cell.role),
                "{}: {} is inventoried but no longer published",
                cell.id,
                cell.ident
            );
            assert!(
                roles_read_by(cell.renderer).iter().any(|r| r == cell.ident),
                "{}: {} does not read {} any more",
                cell.id,
                cell.renderer,
                cell.ident
            );
            // The provenance is part of the contract, not a comment: an empty
            // one would make a row unreviewable.
            assert!(
                !cell.was.is_empty(),
                "{}: every cell must record what it was before the migration",
                cell.id
            );
            match cell.expect {
                Task14Expect::Context { why, proof } => {
                    assert!(
                        !why.is_empty(),
                        "{}: a context exception must say why the cells leave their role",
                        cell.id
                    );
                    // The proof must exist, and it must exercise the branch the
                    // `why` describes: a test that gives the role a marker
                    // value never reaches the `default` substitution at all.
                    let body = test_body(proof).unwrap_or_else(|| {
                        panic!("{}: no test named {proof} in src/tui/mod.rs", cell.id)
                    });
                    assert!(
                        !body.contains(role_path(cell.role)),
                        "{}: {proof} overrides {}, so it never runs the `default` \
                         substitution it is cited for",
                        cell.id,
                        role_path(cell.role)
                    );
                }
                // The spec's parity exception is for direct ANSI *colours*. A
                // cell that merely had no colour is a different thing and must
                // use `Unstyled`, which forces a recorded reason.
                Task14Expect::Ansi(was) => assert!(
                    !matches!(was, Color::Rgb(..) | Color::Indexed(_) | Color::Reset),
                    "{}: {was:?} is not a direct ANSI colour, so the spec's \
                     normalisation exception does not cover it",
                    cell.id
                ),
                Task14Expect::Unstyled { was, why } => {
                    assert!(
                        !was.is_empty(),
                        "{}: record what the uncoloured cell actually was",
                        cell.id
                    );
                    assert!(
                        !why.is_empty(),
                        "{}: a cell that had no colour and now has one is a \
                         deliberate deviation and needs its reasoning here and \
                         in the spec",
                        cell.id
                    );
                }
                _ => {}
            }
        }

        // Reverse: every role a Task 14 renderer reads is inventoried for that
        // renderer, or explicitly declared as an earlier task's.
        for renderer in TASK14_RENDERERS {
            for ident in roles_read_by(renderer) {
                if NOT_TASK14
                    .iter()
                    .any(|(file, role)| file == renderer && *role == ident)
                {
                    continue;
                }
                assert!(
                    cells
                        .iter()
                        .any(|c| c.renderer == *renderer && c.ident == ident),
                    "{renderer} draws from {ident}, which no inventory row covers \
                     — add it to `task14_role_uses` with the `theme.rs` calls it \
                     replaced, or to `NOT_TASK14` if it belongs to another task"
                );
            }
        }
    }
}
