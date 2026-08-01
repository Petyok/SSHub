//! The public surface of a `ResolvedTheme`, exercised from *outside* the crate.
//!
//! The in-crate accessor test cannot prove this: `src/theme/`'s fields are
//! `pub(crate)`, so a unit test still reaches them even if a getter were
//! deleted or a constructor accidentally re-widened. This file compiles as a
//! real consumer does, which is the only way to notice.

use ratatui::layout::Rect;
use ratatui::style::Color;

use sshub::theme::catalog::{
    ColorRole, PaintRole, RoleFallback, RoleRef, StyleRole, TintRole, ROLE_SPECS, SEMANTIC_SPECS,
};
use sshub::theme::model::{ResolvedPaint, ResolvedTint, ThemeId, ValidationMode};
use sshub::theme::registry::{ThemeRegistry, ThemeSource};

#[test]
fn a_resolved_theme_is_fully_readable_through_the_public_api() {
    let registry = ThemeRegistry::builtins(ValidationMode::Strict).expect("built-ins load");
    let theme = registry
        .resolved(&ThemeId::parse("aqua").expect("aqua is a valid id"))
        .expect("aqua resolves");

    // Identity and metadata.
    assert_eq!(theme.id().as_str(), "aqua");
    assert!(!theme.name().is_empty());
    assert!(theme.description().is_some_and(|d| !d.is_empty()));

    // Every semantic slot, by its typed slot rather than by field access.
    for spec in SEMANTIC_SPECS {
        let color = theme.semantic().slot(spec.slot);
        assert_ne!(color, Color::Reset, "aqua paints semantic.{}", spec.key);
    }

    // The whole gradient table, plus the name every gradient was given.
    assert!(!theme.gradients().is_empty());
    for (index, gradient) in theme.gradients().iter().enumerate() {
        let _ = gradient.direction();
        assert!(
            !gradient.stops().is_empty(),
            "gradient {index} has no stops"
        );
        for stop in gradient.stops() {
            assert!((0.0..=1.0).contains(&stop.position()));
            assert_ne!(stop.color(), Color::Reset);
        }
    }

    // Every component role, through the four typed accessors.
    let area = Rect::new(0, 0, 20, 10);
    for spec in ROLE_SPECS {
        match (spec.role, spec.fallback) {
            (RoleRef::Color(role), RoleFallback::Color(_)) => {
                let _: Color = theme.color(role);
            }
            (RoleRef::Style(role), RoleFallback::Style(_)) => {
                let _ = theme.style(role);
            }
            (RoleRef::Paint(role), RoleFallback::Paint(_)) => match theme.paint(role) {
                ResolvedPaint::Solid(_) => {}
                ResolvedPaint::Gradient(id) => {
                    // A gradient role must be resolvable to a table entry *and*
                    // back to the name its author wrote — that is what makes
                    // `theme show --resolved` able to reference it again.
                    assert!(theme.gradient(*id).is_some(), "{}", spec.path);
                    let name = theme
                        .gradient_name(*id)
                        .unwrap_or_else(|| panic!("{} names its gradient", spec.path));
                    assert!(!name.is_empty());
                    let _ = theme.paint_gradient(role).expect("a gradient role");
                    let _ = theme.paint_color_at(role, area, 3, 4);
                }
            },
            (RoleRef::Tint(role), RoleFallback::Tint(_)) => match theme.tint(role) {
                ResolvedTint::Native | ResolvedTint::Color(_) => {}
            },
            _ => panic!("{} pairs a fallback of a different kind", spec.path),
        }
    }

    // The named-role accessors are reachable too, not just the catalogue walk.
    let _ = theme.color(ColorRole::TunnelRunning);
    let _ = theme.style(StyleRole::FooterKey);
    let _ = theme.paint(PaintRole::PopupBorder);
    let _ = theme.tint(TintRole::OsLogoTint);
}

#[test]
fn registry_records_are_readable_from_outside_the_crate() {
    let registry = ThemeRegistry::builtins(ValidationMode::Strict).expect("built-ins load");
    for record in registry.records() {
        assert_eq!(record.source, ThemeSource::BuiltIn);
        assert!(record.is_valid(), "{} is invalid", record.id);
        assert!(!record.toml_source.is_empty());
        assert!(record.resolved().is_some());
        assert!(record.diagnostics.is_empty(), "{:#?}", record.diagnostics);
    }
    assert!(registry.diagnostics().is_empty());
    assert!(registry.get("aqua").is_some());
    assert!(registry.get("no-such-theme").is_none());
}

/// A `GradientId` belongs to the *resolve run* that minted it, not merely to a
/// theme id.
///
/// The ids used to be a bare table index, so an id captured before a theme
/// reload still indexed happily into the new table — silently naming a
/// different gradient, or the same one after the author had edited it. Two
/// independent resolve runs of the *same* built-in are the sharpest case:
/// identical id, identical content, identical table position, and still the
/// foreign id has to be refused.
#[test]
fn a_gradient_id_is_rejected_by_a_theme_from_another_resolve_run() {
    let load = || {
        ThemeRegistry::builtins(ValidationMode::Strict)
            .expect("built-ins load")
            .resolved(&ThemeId::parse("aqua").expect("aqua is a valid id"))
            .expect("aqua resolves")
    };
    let a = load();
    let b = load();

    // Same theme, same content — only the resolve run differs.
    assert_eq!(a.id(), b.id());
    assert_eq!(a.gradients(), b.gradients());

    let id = ROLE_SPECS
        .iter()
        .find_map(|spec| match spec.role {
            RoleRef::Paint(role) => match a.paint(role) {
                ResolvedPaint::Gradient(id) => Some(*id),
                ResolvedPaint::Solid(_) => None,
            },
            _ => None,
        })
        .expect("aqua paints at least one role with a gradient");

    // Its own run answers.
    assert!(
        a.gradient(id).is_some(),
        "the minting theme must resolve it"
    );
    assert!(a.gradient_name(id).is_some());

    // The other run refuses, index in range or not.
    assert!(
        b.gradient(id).is_none(),
        "a foreign resolve run answered a gradient id"
    );
    assert!(
        b.gradient_name(id).is_none(),
        "a foreign resolve run named a gradient id"
    );

    // A clone stays the same run, so it must keep answering.
    let cloned = a.clone();
    assert!(cloned.gradient(id).is_some(), "a clone changed generation");
}
