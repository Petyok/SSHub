//! Turn validated theme definitions into one immutable [`ResolvedTheme`].
//!
//! The order of the pipeline is load-bearing and must not be rearranged:
//!
//! 1. walk `extends` to build the inheritance chain (implicit `default` parent
//!    for every theme but `default` itself);
//! 2. **deep-merge the whole chain**, root first, applying `"auto"` resets as
//!    they are encountered;
//! 3. only then resolve `palette.*` / `semantic.*` references, with cycle and
//!    depth detection;
//! 4. apply brightness, then simulated opacity, per channel in sRGB;
//! 5. resolve gradients and check their endpoints;
//! 6. fill every typed component slot from its override or its `ROLE_SPECS`
//!    fallback, so the result has no gaps and no `Option`.
//!
//! Merging before resolving is what makes a child's `semantic.accent` re-tint
//! every component reference it inherited (spec, "Theme-Vererbung").
//!
//! Every problem is reported as a diagnostic and resolution keeps going with a
//! local fallback, so one run lists everything wrong with a theme; a theme with
//! any error diagnostic is never handed out, because activation is atomic.

use std::collections::BTreeMap;
use std::ops::Range;

use ratatui::style::{Color, Modifier, Style};

use crate::theme::catalog::{RoleFallback, RoleRef, ROLE_SPECS, SEMANTIC_SPECS};
use crate::theme::model::{
    modifier_from_key, semantic_style, ColorBase, ColorSlot, ColorValue, ComponentValue,
    GradientDefinition, GradientDirection, GradientId, ModifierList, PaintSlot, ResolvedComponents,
    ResolvedGradient, ResolvedGradientStop, ResolvedPaint, ResolvedSemantic, ResolvedTheme,
    ResolvedTint, Spanned, ThemeDefinition, ThemeDiagnostic, ThemeId, ThemeOrigin, TintSlot,
    SEMANTIC_SLOT_COUNT,
};

/// V1 limit on how many themes one `extends` chain may contain.
pub const MAX_INHERITANCE_DEPTH: usize = 16;

/// V1 limit on how many palette/semantic entries one colour may be resolved
/// through before the chain is considered runaway.
pub const MAX_COLOR_REFERENCE_DEPTH: usize = 16;

/// Everything one resolution run produced.
pub struct ResolveOutcome {
    /// `None` when any diagnostic is an error — a partially resolved theme is
    /// never activated.
    pub theme: Option<ResolvedTheme>,
    pub diagnostics: Vec<ThemeDiagnostic>,
    /// The chain that was merged, child first, root last. Empty when the chain
    /// itself could not be built.
    pub inheritance_chain: Vec<ThemeId>,
}

/// Resolve `id` against the definitions it may inherit from.
///
/// `definitions` must already be parsed and validated per file; the resolver
/// re-checks nothing a validated definition guarantees.
pub fn resolve_theme(
    id: &ThemeId,
    definitions: &BTreeMap<ThemeId, &ThemeDefinition>,
) -> ResolveOutcome {
    Resolver::new(definitions).resolve(id)
}

/// A value together with the theme and byte range it came from.
///
/// Provenance is not decoration: a diagnostic about an inherited value has to
/// name the file it is written in *and* the theme that supplied it, because the
/// two are usually different files.
#[derive(Clone)]
struct Sourced<T> {
    value: T,
    theme: ThemeId,
    origin: ThemeOrigin,
    span: Range<usize>,
}

/// The theme a merge step is currently copying values out of.
struct Provenance {
    theme: ThemeId,
    origin: ThemeOrigin,
}

impl Provenance {
    fn at<T>(&self, value: T, span: Range<usize>) -> Sourced<T> {
        Sourced {
            value,
            theme: self.theme.clone(),
            origin: self.origin.clone(),
            span,
        }
    }
}

impl<T> Sourced<T> {
    fn site(&self, path: String) -> Site {
        Site {
            path,
            theme: self.theme.clone(),
            origin: self.origin.clone(),
            span: self.span.clone(),
        }
    }
}

/// Where a value being resolved is written.
#[derive(Clone)]
struct Site {
    /// Dotted path used in messages, e.g. `components.footer.key.background`.
    path: String,
    theme: ThemeId,
    origin: ThemeOrigin,
    span: Range<usize>,
}

/// A paint override after the merge: still unresolved, but no longer `"auto"`.
#[derive(Clone)]
enum PaintOverride {
    Color(ColorValue),
    /// Bare gradient name, without the `gradients.` prefix.
    Gradient(String),
}

#[derive(Clone)]
enum TintOverride {
    Native,
    Color(ColorValue),
}

/// The three independently mergeable fields of a `Style` role.
#[derive(Clone, Default)]
struct MergedStyle {
    foreground: Option<Sourced<ColorValue>>,
    background: Option<Sourced<ColorValue>>,
    modifiers: Option<Sourced<Vec<Spanned<String>>>>,
}

/// The whole inheritance chain collapsed into one definition.
struct Merged {
    name: String,
    description: Option<String>,
    palette: BTreeMap<String, Sourced<ColorValue>>,
    semantic: Vec<Option<Sourced<ColorValue>>>,
    gradients: BTreeMap<String, Sourced<GradientDefinition>>,
    colors: Vec<Option<Sourced<ColorValue>>>,
    styles: Vec<MergedStyle>,
    paints: Vec<Option<Sourced<PaintOverride>>>,
    tints: Vec<Option<Sourced<TintOverride>>>,
}

/// A resolved colour plus, when it is `Color::Reset`, the theme whose file
/// actually holds the `"terminal"` literal it came from.
#[derive(Clone)]
struct ColorOutcome {
    color: Color,
    terminal_origin: Option<ThemeId>,
}

impl ColorOutcome {
    fn rgb(color: Color) -> Self {
        Self {
            color,
            terminal_origin: None,
        }
    }
}

struct Resolver<'a> {
    definitions: &'a BTreeMap<ThemeId, &'a ThemeDefinition>,
    diagnostics: Vec<ThemeDiagnostic>,
    merged: Merged,
    /// Memoised reference results, keyed by qualified label. Without it one
    /// dangling `semantic.background` would be reported once per component
    /// that mixes over it.
    cache: BTreeMap<String, Option<ColorOutcome>>,
    /// Qualified labels currently being resolved, for cycle detection.
    stack: Vec<String>,
    /// Depth failures are context dependent, so a result computed under one is
    /// not cacheable; counting them is how that is detected.
    depth_errors: usize,
}

impl<'a> Resolver<'a> {
    fn new(definitions: &'a BTreeMap<ThemeId, &'a ThemeDefinition>) -> Self {
        Self {
            definitions,
            diagnostics: Vec::new(),
            merged: Merged {
                name: String::new(),
                description: None,
                palette: BTreeMap::new(),
                semantic: vec![None; SEMANTIC_SLOT_COUNT],
                gradients: BTreeMap::new(),
                colors: vec![None; role_counts().0],
                styles: vec![MergedStyle::default(); role_counts().1],
                paints: vec![None; role_counts().2],
                tints: vec![None; role_counts().3],
            },
            cache: BTreeMap::new(),
            stack: Vec::new(),
            depth_errors: 0,
        }
    }

    fn resolve(mut self, id: &ThemeId) -> ResolveOutcome {
        let Some(chain) = self.build_chain(id) else {
            return ResolveOutcome {
                theme: None,
                diagnostics: self.diagnostics,
                inheritance_chain: Vec::new(),
            };
        };

        // Root first, so a child's assignment lands last and wins.
        for ancestor in chain.iter().rev() {
            let definition = self.definitions[ancestor];
            self.merge(definition);
        }

        let semantic = self.resolve_semantic(id);
        let (gradients, gradient_ids) = self.resolve_gradients();
        let components = self.resolve_components(id, &semantic, &gradient_ids, &gradients);
        self.resolve_unused_palette();

        // Diagnostics are produced in catalogue order, which is not source
        // order; the presentation contract is the same one `validate` promises.
        self.diagnostics
            .sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        // A runaway reference chain is walked once per consumer on purpose (its
        // failure is not cacheable), so the identical message can be emitted
        // several times.
        self.diagnostics.dedup();

        let failed = self.diagnostics.iter().any(ThemeDiagnostic::is_error);
        let theme = (!failed).then(|| ResolvedTheme {
            id: id.clone(),
            name: self.merged.name.clone(),
            description: self.merged.description.clone(),
            semantic,
            gradients,
            components,
        });
        ResolveOutcome {
            theme,
            diagnostics: self.diagnostics,
            inheritance_chain: chain,
        }
    }

    // -- inheritance ------------------------------------------------------

    /// The chain from `id` up to its root, child first.
    fn build_chain(&mut self, id: &ThemeId) -> Option<Vec<ThemeId>> {
        let mut chain: Vec<ThemeId> = Vec::new();
        let mut current = id.clone();
        loop {
            let Some(definition) = self.definitions.get(&current) else {
                self.diagnostics.push(
                    ThemeDiagnostic::error(
                        ThemeOrigin::BuiltIn,
                        None,
                        format!("unknown theme `{current}`"),
                    )
                    .with_help("no built-in or user theme carries that id"),
                );
                return None;
            };
            let origin = definition.origin.clone();
            chain.push(current.clone());
            if chain.len() > MAX_INHERITANCE_DEPTH {
                self.diagnostics.push(ThemeDiagnostic::error(
                    origin,
                    None,
                    format!(
                        "`{id}` inheritance is nested deeper than {MAX_INHERITANCE_DEPTH} themes"
                    ),
                ));
                return None;
            }

            let (parent, span) = match &definition.extends {
                Some(extends) => match ThemeId::parse(&extends.value) {
                    Ok(parent) => (parent, Some(extends.span.clone())),
                    Err(error) => {
                        self.diagnostics.push(ThemeDiagnostic::error(
                            origin,
                            Some(extends.span.clone()),
                            format!("`extends` is not a valid theme id: {error}"),
                        ));
                        return None;
                    }
                },
                // The root theme is the only one without a parent; every other
                // theme implicitly extends `default`.
                None if current.as_str() == ROOT_THEME_ID => break,
                None => (ThemeId::parse(ROOT_THEME_ID).ok()?, None),
            };

            if chain.contains(&parent) {
                self.diagnostics.push(
                    ThemeDiagnostic::error(
                        origin,
                        span,
                        format!("inheritance cycle: {} -> {parent}", join_chain(&chain)),
                    )
                    .with_help("`extends` must reach a root theme without revisiting a theme"),
                );
                return None;
            }
            if !self.definitions.contains_key(&parent) {
                self.diagnostics.push(ThemeDiagnostic::error(
                    origin,
                    span,
                    format!("unknown parent theme `{parent}`"),
                ));
                return None;
            }
            current = parent;
        }
        Some(chain)
    }

    // -- merge ------------------------------------------------------------

    fn merge(&mut self, definition: &ThemeDefinition) {
        let provenance = Provenance {
            theme: definition.id.clone(),
            origin: definition.origin.clone(),
        };

        if !definition.name.value.is_empty() {
            self.merged.name = definition.name.value.clone();
        }
        if let Some(description) = &definition.description {
            self.merged.description = Some(description.value.clone());
        }

        for entry in &definition.palette {
            self.merged.palette.insert(
                entry.name.value.clone(),
                provenance.at(entry.value.value.clone(), entry.value.span.clone()),
            );
        }
        for entry in &definition.semantic {
            self.merged.semantic[entry.slot as usize] =
                Some(provenance.at(entry.value.value.clone(), entry.value.span.clone()));
        }
        for gradient in &definition.gradients {
            self.merged.gradients.insert(
                gradient.name.value.clone(),
                provenance.at(gradient.clone(), gradient.span.clone()),
            );
        }

        for entry in &definition.components {
            match &entry.value {
                ComponentValue::Color { role, value } => {
                    let index = *role as usize;
                    self.merged.colors[index] = match &value.value {
                        ColorSlot::Auto => None,
                        ColorSlot::Color(color) => {
                            Some(provenance.at(color.clone(), value.span.clone()))
                        }
                    };
                }
                ComponentValue::Paint { role, value } => {
                    let index = *role as usize;
                    self.merged.paints[index] = match &value.value {
                        PaintSlot::Auto => None,
                        PaintSlot::Color(color) => Some(
                            provenance.at(PaintOverride::Color(color.clone()), value.span.clone()),
                        ),
                        PaintSlot::Gradient(name) => Some(provenance.at(
                            PaintOverride::Gradient(name.value.clone()),
                            name.span.clone(),
                        )),
                    };
                }
                ComponentValue::Tint { role, value } => {
                    let index = *role as usize;
                    self.merged.tints[index] = match &value.value {
                        TintSlot::Auto => None,
                        TintSlot::Native => {
                            Some(provenance.at(TintOverride::Native, value.span.clone()))
                        }
                        TintSlot::Color(color) => Some(
                            provenance.at(TintOverride::Color(color.clone()), value.span.clone()),
                        ),
                    };
                }
                ComponentValue::Style { role, value } => {
                    let index = *role as usize;
                    let style = &value.value;
                    // `{ auto = true }` clears the inherited role first, so
                    // concrete fields written in the same table still apply on
                    // top of the catalogue fallback rather than being dead.
                    if style.auto.as_ref().is_some_and(|auto| auto.value) {
                        self.merged.styles[index] = MergedStyle::default();
                    }
                    if let Some(foreground) = &style.foreground {
                        self.merged.styles[index].foreground = match &foreground.value {
                            ColorSlot::Auto => None,
                            ColorSlot::Color(color) => {
                                Some(provenance.at(color.clone(), foreground.span.clone()))
                            }
                        };
                    }
                    if let Some(background) = &style.background {
                        self.merged.styles[index].background = match &background.value {
                            ColorSlot::Auto => None,
                            ColorSlot::Color(color) => {
                                Some(provenance.at(color.clone(), background.span.clone()))
                            }
                        };
                    }
                    if let Some(modifiers) = &style.modifiers {
                        self.merged.styles[index].modifiers = match &modifiers.value {
                            ModifierList::Auto => None,
                            ModifierList::List(list) => {
                                Some(provenance.at(list.clone(), modifiers.span.clone()))
                            }
                        };
                    }
                }
                // Unknown roles are the validator's policy call; by the time a
                // definition reaches the resolver they carry no value to apply.
                ComponentValue::Unknown { .. } => {}
            }
        }
    }

    // -- semantic core ----------------------------------------------------

    fn resolve_semantic(&mut self, id: &ThemeId) -> ResolvedSemantic {
        let mut slots = [Color::Reset; SEMANTIC_SLOT_COUNT];
        for spec in SEMANTIC_SPECS {
            let index = spec.slot as usize;
            let label = format!("semantic.{}", spec.key);
            let Some(entry) = self.merged.semantic[index].clone() else {
                self.diagnostics.push(
                    ThemeDiagnostic::error(
                        self.origin_of(id),
                        None,
                        format!("`{label}` is not defined anywhere in the chain of `{id}`"),
                    )
                    .with_help("the semantic core must be complete after inheritance"),
                );
                continue;
            };
            let site = entry.site(label.clone());
            if let Some(outcome) = self.resolve_reference(&label, &site) {
                slots[index] = outcome.color;
            }
        }
        ResolvedSemantic::from_slots(slots)
    }

    /// Resolve every palette entry nothing referenced.
    ///
    /// Palette entries resolve lazily, so without this sweep a typo inside an
    /// entry that happens to be unused would never be reported, while the same
    /// typo in a used entry is fatal. Gradients are resolved eagerly, and the
    /// checker must not have that asymmetry.
    fn resolve_unused_palette(&mut self) {
        let names: Vec<String> = self.merged.palette.keys().cloned().collect();
        for name in names {
            let label = format!("palette.{name}");
            if self.cache.contains_key(&label) {
                continue;
            }
            let entry = self.merged.palette[&name].clone();
            let site = entry.site(label.clone());
            self.resolve_reference(&label, &site);
        }
    }

    fn origin_of(&self, id: &ThemeId) -> ThemeOrigin {
        self.definitions
            .get(id)
            .map(|definition| definition.origin.clone())
            .unwrap_or(ThemeOrigin::BuiltIn)
    }

    // -- colour references ------------------------------------------------

    /// Resolve a qualified `palette.*` / `semantic.*` entry.
    ///
    /// `site` is the *use* site and is only used for the diagnostic about the
    /// reference itself; the referenced value is reported at its own site.
    fn resolve_reference(&mut self, label: &str, site: &Site) -> Option<ColorOutcome> {
        if let Some(cached) = self.cache.get(label) {
            return cached.clone();
        }
        if self.stack.iter().any(|entry| entry == label) {
            self.diagnostics.push(
                ThemeDiagnostic::error(
                    site.origin.clone(),
                    Some(site.span.clone()),
                    format!(
                        "colour reference cycle: {} -> {label}",
                        self.stack.join(" -> ")
                    ),
                )
                .with_help("a colour may not resolve through itself"),
            );
            return None;
        }

        let Some(entry) = self.lookup(label) else {
            self.diagnostics.push(
                ThemeDiagnostic::error(
                    site.origin.clone(),
                    Some(site.span.clone()),
                    format!("unknown colour reference `{label}`"),
                )
                .with_help(format!(
                    "`{}` uses it, but no theme in the inheritance chain defines it",
                    site.path
                )),
            );
            self.cache.insert(label.to_string(), None);
            return None;
        };

        if self.stack.len() >= MAX_COLOR_REFERENCE_DEPTH {
            self.depth_errors += 1;
            self.diagnostics.push(ThemeDiagnostic::error(
                entry.origin.clone(),
                Some(entry.span.clone()),
                format!(
                    "`{label}` colour reference is nested deeper than \
                     {MAX_COLOR_REFERENCE_DEPTH} entries"
                ),
            ));
            return None;
        }

        let before = self.depth_errors;
        self.stack.push(label.to_string());
        let outcome = self.resolve_color(&entry.value, &entry.site(label.to_string()));
        self.stack.pop();
        if self.depth_errors == before {
            self.cache.insert(label.to_string(), outcome.clone());
        }
        outcome
    }

    fn lookup(&self, label: &str) -> Option<Sourced<ColorValue>> {
        if let Some(name) = label.strip_prefix("palette.") {
            return self.merged.palette.get(name).cloned();
        }
        let key = label.strip_prefix("semantic.")?;
        let slot = SEMANTIC_SPECS
            .iter()
            .find(|spec| spec.key == key)
            .map(|spec| spec.slot)?;
        self.merged.semantic[slot as usize].clone()
    }

    /// Resolve one colour value: base, then brightness, then simulated opacity.
    fn resolve_color(&mut self, value: &ColorValue, site: &Site) -> Option<ColorOutcome> {
        let base = match &value.base {
            ColorBase::Terminal => ColorOutcome {
                color: Color::Reset,
                terminal_origin: Some(site.theme.clone()),
            },
            ColorBase::Hex([r, g, b]) => ColorOutcome::rgb(Color::Rgb(*r, *g, *b)),
            ColorBase::Rgb(channels) => {
                let channel = |value: &Spanned<i64>| value.value.clamp(0, 255) as u8;
                ColorOutcome::rgb(Color::Rgb(
                    channel(&channels[0]),
                    channel(&channels[1]),
                    channel(&channels[2]),
                ))
            }
            ColorBase::Reference(reference) => {
                let label = format!("{}.{}", reference.scope.prefix(), reference.name);
                self.resolve_reference(&label, site)?
            }
        };

        let mut color = base.color;
        if let Some(brightness) = &value.brightness {
            let rgb =
                self.require_opaque(&base, color, base_label(&value.base), site, "brightness")?;
            color = apply_brightness(rgb, brightness.value as f32);
        }

        if let Some(opacity) = &value.opacity {
            let rgb =
                self.require_opaque(&base, color, base_label(&value.base), site, "opacity")?;
            let (ground, ground_label) = match &value.over {
                Some(over) => {
                    let mut over_site = site.clone();
                    over_site.path = format!("{}.over", site.path);
                    over_site.span = over.span.clone();
                    (
                        self.resolve_color(&over.value, &over_site)?,
                        base_label(&over.value.base),
                    )
                }
                // The spec's default mixing ground.
                None => {
                    let label = "semantic.background".to_string();
                    (self.resolve_reference(&label, site)?, label)
                }
            };
            let ground_rgb =
                self.require_opaque(&ground, ground.color, ground_label, site, "opacity")?;
            color = mix_over(rgb, ground_rgb, opacity.value as f32);
        }

        Some(ColorOutcome {
            color,
            terminal_origin: match color {
                Color::Reset => base.terminal_origin,
                _ => None,
            },
        })
    }

    /// The RGB channels of `color`, or a diagnostic naming where the
    /// `"terminal"` it resolved to actually comes from.
    fn require_opaque(
        &mut self,
        outcome: &ColorOutcome,
        color: Color,
        label: String,
        site: &Site,
        what: &str,
    ) -> Option<[u8; 3]> {
        if let Color::Rgb(r, g, b) = color {
            return Some([r, g, b]);
        }
        let origin = outcome
            .terminal_origin
            .clone()
            .unwrap_or_else(|| site.theme.clone());
        self.diagnostics.push(
            ThemeDiagnostic::error(
                site.origin.clone(),
                Some(site.span.clone()),
                format!("{label} resolves via {origin} to terminal; {what} requires opaque RGB"),
            )
            .with_help(format!(
                "`{}` needs an opaque colour here; set an explicit `over` or an opaque \
                 `semantic.background`",
                site.path
            )),
        );
        None
    }

    // -- gradients --------------------------------------------------------

    fn resolve_gradients(&mut self) -> (Vec<ResolvedGradient>, BTreeMap<String, GradientId>) {
        let names: Vec<String> = self.merged.gradients.keys().cloned().collect();
        let mut gradients = Vec::with_capacity(names.len());
        let mut ids = BTreeMap::new();

        for (index, name) in names.iter().enumerate() {
            let entry = self.merged.gradients[name].clone();
            ids.insert(name.clone(), GradientId::new(index));

            let direction = entry
                .value
                .direction
                .as_ref()
                .and_then(|raw| GradientDirection::from_key(&raw.value))
                .unwrap_or(GradientDirection::Horizontal);

            let mut stops = Vec::with_capacity(entry.value.stops.len());
            let mut complete = true;
            for (position, stop) in entry.value.stops.iter().enumerate() {
                let (Some(at), Some(color)) = (&stop.at, &stop.color) else {
                    complete = false;
                    continue;
                };
                let mut site = entry.site(format!("gradients.{name}.stops[{position}].color"));
                site.span = color.span.clone();
                let Some(outcome) = self.resolve_color(&color.value, &site) else {
                    complete = false;
                    continue;
                };
                if outcome.color == Color::Reset {
                    let origin = outcome
                        .terminal_origin
                        .clone()
                        .unwrap_or_else(|| entry.theme.clone());
                    self.diagnostics.push(ThemeDiagnostic::error(
                        site.origin.clone(),
                        Some(site.span.clone()),
                        format!(
                            "{} resolves via {origin} to terminal; gradient stops require \
                             opaque RGB",
                            base_label(&color.value.base)
                        ),
                    ));
                    complete = false;
                    continue;
                }
                stops.push(ResolvedGradientStop {
                    position: at.value as f32,
                    color: outcome.color,
                });
            }

            // A seam check on a half-resolved stop list would be noise on top
            // of the error that already explains why it is half-resolved.
            if complete && direction == GradientDirection::Perimeter {
                let ends = (stops.first(), stops.last());
                if let (Some(first), Some(last)) = ends {
                    if first.color != last.color {
                        self.diagnostics.push(
                            ThemeDiagnostic::error(
                                entry.origin.clone(),
                                Some(entry.span.clone()),
                                format!(
                                    "`gradients.{name}` is a `perimeter` gradient; its first \
                                     and last stop must resolve to the same colour"
                                ),
                            )
                            .with_help("a perimeter gradient runs a ring and would show a seam"),
                        );
                    }
                }
            }

            gradients.push(ResolvedGradient { direction, stops });
        }
        (gradients, ids)
    }

    // -- components -------------------------------------------------------

    fn resolve_components(
        &mut self,
        id: &ThemeId,
        semantic: &ResolvedSemantic,
        gradient_ids: &BTreeMap<String, GradientId>,
        gradients: &[ResolvedGradient],
    ) -> ResolvedComponents {
        let (color_count, style_count, paint_count, tint_count) = role_counts();
        let mut colors = vec![Color::Reset; color_count];
        let mut styles = vec![Style::default(); style_count];
        let mut paints = vec![ResolvedPaint::Solid(Color::Reset); paint_count];
        let mut tints = vec![ResolvedTint::Native; tint_count];

        for spec in ROLE_SPECS {
            match (spec.role, spec.fallback) {
                (RoleRef::Color(role), RoleFallback::Color(slot)) => {
                    let index = role as usize;
                    colors[index] = self
                        .resolve_role_color(index, spec.path)
                        .unwrap_or_else(|| semantic.slot(slot));
                }
                (RoleRef::Style(role), RoleFallback::Style(recipe)) => {
                    styles[role as usize] =
                        self.resolve_role_style(role as usize, spec.path, semantic, recipe);
                }
                (RoleRef::Paint(role), RoleFallback::Paint(slot)) => {
                    paints[role as usize] = self
                        .resolve_role_paint(id, spec, gradient_ids, gradients)
                        .unwrap_or_else(|| ResolvedPaint::Solid(semantic.slot(slot)));
                }
                (RoleRef::Tint(role), RoleFallback::Tint(fallback)) => {
                    tints[role as usize] = self
                        .resolve_role_tint(role as usize, spec.path)
                        .unwrap_or(match fallback {
                            crate::theme::catalog::SemanticTint::Native => ResolvedTint::Native,
                            crate::theme::catalog::SemanticTint::Color(slot) => {
                                ResolvedTint::Color(semantic.slot(slot))
                            }
                        });
                }
                // The catalogue guards this pairing in its own tests; a
                // mismatch here would silently mis-type a role.
                _ => unreachable!("{} has a type-incompatible fallback", spec.path),
            }
        }

        ResolvedComponents::new(
            to_array(colors),
            to_array(styles),
            to_array(paints),
            to_array(tints),
        )
    }

    fn resolve_role_color(&mut self, index: usize, path: &str) -> Option<Color> {
        let entry = self.merged.colors[index].clone()?;
        let site = entry.site(path.to_string());
        self.resolve_color(&entry.value, &site)
            .map(|outcome| outcome.color)
    }

    fn resolve_role_style(
        &mut self,
        index: usize,
        path: &str,
        semantic: &ResolvedSemantic,
        recipe: crate::theme::catalog::SemanticStyle,
    ) -> Style {
        let mut style = semantic_style(semantic, recipe);
        let merged = self.merged.styles[index].clone();
        if let Some(foreground) = merged.foreground {
            let site = foreground.site(format!("{path}.foreground"));
            if let Some(outcome) = self.resolve_color(&foreground.value, &site) {
                style.fg = Some(outcome.color);
            }
        }
        if let Some(background) = merged.background {
            let site = background.site(format!("{path}.background"));
            if let Some(outcome) = self.resolve_color(&background.value, &site) {
                style.bg = Some(outcome.color);
            }
        }
        if let Some(modifiers) = merged.modifiers {
            // A set list replaces the fallback's modifiers completely, so an
            // empty list is the documented way to clear them.
            let mut set = Modifier::empty();
            for name in &modifiers.value {
                if let Some(modifier) = modifier_from_key(&name.value) {
                    set |= modifier;
                }
            }
            style.add_modifier = set;
            style.sub_modifier = Modifier::empty();
        }
        style
    }

    fn resolve_role_paint(
        &mut self,
        theme_id: &ThemeId,
        spec: &crate::theme::catalog::RoleSpec,
        gradient_ids: &BTreeMap<String, GradientId>,
        gradients: &[ResolvedGradient],
    ) -> Option<ResolvedPaint> {
        let RoleRef::Paint(role) = spec.role else {
            return None;
        };
        let entry = self.merged.paints[role as usize].clone()?;
        let site = entry.site(spec.path.to_string());
        match &entry.value {
            PaintOverride::Color(color) => self
                .resolve_color(color, &site)
                .map(|outcome| ResolvedPaint::Solid(outcome.color)),
            PaintOverride::Gradient(name) => {
                let Some(id) = gradient_ids.get(name) else {
                    self.diagnostics.push(ThemeDiagnostic::error(
                        site.origin.clone(),
                        Some(site.span.clone()),
                        format!("unknown gradient `gradients.{name}`"),
                    ));
                    return None;
                };
                let gradient_theme = self.merged.gradients[name].theme.clone();
                let perimeter = gradients
                    .get(id.index())
                    .is_some_and(|gradient| gradient.direction == GradientDirection::Perimeter);
                // Task 3 already rejects this pairing, but only in the file it
                // is written in — and that file's diagnostics are not part of
                // *this* outcome unless it is the theme being resolved. So the
                // only case that may be suppressed here is both halves living
                // in the theme under resolution.
                let reported_by_this_files_validator =
                    gradient_theme == entry.theme && &entry.theme == theme_id;
                if perimeter && !spec.closed_frame && !reported_by_this_files_validator {
                    self.diagnostics.push(ThemeDiagnostic::error(
                        site.origin.clone(),
                        Some(site.span.clone()),
                        format!(
                            "`{}` is not a closed frame and cannot use the `perimeter` \
                             gradient `{name}` inherited from `{gradient_theme}`",
                            spec.path
                        ),
                    ));
                    return None;
                }
                Some(ResolvedPaint::Gradient(*id))
            }
        }
    }

    fn resolve_role_tint(&mut self, index: usize, path: &str) -> Option<ResolvedTint> {
        let entry = self.merged.tints[index].clone()?;
        let site = entry.site(path.to_string());
        match &entry.value {
            TintOverride::Native => Some(ResolvedTint::Native),
            TintOverride::Color(color) => self
                .resolve_color(color, &site)
                .map(|outcome| ResolvedTint::Color(outcome.color)),
        }
    }
}

/// Id of the only theme without a parent.
const ROOT_THEME_ID: &str = "default";

fn role_counts() -> (usize, usize, usize, usize) {
    use crate::theme::catalog::{ColorRole, PaintRole, StyleRole, TintRole};
    (
        ColorRole::COUNT,
        StyleRole::COUNT,
        PaintRole::COUNT,
        TintRole::COUNT,
    )
}

fn to_array<T: std::fmt::Debug, const N: usize>(values: Vec<T>) -> [T; N] {
    values
        .try_into()
        .expect("role vectors are sized by the catalogue COUNT constants")
}

fn join_chain(chain: &[ThemeId]) -> String {
    chain
        .iter()
        .map(ThemeId::as_str)
        .collect::<Vec<_>>()
        .join(" -> ")
}

/// How a colour base is named in a diagnostic.
fn base_label(base: &ColorBase) -> String {
    match base {
        ColorBase::Reference(reference) => {
            format!("{}.{}", reference.scope.prefix(), reference.name)
        }
        ColorBase::Terminal => "terminal".to_string(),
        ColorBase::Hex(_) | ColorBase::Rgb(_) => "the colour".to_string(),
    }
}

/// Positive brightness mixes towards white, negative towards black.
fn apply_brightness(rgb: [u8; 3], brightness: f32) -> Color {
    let target = if brightness >= 0.0 { 255.0 } else { 0.0 };
    let amount = brightness.abs();
    let channel = |value: u8| -> u8 {
        let value = value as f32;
        (value + (target - value) * amount)
            .clamp(0.0, 255.0)
            .round() as u8
    };
    Color::Rgb(channel(rgb[0]), channel(rgb[1]), channel(rgb[2]))
}

/// `result = color * opacity + ground * (1 - opacity)`, per channel in sRGB.
fn mix_over(rgb: [u8; 3], ground: [u8; 3], opacity: f32) -> Color {
    let channel = |value: u8, ground: u8| -> u8 {
        (value as f32 * opacity + ground as f32 * (1.0 - opacity))
            .clamp(0.0, 255.0)
            .round() as u8
    };
    Color::Rgb(
        channel(rgb[0], ground[0]),
        channel(rgb[1], ground[1]),
        channel(rgb[2], ground[2]),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use ratatui::style::{Color, Modifier, Style};

    use crate::theme::catalog::{
        PaintRole, RoleFallback, RoleRef, SemanticStyle, StyleRole, TintRole, ROLE_SPECS,
    };
    use crate::theme::model::{
        semantic_style, ResolvedPaint, ResolvedTheme, ResolvedTint, ThemeDefinition,
        ThemeDiagnostic, ThemeId, ThemeOrigin, ValidationMode,
    };
    use crate::theme::resolve::{resolve_theme, ResolveOutcome};
    use crate::theme::validate::parse_and_validate;

    /// The frozen `default` semantic core of the spec, plus the two component
    /// overrides the merge-order tests need.
    const DEFAULT_TEST_THEME: &str = "\
schema_version = 1
name = \"Default\"

[semantic]
background = \"terminal\"
canvas = \"#0b0d10\"
surface = \"terminal\"
surface_raised = \"terminal\"
border = \"#1f2a24\"
border_focus = \"#6fb3b8\"
border_popup = \"#6a7a72\"
text = \"#d6e1d4\"
text_bright = \"#c7e8c9\"
text_highlight = \"#f4f8f3\"
text_muted = \"#6a7a72\"
text_dim = \"#3d4a44\"
text_inverse = \"#06080a\"
accent = \"#9ec99b\"
selection_bg = \"#182b22\"
selection_fg = \"#c7e8c9\"
success = \"#7cb992\"
warning = \"#d6a76b\"
error = \"#c97a7a\"
info = \"#6fb3b8\"
connecting = \"#d6a76b\"
exited = \"#c97a7a\"
unknown = \"#3d4a44\"

[components.focus.indicator]
foreground = \"semantic.accent\"
";

    fn id(raw: &str) -> ThemeId {
        ThemeId::parse(raw).expect("valid theme id")
    }

    /// Parse and validate every source in memory, asserting each is clean, so a
    /// resolver test can never be passing because of a validator error.
    fn definitions<'a>(
        entries: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> BTreeMap<ThemeId, ThemeDefinition> {
        let mut map = BTreeMap::new();
        for (raw_id, source) in entries {
            let theme = id(raw_id);
            let origin = ThemeOrigin::User(PathBuf::from(format!("{raw_id}.toml")));
            let outcome = parse_and_validate(theme.clone(), origin, source, ValidationMode::Strict);
            assert!(
                !outcome.has_errors(),
                "fixture `{raw_id}` is not valid: {:#?}",
                outcome.diagnostics
            );
            map.insert(theme, outcome.definition.expect("definition"));
        }
        map
    }

    /// Same, but without the cleanliness assertion — for the one case that has
    /// to model a *parent* file whose own validator run already found a
    /// problem, because that run's diagnostics never reach the child's outcome.
    fn definitions_unchecked<'a>(
        entries: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> BTreeMap<ThemeId, ThemeDefinition> {
        let mut map = BTreeMap::new();
        for (raw_id, source) in entries {
            let theme = id(raw_id);
            let origin = ThemeOrigin::User(PathBuf::from(format!("{raw_id}.toml")));
            let outcome = parse_and_validate(theme.clone(), origin, source, ValidationMode::Strict);
            map.insert(theme, outcome.definition.expect("definition"));
        }
        map
    }

    fn definition_refs(
        definitions: &BTreeMap<ThemeId, ThemeDefinition>,
    ) -> BTreeMap<ThemeId, &ThemeDefinition> {
        definitions
            .iter()
            .map(|(id, definition)| (id.clone(), definition))
            .collect()
    }

    /// Resolve `target` against `default` plus the given extra sources.
    fn resolve_with<'a>(
        target: &str,
        extra: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> ResolveOutcome {
        let mut sources: Vec<(&str, &str)> = vec![("default", DEFAULT_TEST_THEME)];
        sources.extend(extra);
        let definitions = definitions(sources);
        resolve_theme(&id(target), &definition_refs(&definitions))
    }

    fn resolved(target: &str, extra: Vec<(&str, &str)>) -> ResolvedTheme {
        let outcome = resolve_with(target, extra);
        assert!(
            outcome.diagnostics.iter().all(|d| !d.is_error()),
            "unexpected errors: {:#?}",
            outcome.diagnostics
        );
        outcome.theme.expect("a clean theme resolves")
    }

    fn assert_diagnostic_contains(diagnostics: &[ThemeDiagnostic], needle: &str) {
        assert!(
            diagnostics.iter().any(|d| d.message.contains(needle)),
            "no diagnostic contains {needle:?}: {:#?}",
            diagnostics
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
        );
    }

    fn assert_failed_with(outcome: &ResolveOutcome, needle: &str) {
        assert!(outcome.theme.is_none(), "expected no theme for {needle:?}");
        assert_diagnostic_contains(&outcome.diagnostics, needle);
    }

    // -----------------------------------------------------------------------
    // Merge before resolution
    // -----------------------------------------------------------------------

    #[test]
    fn references_are_resolved_after_the_definition_merge() {
        let definitions = definitions([
            ("default", DEFAULT_TEST_THEME),
            (
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [semantic]\naccent = \"#ff0000\"\n",
            ),
        ]);
        let result = resolve_theme(&id("child"), &definition_refs(&definitions));
        let theme = result.theme.unwrap();
        assert_eq!(
            theme.style(StyleRole::FocusIndicator).fg,
            Some(Color::Rgb(255, 0, 0))
        );
    }

    #[test]
    fn auto_resets_whole_roles_and_individual_style_fields_to_catalog_fallbacks() {
        let theme = resolved(
            "grandchild",
            vec![
                (
                    "child",
                    "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                     [components.dashboard.host_list]\nborder = \"#010203\"\n\
                     [components.footer.key]\nbackground = \"#040506\"\nmodifiers = [\"bold\"]\n",
                ),
                (
                    "grandchild",
                    "schema_version = 1\nname = \"Grandchild\"\nextends = \"child\"\n\
                     [components.dashboard.host_list]\nborder = \"auto\"\n\
                     [components.footer.key]\nbackground = \"auto\"\n",
                ),
            ],
        );
        assert_eq!(
            theme.paint(PaintRole::DashboardHostListBorder),
            &ResolvedPaint::Solid(theme.semantic.border)
        );
        assert_eq!(
            theme.style(StyleRole::FooterKey).bg,
            semantic_style(&theme.semantic, SemanticStyle::TextBright).bg
        );
        // The sibling field the grandchild did not reset must survive.
        assert!(theme
            .style(StyleRole::FooterKey)
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn a_child_may_reference_names_defined_only_in_the_parent() {
        let theme = resolved(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [components.focus.indicator]\nbackground = \"semantic.canvas\"\n",
            )],
        );
        assert_eq!(
            theme.style(StyleRole::FocusIndicator).bg,
            Some(Color::Rgb(0x0b, 0x0d, 0x10))
        );
    }

    #[test]
    fn the_inheritance_chain_runs_from_the_child_to_the_root() {
        let outcome = resolve_with(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n",
            )],
        );
        assert_eq!(outcome.inheritance_chain, vec![id("child"), id("default")]);
    }

    #[test]
    fn a_theme_without_extends_inherits_default_implicitly() {
        let theme = resolved(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\n[semantic]\naccent = \"#ff0000\"\n",
            )],
        );
        // `text` is never set by the child, so it can only come from `default`.
        assert_eq!(theme.semantic.text, Color::Rgb(0xd6, 0xe1, 0xd4));
    }

    // -----------------------------------------------------------------------
    // Cycles, missing links and limits
    // -----------------------------------------------------------------------

    #[test]
    fn a_direct_inheritance_cycle_is_reported() {
        let definitions = definitions([(
            "child",
            "schema_version = 1\nname = \"Child\"\nextends = \"child\"\n",
        )]);
        let outcome = resolve_theme(&id("child"), &definition_refs(&definitions));
        assert_failed_with(&outcome, "inheritance cycle");
    }

    #[test]
    fn an_indirect_inheritance_cycle_is_reported() {
        let definitions = definitions([
            ("a", "schema_version = 1\nname = \"A\"\nextends = \"b\"\n"),
            ("b", "schema_version = 1\nname = \"B\"\nextends = \"c\"\n"),
            ("c", "schema_version = 1\nname = \"C\"\nextends = \"a\"\n"),
        ]);
        let outcome = resolve_theme(&id("a"), &definition_refs(&definitions));
        assert_failed_with(&outcome, "inheritance cycle");
    }

    #[test]
    fn a_missing_parent_is_reported() {
        let definitions = definitions([(
            "child",
            "schema_version = 1\nname = \"Child\"\nextends = \"ghost\"\n",
        )]);
        let outcome = resolve_theme(&id("child"), &definition_refs(&definitions));
        assert_failed_with(&outcome, "unknown parent theme `ghost`");
    }

    #[test]
    fn a_missing_theme_is_reported() {
        let definitions = definitions([("default", DEFAULT_TEST_THEME)]);
        let outcome = resolve_theme(&id("ghost"), &definition_refs(&definitions));
        assert_failed_with(&outcome, "unknown theme `ghost`");
    }

    #[test]
    fn a_colour_reference_cycle_is_reported() {
        let outcome = resolve_with(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [palette]\na = \"palette.b\"\nb = \"palette.a\"\n\
                 [semantic]\naccent = \"palette.a\"\n",
            )],
        );
        assert_failed_with(&outcome, "colour reference cycle");
    }

    #[test]
    fn a_missing_colour_reference_is_reported() {
        let outcome = resolve_with(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [semantic]\naccent = \"palette.ghost\"\n",
            )],
        );
        assert_failed_with(&outcome, "unknown colour reference `palette.ghost`");
    }

    #[test]
    fn a_missing_gradient_reference_is_reported() {
        let outcome = resolve_with(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [components.dashboard.host_list]\nborder = { gradient = \"gradients.ghost\" }\n",
            )],
        );
        assert_failed_with(&outcome, "unknown gradient `gradients.ghost`");
    }

    /// A tower of `levels` themes, the last one extending `default`.
    fn inheritance_tower(levels: usize) -> Vec<(String, String)> {
        (0..levels)
            .map(|i| {
                let parent = if i + 1 == levels {
                    "default".to_string()
                } else {
                    format!("t{}", i + 1)
                };
                (
                    format!("t{i}"),
                    format!("schema_version = 1\nname = \"T{i}\"\nextends = \"{parent}\"\n"),
                )
            })
            .collect()
    }

    #[test]
    fn inheritance_depth_is_capped_at_sixteen() {
        // 15 themes plus `default` is exactly the limit.
        let owned = inheritance_tower(15);
        let extra: Vec<(&str, &str)> = owned
            .iter()
            .map(|(id, src)| (id.as_str(), src.as_str()))
            .collect();
        let outcome = resolve_with("t0", extra);
        assert!(
            outcome.theme.is_some(),
            "16 themes must resolve: {:#?}",
            outcome.diagnostics
        );
        assert_eq!(outcome.inheritance_chain.len(), 16);

        let owned = inheritance_tower(16);
        let extra: Vec<(&str, &str)> = owned
            .iter()
            .map(|(id, src)| (id.as_str(), src.as_str()))
            .collect();
        let outcome = resolve_with("t0", extra);
        assert_failed_with(&outcome, "inheritance is nested deeper than 16 themes");
    }

    /// `p0 -> p1 -> … -> p{len-1} = #010203`, entered through `semantic.accent`.
    fn palette_chain(len: usize) -> String {
        let mut source =
            "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n[palette]\n".to_string();
        for i in 0..len {
            if i + 1 == len {
                source.push_str(&format!("p{i} = \"#010203\"\n"));
            } else {
                source.push_str(&format!("p{i} = \"palette.p{}\"\n", i + 1));
            }
        }
        source.push_str("[semantic]\naccent = \"palette.p0\"\n");
        source
    }

    #[test]
    fn colour_reference_depth_is_capped_at_sixteen() {
        // `semantic.accent` plus 15 palette entries is exactly 16 entries deep.
        let source = palette_chain(15);
        let outcome = resolve_with("child", vec![("child", source.as_str())]);
        assert!(
            outcome.theme.is_some(),
            "16 entries must resolve: {:#?}",
            outcome.diagnostics
        );

        let source = palette_chain(16);
        let outcome = resolve_with("child", vec![("child", source.as_str())]);
        assert_failed_with(
            &outcome,
            "colour reference is nested deeper than 16 entries",
        );
    }

    // -----------------------------------------------------------------------
    // Colour maths
    // -----------------------------------------------------------------------

    #[test]
    fn brightness_is_applied_before_opacity() {
        // #808080 lightened by 0.5 is 192; mixed half over black that is 96.
        // Swapping the two steps would give 160 instead.
        let theme = resolved(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [palette]\nground = \"#000000\"\nbase = \"#808080\"\n\
                 [semantic]\naccent = { color = \"palette.base\", brightness = 0.5, \
                 opacity = 0.5, over = \"palette.ground\" }\n",
            )],
        );
        assert_eq!(theme.semantic.accent, Color::Rgb(96, 96, 96));
    }

    #[test]
    fn colour_maths_round_and_clamp_at_the_endpoints() {
        let theme = resolved(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [palette]\nground = \"#000000\"\nbase = \"#808080\"\n\
                 [semantic]\n\
                 accent = { color = \"palette.base\", brightness = 1.0 }\n\
                 warning = { color = \"palette.base\", brightness = -1.0 }\n\
                 error = { color = \"palette.base\", opacity = 1.0, over = \"palette.ground\" }\n\
                 info = { color = \"palette.base\", opacity = 0.0, over = \"palette.ground\" }\n\
                 success = { color = \"palette.base\", brightness = 0.5 }\n",
            )],
        );
        assert_eq!(theme.semantic.accent, Color::Rgb(255, 255, 255));
        assert_eq!(theme.semantic.warning, Color::Rgb(0, 0, 0));
        assert_eq!(theme.semantic.error, Color::Rgb(128, 128, 128));
        assert_eq!(theme.semantic.info, Color::Rgb(0, 0, 0));
        // 128 + 127 * 0.5 = 191.5, rounded away from zero.
        assert_eq!(theme.semantic.success, Color::Rgb(192, 192, 192));
    }

    #[test]
    fn a_dangling_reference_in_an_unreferenced_palette_entry_is_still_reported() {
        let outcome = resolve_with(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [palette]\ndead = \"palette.ghost\"\n",
            )],
        );
        assert_failed_with(&outcome, "unknown colour reference `palette.ghost`");
    }

    #[test]
    fn diagnostics_come_back_in_presentation_order() {
        let outcome = resolve_with(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [palette]\ndead = \"palette.ghost\"\n\
                 [semantic]\naccent = \"palette.nowhere\"\n\
                 [components.footer.key]\nforeground = \"palette.missing\"\n",
            )],
        );
        assert_eq!(outcome.diagnostics.len(), 3);
        assert!(
            outcome
                .diagnostics
                .windows(2)
                .all(|pair| pair[0].sort_key() <= pair[1].sort_key()),
            "diagnostics are not sorted: {:#?}",
            outcome
                .diagnostics
                .iter()
                .map(|d| (d.span.clone(), d.message.as_str()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_runaway_reference_chain_is_reported_once_for_all_its_consumers() {
        // Both semantic slots enter the same over-long chain, and the depth
        // failure is deliberately not cached — the message must not double.
        let mut source = palette_chain(16);
        source.push_str("warning = \"palette.p0\"\n");
        let outcome = resolve_with("child", vec![("child", source.as_str())]);
        let depth = outcome
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("nested deeper than 16 entries"))
            .count();
        assert_eq!(depth, 1, "{:#?}", outcome.diagnostics);
    }

    #[test]
    fn the_implicit_mixing_ground_is_the_merged_semantic_background() {
        let theme = resolved(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [palette]\nbase = \"#ffffff\"\n\
                 [semantic]\nbackground = \"#000000\"\n\
                 accent = { color = \"palette.base\", opacity = 0.5 }\n",
            )],
        );
        assert_eq!(theme.semantic.accent, Color::Rgb(128, 128, 128));
    }

    #[test]
    fn an_opacity_ground_that_resolves_to_terminal_names_its_origin() {
        let outcome = resolve_with(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [components.footer.key]\nbackground = { color = \"semantic.accent\", \
                 opacity = 0.5, over = \"semantic.surface\" }\n",
            )],
        );
        assert_failed_with(
            &outcome,
            "semantic.surface resolves via default to terminal; opacity requires opaque RGB",
        );
    }

    #[test]
    fn the_implicit_ground_reports_its_own_origin_when_it_is_terminal() {
        // `default` leaves `semantic.background` at `"terminal"`.
        let outcome = resolve_with(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [palette]\nbase = \"#ffffff\"\n\
                 [semantic]\naccent = { color = \"palette.base\", opacity = 0.5 }\n",
            )],
        );
        assert_failed_with(
            &outcome,
            "semantic.background resolves via default to terminal; opacity requires opaque RGB",
        );
    }

    #[test]
    fn a_transform_on_a_colour_that_resolves_to_terminal_is_rejected() {
        let outcome = resolve_with(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [semantic]\naccent = { color = \"semantic.surface\", brightness = 0.2 }\n",
            )],
        );
        assert_failed_with(
            &outcome,
            "semantic.surface resolves via default to terminal; brightness requires opaque RGB",
        );
    }

    // -----------------------------------------------------------------------
    // Gradients
    // -----------------------------------------------------------------------

    #[test]
    fn a_gradient_stop_that_resolves_to_terminal_is_rejected() {
        let outcome = resolve_with(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [gradients.ring]\ndirection = \"horizontal\"\n\
                 stops = [{ at = 0.0, color = \"semantic.surface\" }, \
                 { at = 1.0, color = \"#ffffff\" }]\n",
            )],
        );
        assert_failed_with(
            &outcome,
            "semantic.surface resolves via default to terminal; gradient stops require opaque RGB",
        );
    }

    #[test]
    fn a_perimeter_gradient_needs_identical_resolved_endpoints() {
        let outcome = resolve_with(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [gradients.ring]\ndirection = \"perimeter\"\n\
                 stops = [{ at = 0.0, color = \"#000000\" }, { at = 1.0, color = \"#ffffff\" }]\n",
            )],
        );
        assert_failed_with(
            &outcome,
            "first and last stop must resolve to the same colour",
        );
    }

    #[test]
    fn a_perimeter_gradient_whose_endpoints_agree_after_resolution_is_accepted() {
        let theme = resolved(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [palette]\nedge = \"#102030\"\n\
                 [gradients.ring]\ndirection = \"perimeter\"\n\
                 stops = [{ at = 0.0, color = \"palette.edge\" }, \
                 { at = 0.5, color = \"#ffffff\" }, { at = 1.0, color = \"#102030\" }]\n\
                 [components.dashboard.host_list]\nborder = { gradient = \"gradients.ring\" }\n",
            )],
        );
        let ResolvedPaint::Gradient(gradient) = theme.paint(PaintRole::DashboardHostListBorder)
        else {
            panic!("expected a gradient paint");
        };
        assert_eq!(theme.gradients[gradient.index()].stops.len(), 3);
    }

    #[test]
    fn an_inherited_perimeter_gradient_is_rejected_on_an_open_role() {
        let outcome = resolve_with(
            "child",
            vec![
                (
                    "base",
                    "schema_version = 1\nname = \"Base\"\nextends = \"default\"\n\
                     [gradients.ring]\ndirection = \"perimeter\"\n\
                     stops = [{ at = 0.0, color = \"#102030\" }, \
                     { at = 1.0, color = \"#102030\" }]\n",
                ),
                (
                    "child",
                    "schema_version = 1\nname = \"Child\"\nextends = \"base\"\n\
                     [components.separator]\nprimary = { gradient = \"gradients.ring\" }\n",
                ),
            ],
        );
        assert_failed_with(&outcome, "is not a closed frame");
    }

    #[test]
    fn an_ancestor_that_defines_both_halves_still_fails_the_child() {
        // `b` writes the gradient *and* the offending override, so the two
        // provenances match — but `b`'s own validator diagnostics are not part
        // of `c`'s outcome, and a caller gating activation on this outcome
        // would otherwise ship the seam.
        // `b` is the one fixture that is deliberately *not* validator-clean.
        let definitions = definitions_unchecked([
            ("default", DEFAULT_TEST_THEME),
            (
                "b",
                "schema_version = 1\nname = \"B\"\nextends = \"default\"\n\
                 [gradients.ring]\ndirection = \"perimeter\"\n\
                 stops = [{ at = 0.0, color = \"#102030\" }, \
                 { at = 1.0, color = \"#102030\" }]\n\
                 [components.separator]\nprimary = { gradient = \"gradients.ring\" }\n",
            ),
            ("c", "schema_version = 1\nname = \"C\"\nextends = \"b\"\n"),
        ]);
        let outcome = resolve_theme(&id("c"), &definition_refs(&definitions));
        assert_failed_with(&outcome, "is not a closed frame");
    }

    // -----------------------------------------------------------------------
    // Styles, resets and completeness
    // -----------------------------------------------------------------------

    #[test]
    fn an_empty_modifier_list_clears_inherited_modifiers() {
        let theme = resolved(
            "grandchild",
            vec![
                (
                    "child",
                    "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                     [components.footer.key]\nmodifiers = [\"bold\", \"italic\"]\n",
                ),
                (
                    "grandchild",
                    "schema_version = 1\nname = \"Grandchild\"\nextends = \"child\"\n\
                     [components.footer.key]\nmodifiers = []\n",
                ),
            ],
        );
        assert_eq!(
            theme.style(StyleRole::FooterKey).add_modifier,
            Modifier::empty()
        );
    }

    #[test]
    fn modifiers_auto_restores_the_catalogue_fallback() {
        let theme = resolved(
            "grandchild",
            vec![
                (
                    "child",
                    "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                     [components.form.input_editing]\nmodifiers = []\n",
                ),
                (
                    "grandchild",
                    "schema_version = 1\nname = \"Grandchild\"\nextends = \"child\"\n\
                     [components.form.input_editing]\nmodifiers = \"auto\"\n",
                ),
            ],
        );
        assert_eq!(
            theme.style(StyleRole::FormInputEditing).add_modifier,
            Modifier::UNDERLINED | Modifier::BOLD
        );
    }

    #[test]
    fn auto_true_next_to_concrete_style_fields_resets_first() {
        let theme = resolved(
            "grandchild",
            vec![
                (
                    "child",
                    "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                     [components.footer.key]\nforeground = \"#010203\"\n\
                     background = \"#040506\"\nmodifiers = [\"bold\"]\n",
                ),
                (
                    "grandchild",
                    "schema_version = 1\nname = \"Grandchild\"\nextends = \"child\"\n\
                     [components.footer.key]\nauto = true\nforeground = \"#ff0000\"\n",
                ),
            ],
        );
        let style = theme.style(StyleRole::FooterKey);
        // The sibling field wins over the reset it sits next to …
        assert_eq!(style.fg, Some(Color::Rgb(255, 0, 0)));
        // … while everything it does not mention falls back to the catalogue.
        let fallback = semantic_style(&theme.semantic, SemanticStyle::TextBright);
        assert_eq!(style.bg, fallback.bg);
        assert_eq!(style.add_modifier, fallback.add_modifier);
    }

    #[test]
    fn terminal_stays_a_legal_style_colour() {
        let theme = resolved(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [components.footer.key]\nbackground = \"terminal\"\n",
            )],
        );
        assert_eq!(theme.style(StyleRole::FooterKey).bg, Some(Color::Reset));
    }

    #[test]
    fn a_tint_role_resolves_native_and_auto() {
        let theme = resolved(
            "child",
            vec![(
                "child",
                "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                 [components.os_logo]\ntint = \"semantic.accent\"\n",
            )],
        );
        assert_eq!(
            theme.tint(TintRole::OsLogoTint),
            &ResolvedTint::Color(Color::Rgb(0x9e, 0xc9, 0x9b))
        );

        let theme = resolved(
            "grandchild",
            vec![
                (
                    "child",
                    "schema_version = 1\nname = \"Child\"\nextends = \"default\"\n\
                     [components.os_logo]\ntint = \"semantic.accent\"\n",
                ),
                (
                    "grandchild",
                    "schema_version = 1\nname = \"Grandchild\"\nextends = \"child\"\n\
                     [components.os_logo]\ntint = \"auto\"\n",
                ),
            ],
        );
        assert_eq!(theme.tint(TintRole::OsLogoTint), &ResolvedTint::Native);
    }

    #[test]
    fn every_role_without_an_override_carries_its_catalogue_fallback() {
        let definitions = definitions([("default", DEFAULT_TEST_THEME)]);
        let outcome = resolve_theme(&id("default"), &definition_refs(&definitions));
        let theme = outcome.theme.expect("the root theme resolves");

        for spec in ROLE_SPECS {
            // `components.focus.indicator` is the one override in the fixture.
            if spec.path == "components.focus.indicator" {
                continue;
            }
            match (spec.role, spec.fallback) {
                (RoleRef::Color(role), RoleFallback::Color(slot)) => {
                    assert_eq!(
                        theme.color(role),
                        theme.semantic.slot(slot),
                        "{}",
                        spec.path
                    );
                }
                (RoleRef::Style(role), RoleFallback::Style(recipe)) => {
                    assert_eq!(
                        theme.style(role),
                        semantic_style(&theme.semantic, recipe),
                        "{}",
                        spec.path
                    );
                }
                (RoleRef::Paint(role), RoleFallback::Paint(slot)) => {
                    assert_eq!(
                        theme.paint(role),
                        &ResolvedPaint::Solid(theme.semantic.slot(slot)),
                        "{}",
                        spec.path
                    );
                }
                (RoleRef::Tint(role), RoleFallback::Tint(_)) => {
                    assert_eq!(theme.tint(role), &ResolvedTint::Native, "{}", spec.path);
                }
                _ => panic!("{} has a type-incompatible fallback", spec.path),
            }
        }
        assert_eq!(
            theme.style(StyleRole::FocusIndicator),
            Style::default().fg(theme.semantic.accent)
        );
    }

    #[test]
    fn a_semantic_slot_missing_from_the_whole_chain_is_reported() {
        let definitions = definitions([(
            "default",
            "schema_version = 1\nname = \"Bare\"\n[semantic]\ntext = \"#ffffff\"\n",
        )]);
        let outcome = resolve_theme(&id("default"), &definition_refs(&definitions));
        assert_failed_with(&outcome, "`semantic.accent` is not defined");
    }
}
