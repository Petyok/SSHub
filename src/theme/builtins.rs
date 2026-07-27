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

        // Two different whites on the same selection background: the dashboard
        // and command palette highlight with WHITE, the pickers with SEL_FG.
        assert_ne!(legacy::WHITE, legacy::SEL_FG);
        assert_eq!(
            theme.style(StyleRole::DashboardHostListHostSelected),
            Style::default().fg(legacy::WHITE).bg(legacy::SEL_BG)
        );
        assert_eq!(
            theme.style(StyleRole::CommandPaletteRowSelected),
            Style::default().fg(legacy::WHITE).bg(legacy::SEL_BG)
        );
        assert_eq!(
            theme.style(StyleRole::PickerRowSelected),
            legacy::selected()
        );
        assert_eq!(
            theme.style(StyleRole::KeychainRowSelected),
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

        // The tunnels tab keeps SEL_FG for its selected row, unlike the tables
        // that highlight with WHITE.
        assert_eq!(
            theme.style(StyleRole::TunnelsRowSelected),
            legacy::selected()
        );
        assert_ne!(
            theme.style(StyleRole::TunnelsRowSelected),
            theme.style(StyleRole::TableRowSelected)
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
    }
}
