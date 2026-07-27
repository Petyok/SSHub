//! Schema validation of a parsed theme definition.
//!
//! The parser reports *shape*: it decides whether a value can be represented at
//! all. This module reports *schema*: legal ranges, legal spellings, legal
//! combinations and the size limits of V1. What it deliberately does **not** do
//! is resolve anything — reference existence, inheritance cycles, the opacity
//! mixing ground behind a reference and the resolved endpoints of a `perimeter`
//! gradient all need a merged, resolved theme and therefore belong to the
//! resolver.
//!
//! [`ValidationMode`] changes exactly one verdict: an unknown *component role*
//! is an error for the CLI checker and a warning at runtime, so a theme written
//! against a newer SSHub still loads. Everything else — unknown sections,
//! unknown semantic slots, unknown style fields, wrong shapes and wrong schema
//! versions — stays fatal in both modes.

use std::ops::Range;

use strsim::levenshtein;

use crate::theme::catalog::{RoleRef, SemanticSlot, ROLE_SPECS, SEMANTIC_SPECS};
use crate::theme::model::{
    ColorBase, ColorSlot, ColorValue, ComponentValue, GradientDefinition, GradientDirection,
    ModifierList, PaintSlot, Spanned, ThemeDefinition, ThemeDiagnostic, ThemeId, ThemeOrigin,
    TintSlot, ValidationMode, MODIFIER_KEYS,
};
use crate::theme::parse::{is_role_prefix, parse_theme, role_by_path, ParseOutcome};

/// V1 upper bounds. They exist so a hostile or generated file cannot make the
/// validator and resolver allocate without limit.
pub const MAX_PALETTE_ENTRIES: usize = 256;
pub const MAX_GRADIENTS: usize = 128;
pub const MAX_GRADIENT_STOPS: usize = 32;

/// The only schema version this build understands.
const SCHEMA_VERSION: i64 = 1;

/// Reserved sentinels, in the order a suggestion should break ties.
const SENTINELS: [&str; 3] = ["auto", "native", "terminal"];

const TOP_LEVEL_KEYS: [&str; 9] = [
    "author",
    "components",
    "description",
    "extends",
    "gradients",
    "name",
    "palette",
    "schema_version",
    "semantic",
];
const COLOR_KEYS: [&str; 5] = ["brightness", "color", "opacity", "over", "rgb"];
const PAINT_KEYS: [&str; 6] = ["brightness", "color", "gradient", "opacity", "over", "rgb"];
const STYLE_KEYS: [&str; 4] = ["auto", "background", "foreground", "modifiers"];
const GRADIENT_KEYS: [&str; 2] = ["direction", "stops"];
const GRADIENT_STOP_KEYS: [&str; 2] = ["at", "color"];

/// Validate a parsed definition against the V1 schema.
///
/// Diagnostics come back in presentation order (file, then source position,
/// then severity) so a caller can print them without sorting again.
pub fn validate_definition(
    definition: &ThemeDefinition,
    mode: ValidationMode,
) -> Vec<ThemeDiagnostic> {
    let mut diagnostics = Vec::new();
    validate_metadata(definition, &mut diagnostics);
    validate_colors(definition, &mut diagnostics);
    validate_gradients(definition, &mut diagnostics);
    validate_components(definition, mode, &mut diagnostics);
    sort_diagnostics(&mut diagnostics);
    diagnostics
}

/// Parse and validate one theme source in a single pass.
///
/// This is the only entry point that still has the file text, which is what
/// lets it turn the parser's generic "not a colour value" into a concrete
/// `did you mean \`terminal\`?`: the parser cannot keep a string it could not
/// represent, so the suggestion has to be attached from the source afterwards.
pub fn parse_and_validate(
    id: ThemeId,
    origin: ThemeOrigin,
    source: &str,
    mode: ValidationMode,
) -> ParseOutcome {
    let mut outcome = parse_theme(id, origin, source);
    suggest_sentinels(source, &mut outcome.diagnostics);
    if let Some(definition) = &outcome.definition {
        outcome
            .diagnostics
            .extend(validate_definition(definition, mode));
    }
    sort_diagnostics(&mut outcome.diagnostics);
    outcome
}

/// `ThemeDiagnostic::sort_key` borrows from `self`, so the total order has to
/// be applied with `sort_by` rather than `sort_by_key`.
fn sort_diagnostics(diagnostics: &mut [ThemeDiagnostic]) {
    diagnostics.sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
}

/// The parser's message for a string that is not part of the colour grammar.
/// Only those diagnostics may have their help rewritten by [`suggest_sentinels`].
const NOT_A_COLOUR_VALUE: &str = "is not a colour value";

/// Replace the parser's generic colour help with a sentinel suggestion when the
/// rejected string is a near miss of `"auto"`, `"native"` or `"terminal"`.
///
/// Scoped to the one parser message this is about: the pass reads raw source at
/// a diagnostic's span, so letting it touch type errors or the TOML syntax
/// diagnostic would make an unrelated future message able to trigger a rewrite.
fn suggest_sentinels(source: &str, diagnostics: &mut [ThemeDiagnostic]) {
    for diagnostic in diagnostics {
        if !diagnostic.message.contains(NOT_A_COLOUR_VALUE) {
            continue;
        }
        let Some(span) = &diagnostic.span else {
            continue;
        };
        let Some(raw) = source.get(span.clone()) else {
            continue;
        };
        let literal = raw.trim_matches(|c| c == '"' || c == '\'');
        if let Some(suggestion) = suggest(literal, SENTINELS) {
            diagnostic.help = Some(format!("did you mean `{suggestion}`?"));
        }
    }
}

// ---------------------------------------------------------------------------
// Suggestions
// ---------------------------------------------------------------------------

/// The closest candidate to `input`, or `None` when nothing is close enough.
///
/// Ties break lexicographically so the same typo always yields the same advice
/// regardless of catalogue order.
fn suggest<'a>(input: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut best: Option<(usize, &'a str)> = None;
    for candidate in candidates {
        let distance = levenshtein(input, candidate);
        let better = match best {
            None => true,
            Some((best_distance, best_candidate)) => {
                distance < best_distance
                    || (distance == best_distance && candidate < best_candidate)
            }
        };
        if better {
            best = Some((distance, candidate));
        }
    }
    let (distance, candidate) = best?;
    is_near_miss(distance, input, candidate).then_some(candidate)
}

/// Whether `distance` edits are close enough to be a typo rather than a
/// different word.
///
/// Two edits are a typo at any length; longer names get a proportional budget
/// so `forgrnd` still finds `foreground` while `zzz` finds nothing.
fn is_near_miss(distance: usize, input: &str, candidate: &str) -> bool {
    let longest = input.len().max(candidate.len());
    distance > 0 && (distance <= 2 || distance * 3 <= longest)
}

/// The catalogue role closest to an unknown `path`.
///
/// Scored on the diverging tail only: every role path starts with
/// `components.`, and most share several more segments, so comparing full paths
/// would inflate the proportional budget until a six-edit difference counts as a
/// near miss — the checker would then confidently suggest an unrelated role.
fn suggest_role_path(path: &str) -> Option<&'static str> {
    let mut best: Option<(usize, &'static str)> = None;
    for spec in ROLE_SPECS {
        let (input, candidate) = diverging_tails(path, spec.path);
        let distance = levenshtein(input, candidate);
        if !is_near_miss(distance, input, candidate) {
            continue;
        }
        // Ties break on the full path so the advice is stable regardless of
        // catalogue order.
        let better = best.is_none_or(|(best_distance, best_path)| {
            distance < best_distance || (distance == best_distance && spec.path < best_path)
        });
        if better {
            best = Some((distance, spec.path));
        }
    }
    best.map(|(_, path)| path)
}

/// Both paths with their common leading dot-segments removed.
fn diverging_tails<'a>(left: &'a str, right: &'static str) -> (&'a str, &'static str) {
    let mut offset = 0;
    loop {
        let next = match (left[offset..].find('.'), right[offset..].find('.')) {
            (Some(a), Some(b)) if a == b => offset + a + 1,
            _ => return (&left[offset..], &right[offset..]),
        };
        if left[offset..next] != right[offset..next] {
            return (&left[offset..], &right[offset..]);
        }
        offset = next;
    }
}

/// Attach a `did you mean` help when a suggestion exists.
fn with_suggestion<'a>(
    diagnostic: ThemeDiagnostic,
    input: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> ThemeDiagnostic {
    match suggest(input, candidates) {
        Some(suggestion) => diagnostic.with_help(format!("did you mean `{suggestion}`?")),
        None => diagnostic,
    }
}

/// The keys that could legally stand where an unknown dotted `path` stands.
///
/// Derived from the path's parent so one function covers every context the
/// parser can hand over, instead of the parser having to tag each unknown key.
fn candidates_for(path: &str) -> Vec<&'static str> {
    let Some((parent, _)) = path.rsplit_once('.') else {
        return TOP_LEVEL_KEYS.to_vec();
    };
    if parent == "semantic" {
        return SEMANTIC_SPECS.iter().map(|spec| spec.key).collect();
    }
    if parent == "palette" || parent == "gradients" || parent == "components" {
        // Free-form names; nothing to suggest.
        return Vec::new();
    }
    if let Some(rest) = parent.strip_prefix("gradients.") {
        return match rest.split_once(".stops[") {
            None if !rest.contains('.') => GRADIENT_KEYS.to_vec(),
            None => COLOR_KEYS.to_vec(),
            // `<name>.stops[i]` itself vs. a colour table nested inside it.
            Some((_, tail)) if tail.ends_with(']') => GRADIENT_STOP_KEYS.to_vec(),
            Some(_) => COLOR_KEYS.to_vec(),
        };
    }
    if parent.starts_with("components.") {
        // Walk up until a catalogue role is found: an unknown key below
        // `foreground` is still a colour key.
        let mut current = parent;
        loop {
            if let Some(spec) = role_by_path(current) {
                return match spec.role {
                    RoleRef::Style(_) if current == parent => STYLE_KEYS.to_vec(),
                    RoleRef::Paint(_) if current == parent => PAINT_KEYS.to_vec(),
                    _ => COLOR_KEYS.to_vec(),
                };
            }
            match current.rsplit_once('.') {
                Some((next, _)) if next.contains('.') => current = next,
                _ => return Vec::new(),
            }
        }
    }
    if parent.starts_with("palette.") {
        return COLOR_KEYS.to_vec();
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// Metadata, unknown keys and limits
// ---------------------------------------------------------------------------

fn validate_metadata(definition: &ThemeDefinition, out: &mut Vec<ThemeDiagnostic>) {
    let origin = &definition.origin;

    if let Some(version) = &definition.schema_version {
        if version.value != SCHEMA_VERSION {
            out.push(
                ThemeDiagnostic::error(
                    origin.clone(),
                    Some(version.span.clone()),
                    format!("unsupported `schema_version` {}", version.value),
                )
                .with_help("this build understands schema version 1"),
            );
        }
    }

    // A present-but-empty name would show as a blank row in the picker.
    if definition.name.span != (0..0) && definition.name.value.trim().is_empty() {
        out.push(
            ThemeDiagnostic::error(
                origin.clone(),
                Some(definition.name.span.clone()),
                "`name` must not be empty",
            )
            .with_help("the display name is what the theme picker shows"),
        );
    }

    if let Some(extends) = &definition.extends {
        if let Err(error) = ThemeId::parse(&extends.value) {
            out.push(
                ThemeDiagnostic::error(
                    origin.clone(),
                    Some(extends.span.clone()),
                    format!("`extends` is not a valid theme id: {error}"),
                )
                .with_help("`extends` names a theme id, never a path"),
            );
        }
    }

    for field in &definition.unknown_fields {
        let path = field.path.value.as_str();
        let last = path.rsplit('.').next().unwrap_or(path);
        out.push(with_suggestion(
            ThemeDiagnostic::error(
                origin.clone(),
                Some(field.path.span.clone()),
                format!("unknown key `{path}`"),
            ),
            last,
            candidates_for(path),
        ));
    }

    if definition.palette.len() > MAX_PALETTE_ENTRIES {
        out.push(ThemeDiagnostic::error(
            origin.clone(),
            None,
            format!(
                "a theme may define at most {MAX_PALETTE_ENTRIES} palette entries, found {}",
                definition.palette.len()
            ),
        ));
    }
    if definition.gradients.len() > MAX_GRADIENTS {
        out.push(ThemeDiagnostic::error(
            origin.clone(),
            None,
            format!(
                "a theme may define at most {MAX_GRADIENTS} gradients, found {}",
                definition.gradients.len()
            ),
        ));
    }
}

// ---------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------

/// Where a colour sits, for the rules that only apply in one place.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ColorContext {
    /// Palette entries, semantic slots other than `background`, components.
    General,
    /// `semantic.background` — it is the default mixing ground, so it has to
    /// stay opaque.
    SemanticBackground,
    /// A gradient stop — interpolation needs concrete channels.
    GradientStop,
    /// An explicit `over` value — the mixing ground has to be opaque.
    MixingGround,
}

fn validate_colors(definition: &ThemeDefinition, out: &mut Vec<ThemeDiagnostic>) {
    let origin = &definition.origin;

    for entry in &definition.palette {
        let path = format!("palette.{}", entry.name.value);
        validate_color(origin, &path, &entry.value, ColorContext::General, out);
    }

    for entry in &definition.semantic {
        let path = format!("semantic.{}", entry.key.value);
        let context = if entry.slot == SemanticSlot::Background {
            ColorContext::SemanticBackground
        } else {
            ColorContext::General
        };
        validate_color(origin, &path, &entry.value, context, out);
    }
}

/// Check one colour value and, recursively, its mixing ground.
fn validate_color(
    origin: &ThemeOrigin,
    path: &str,
    color: &Spanned<ColorValue>,
    context: ColorContext,
    out: &mut Vec<ThemeDiagnostic>,
) {
    let value = &color.value;

    if let ColorBase::Rgb(channels) = &value.base {
        for channel in channels {
            if !(0..=255).contains(&channel.value) {
                out.push(
                    ThemeDiagnostic::error(
                        origin.clone(),
                        Some(channel.span.clone()),
                        format!("`{path}.rgb` channel {} is out of range", channel.value),
                    )
                    .with_help("each channel is an integer in 0..=255"),
                );
            }
        }
    }

    if value.base_from_color_key && matches!(value.base, ColorBase::Hex(_) | ColorBase::Terminal) {
        out.push(
            ThemeDiagnostic::error(
                origin.clone(),
                Some(value.base_span.clone()),
                format!("`{path}.color` must be a `palette.<name>` or `semantic.<name>` reference"),
            )
            .with_help("write a hex literal or `\"terminal\"` as the bare value instead"),
        );
    }

    let terminal = value.base == ColorBase::Terminal;
    if terminal {
        for (field, span) in [
            (
                "brightness",
                value.brightness.as_ref().map(|v| v.span.clone()),
            ),
            ("opacity", value.opacity.as_ref().map(|v| v.span.clone())),
            ("over", value.over.as_ref().map(|v| v.span.clone())),
        ] {
            if let Some(span) = span {
                out.push(
                    ThemeDiagnostic::error(
                        origin.clone(),
                        Some(span),
                        format!("`{path}.{field}` is not allowed on `\"terminal\"`"),
                    )
                    .with_help("`\"terminal\"` has no channels to transform"),
                );
            }
        }
    }

    if let Some(brightness) = &value.brightness {
        if !(-1.0..=1.0).contains(&brightness.value) {
            out.push(ThemeDiagnostic::error(
                origin.clone(),
                Some(brightness.span.clone()),
                format!(
                    "`{path}.brightness` must be in -1.0..=1.0, found {}",
                    brightness.value
                ),
            ));
        }
    }

    if let Some(opacity) = &value.opacity {
        if !(0.0..=1.0).contains(&opacity.value) {
            out.push(ThemeDiagnostic::error(
                origin.clone(),
                Some(opacity.span.clone()),
                format!(
                    "`{path}.opacity` must be in 0.0..=1.0, found {}",
                    opacity.value
                ),
            ));
        }
        if context == ColorContext::SemanticBackground {
            out.push(
                ThemeDiagnostic::error(
                    origin.clone(),
                    Some(opacity.span.clone()),
                    "`semantic.background` must not use `opacity`",
                )
                .with_help("the background is the default mixing ground and has to stay opaque"),
            );
        }
    }

    if let Some(over) = &value.over {
        if value.opacity.is_none() && !terminal {
            out.push(
                ThemeDiagnostic::error(
                    origin.clone(),
                    Some(over.span.clone()),
                    format!("`{path}.over` requires `opacity`"),
                )
                .with_help("`over` only picks the mixing ground that `opacity` blends into"),
            );
        }
        validate_color(
            origin,
            &format!("{path}.over"),
            over,
            ColorContext::MixingGround,
            out,
        );
    }

    if terminal {
        match context {
            ColorContext::GradientStop => out.push(
                ThemeDiagnostic::error(
                    origin.clone(),
                    Some(value.base_span.clone()),
                    format!("`{path}` must be an RGB colour"),
                )
                .with_help("gradient stops are interpolated and need concrete channels"),
            ),
            ColorContext::MixingGround => out.push(
                ThemeDiagnostic::error(
                    origin.clone(),
                    Some(value.base_span.clone()),
                    format!("`{path}` must be an opaque colour"),
                )
                .with_help("`\"terminal\"` has no channels to mix into"),
            ),
            ColorContext::General | ColorContext::SemanticBackground => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Gradients
// ---------------------------------------------------------------------------

fn validate_gradients(definition: &ThemeDefinition, out: &mut Vec<ThemeDiagnostic>) {
    let origin = &definition.origin;

    for gradient in &definition.gradients {
        let path = format!("gradients.{}", gradient.name.value);

        match &gradient.direction {
            None => out.push(
                ThemeDiagnostic::error(
                    origin.clone(),
                    Some(gradient.span.clone()),
                    format!("`{path}` is missing `direction`"),
                )
                .with_help(
                    "one of `horizontal`, `vertical`, `diagonal_down`, `diagonal_up`, `perimeter`",
                ),
            ),
            Some(direction) if GradientDirection::from_key(&direction.value).is_none() => {
                out.push(with_suggestion(
                    ThemeDiagnostic::error(
                        origin.clone(),
                        Some(direction.span.clone()),
                        format!("unknown gradient direction `{}`", direction.value),
                    ),
                    &direction.value,
                    GradientDirection::KEYS,
                ));
            }
            Some(_) => {}
        }

        validate_gradient_stops(definition, gradient, &path, out);
    }
}

fn validate_gradient_stops(
    definition: &ThemeDefinition,
    gradient: &GradientDefinition,
    path: &str,
    out: &mut Vec<ThemeDiagnostic>,
) {
    let origin = &definition.origin;

    if gradient.stops.len() < 2 {
        out.push(
            ThemeDiagnostic::error(
                origin.clone(),
                Some(gradient.stops_span.clone()),
                format!(
                    "`{path}.stops` needs at least two stops, found {}",
                    gradient.stops.len()
                ),
            )
            .with_help("a gradient interpolates between stops, so it needs a start and an end"),
        );
    }
    if gradient.stops.len() > MAX_GRADIENT_STOPS {
        out.push(ThemeDiagnostic::error(
            origin.clone(),
            Some(gradient.stops_span.clone()),
            format!(
                "`{path}.stops` takes at most {MAX_GRADIENT_STOPS} stops, found {}",
                gradient.stops.len()
            ),
        ));
    }

    let mut previous: Option<f64> = None;
    for (index, stop) in gradient.stops.iter().enumerate() {
        let stop_path = format!("{path}.stops[{index}]");

        match &stop.at {
            None => out.push(ThemeDiagnostic::error(
                origin.clone(),
                Some(stop.span.clone()),
                format!("`{stop_path}` is missing `at`"),
            )),
            Some(at) => {
                if !(0.0..=1.0).contains(&at.value) {
                    out.push(ThemeDiagnostic::error(
                        origin.clone(),
                        Some(at.span.clone()),
                        format!("`{stop_path}.at` must be in 0.0..=1.0, found {}", at.value),
                    ));
                } else if previous.is_some_and(|last| at.value <= last) {
                    out.push(
                        ThemeDiagnostic::error(
                            origin.clone(),
                            Some(at.span.clone()),
                            format!("`{stop_path}.at` must be greater than the previous stop"),
                        )
                        .with_help("stop positions are strictly ascending"),
                    );
                }
                previous = Some(at.value);
            }
        }

        match &stop.color {
            None => out.push(ThemeDiagnostic::error(
                origin.clone(),
                Some(stop.span.clone()),
                format!("`{stop_path}` is missing `color`"),
            )),
            Some(color) => validate_color(
                origin,
                &format!("{stop_path}.color"),
                color,
                ColorContext::GradientStop,
                out,
            ),
        }
    }

    // Endpoints only mean something once the list is complete and in order.
    let endpoints = (
        gradient.stops.first().and_then(|stop| stop.at.as_ref()),
        gradient.stops.last().and_then(|stop| stop.at.as_ref()),
    );
    if let (Some(first), Some(last)) = endpoints {
        if gradient.stops.len() >= 2 && (first.value != 0.0 || last.value != 1.0) {
            out.push(
                ThemeDiagnostic::error(
                    origin.clone(),
                    Some(gradient.stops_span.clone()),
                    format!("`{path}.stops` must start at 0.0 and end at 1.0"),
                )
                .with_help("the outer stops anchor the gradient to the component rect"),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

fn validate_components(
    definition: &ThemeDefinition,
    mode: ValidationMode,
    out: &mut Vec<ThemeDiagnostic>,
) {
    let origin = &definition.origin;

    for entry in &definition.components {
        let path = entry.path.value.as_str();
        match &entry.value {
            ComponentValue::Unknown { value_span } => {
                validate_unknown_role(origin, path, value_span, mode, out)
            }
            ComponentValue::Color { value, .. } => {
                if let ColorSlot::Color(color) = &value.value {
                    validate_color(
                        origin,
                        path,
                        &Spanned::new(color.clone(), value.span.clone()),
                        ColorContext::General,
                        out,
                    );
                }
            }
            ComponentValue::Tint { value, .. } => {
                if let TintSlot::Color(color) = &value.value {
                    validate_color(
                        origin,
                        path,
                        &Spanned::new(color.clone(), value.span.clone()),
                        ColorContext::General,
                        out,
                    );
                }
            }
            ComponentValue::Paint { value, .. } => match &value.value {
                PaintSlot::Color(color) => validate_color(
                    origin,
                    path,
                    &Spanned::new(color.clone(), value.span.clone()),
                    ColorContext::General,
                    out,
                ),
                PaintSlot::Gradient(reference) => {
                    validate_paint_gradient(definition, path, reference, out)
                }
                PaintSlot::Auto => {}
            },
            ComponentValue::Style { value, .. } => {
                let style = &value.value;
                for (field, slot) in [
                    ("foreground", style.foreground.as_ref()),
                    ("background", style.background.as_ref()),
                ] {
                    if let Some(Spanned {
                        value: ColorSlot::Color(color),
                        span,
                    }) = slot
                    {
                        validate_color(
                            origin,
                            &format!("{path}.{field}"),
                            &Spanned::new(color.clone(), span.clone()),
                            ColorContext::General,
                            out,
                        );
                    }
                }
                if let Some(Spanned {
                    value: ModifierList::List(names),
                    ..
                }) = style.modifiers.as_ref()
                {
                    for name in names {
                        if MODIFIER_KEYS.contains(&name.value.as_str()) {
                            continue;
                        }
                        out.push(with_suggestion(
                            ThemeDiagnostic::error(
                                origin.clone(),
                                Some(name.span.clone()),
                                format!("unknown modifier `{}`", name.value),
                            ),
                            &name.value,
                            MODIFIER_KEYS,
                        ));
                    }
                }
            }
        }
    }
}

/// Policy for a role this build cannot type.
///
/// A path that is a *prefix* of a known role is a wrong shape, not a role from
/// a newer SSHub, so it stays fatal in both modes.
fn validate_unknown_role(
    origin: &ThemeOrigin,
    path: &str,
    value_span: &Range<usize>,
    mode: ValidationMode,
    out: &mut Vec<ThemeDiagnostic>,
) {
    if is_role_prefix(path) {
        out.push(
            ThemeDiagnostic::error(
                origin.clone(),
                Some(value_span.clone()),
                format!("`{path}` is a role section, not a role"),
            )
            .with_help("assign the roles inside it, not the section itself"),
        );
        return;
    }
    let message = format!("unknown component role `{path}`");
    let diagnostic = match mode {
        ValidationMode::Strict => {
            ThemeDiagnostic::error(origin.clone(), Some(value_span.clone()), message)
        }
        ValidationMode::Compatible => {
            ThemeDiagnostic::warning(origin.clone(), Some(value_span.clone()), message)
        }
    };
    out.push(match suggest_role_path(path) {
        Some(suggestion) => diagnostic.with_help(format!("did you mean `{suggestion}`?")),
        None => diagnostic,
    });
}

/// `perimeter` runs a seamless ring, which only makes sense on a closed frame.
///
/// A gradient this file does not define is left alone: resolving a reference
/// (possibly against an inherited parent) is the resolver's job.
fn validate_paint_gradient(
    definition: &ThemeDefinition,
    path: &str,
    reference: &Spanned<String>,
    out: &mut Vec<ThemeDiagnostic>,
) {
    let Some(spec) = role_by_path(path) else {
        return;
    };
    if spec.closed_frame {
        return;
    }
    let Some(gradient) = definition
        .gradients
        .iter()
        .find(|gradient| gradient.name.value == reference.value)
    else {
        return;
    };
    let perimeter = gradient
        .direction
        .as_ref()
        .is_some_and(|direction| direction.value == "perimeter");
    if perimeter {
        out.push(
            ThemeDiagnostic::error(
                definition.origin.clone(),
                Some(reference.span.clone()),
                format!(
                    "`{path}` is not a closed frame and cannot use the `perimeter` gradient `{}`",
                    reference.value
                ),
            )
            .with_help("`perimeter` is only valid on roles that paint a closed border"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::catalog::{RoleRef, ROLE_SPECS};
    use crate::theme::model::*;
    use crate::theme::parse::parse_theme;
    use std::path::PathBuf;

    fn origin() -> ThemeOrigin {
        ThemeOrigin::User(PathBuf::from("themes/test.toml"))
    }

    fn theme_id() -> ThemeId {
        ThemeId::parse("test").unwrap()
    }

    fn parse_ok(source: &str) -> ThemeDefinition {
        parse_theme(theme_id(), origin(), source)
            .definition
            .expect("source is valid TOML")
    }

    /// Parse and validate one source, exactly as a caller would.
    fn validate_source(source: &str, mode: ValidationMode) -> Vec<ThemeDiagnostic> {
        parse_and_validate(theme_id(), origin(), source, mode).diagnostics
    }

    /// A minimal valid preamble so tests only see the diagnostics they provoke.
    fn header(body: &str) -> String {
        format!("schema_version = 1\nname = \"Test\"\n{body}")
    }

    fn messages(diagnostics: &[ThemeDiagnostic]) -> Vec<&str> {
        diagnostics.iter().map(|d| d.message.as_str()).collect()
    }

    fn helps(diagnostics: &[ThemeDiagnostic]) -> Vec<&str> {
        diagnostics
            .iter()
            .filter_map(|d| d.help.as_deref())
            .collect()
    }

    fn assert_help(diagnostics: &[ThemeDiagnostic], expected: &str) {
        assert!(
            diagnostics
                .iter()
                .any(|d| d.help.as_deref() == Some(expected)),
            "expected help {expected:?}, got {:?}",
            helps(diagnostics)
        );
    }

    fn assert_contains(diagnostics: &[ThemeDiagnostic], needle: &str) {
        assert!(
            diagnostics.iter().any(|d| d.message.contains(needle)),
            "expected a diagnostic containing {needle:?}, got {:?}",
            messages(diagnostics)
        );
    }

    /// Both modes must reject `source`; only unknown component roles differ.
    fn assert_fatal_in_both_modes(source: &str, needle: &str) {
        for mode in [ValidationMode::Strict, ValidationMode::Compatible] {
            let diagnostics = validate_source(source, mode);
            let matching: Vec<_> = diagnostics
                .iter()
                .filter(|d| d.message.contains(needle))
                .collect();
            assert!(
                !matching.is_empty(),
                "{mode:?}: expected {needle:?}, got {:?}",
                messages(&diagnostics)
            );
            assert!(
                matching.iter().all(|d| d.is_error()),
                "{mode:?}: {needle:?} must stay an error"
            );
        }
    }

    #[test]
    fn compatible_only_downgrades_unknown_component_roles() {
        let definition = parse_ok(
            "schema_version = 1\nname = \"Future\"\n\
             [components.future]\nglow = \"#ffffff\"\n",
        );
        let strict = validate_definition(&definition, ValidationMode::Strict);
        let compatible = validate_definition(&definition, ValidationMode::Compatible);
        assert!(strict.iter().any(ThemeDiagnostic::is_error));
        assert!(compatible.iter().any(ThemeDiagnostic::is_warning));
        assert!(!compatible.iter().any(ThemeDiagnostic::is_error));
    }

    #[test]
    fn sentinel_typo_gets_a_specific_suggestion() {
        let diagnostics = validate_source(
            "[components.text.primary]\nforeground = \"termnial\"\n",
            ValidationMode::Strict,
        );
        assert_help(&diagnostics, "did you mean `terminal`?");
    }

    #[test]
    fn auto_and_native_typos_suggest_their_sentinels() {
        let diagnostics = validate_source(
            &header("[components.dashboard.host_list]\nborder = \"atuo\"\n"),
            ValidationMode::Strict,
        );
        assert_help(&diagnostics, "did you mean `auto`?");

        let diagnostics = validate_source(
            &header("[components.os_logo]\ntint = \"natiev\"\n"),
            ValidationMode::Strict,
        );
        assert_help(&diagnostics, "did you mean `native`?");
    }

    #[test]
    fn unknown_top_level_semantic_and_style_fields_are_fatal_in_both_modes() {
        let source = header(
            "nam = \"x\"\n\
             [semantic]\naccnt = \"#101010\"\n\
             [components.footer.key]\nforgrnd = \"#101010\"\n",
        );
        assert_fatal_in_both_modes(&source, "unknown key `nam`");
        assert_fatal_in_both_modes(&source, "unknown key `semantic.accnt`");
        assert_fatal_in_both_modes(&source, "unknown key `components.footer.key.forgrnd`");

        let diagnostics = validate_source(&source, ValidationMode::Strict);
        assert_help(&diagnostics, "did you mean `name`?");
        assert_help(&diagnostics, "did you mean `accent`?");
        assert_help(&diagnostics, "did you mean `foreground`?");
    }

    #[test]
    fn unknown_gradient_and_stop_keys_are_reported_with_their_path() {
        let source = header(
            "[gradients.ring]\ndirecton = \"horizontal\"\n\
             stops = [{ att = 0.0, color = \"#000000\" }, { at = 1.0, color = \"#ffffff\" }]\n",
        );
        assert_fatal_in_both_modes(&source, "unknown key `gradients.ring.directon`");
        assert_fatal_in_both_modes(&source, "unknown key `gradients.ring.stops[0].att`");
        let diagnostics = validate_source(&source, ValidationMode::Strict);
        assert_help(&diagnostics, "did you mean `direction`?");
        assert_help(&diagnostics, "did you mean `at`?");
    }

    #[test]
    fn schema_version_must_be_exactly_one() {
        assert_fatal_in_both_modes(
            "schema_version = 2\nname = \"Test\"\n",
            "unsupported `schema_version` 2",
        );
        let clean = validate_source(
            "schema_version = 1\nname = \"Test\"\n",
            ValidationMode::Strict,
        );
        assert!(clean.is_empty(), "{:?}", messages(&clean));
    }

    #[test]
    fn theme_ids_in_extends_are_checked() {
        assert_fatal_in_both_modes(
            "schema_version = 1\nname = \"Test\"\nextends = \"My Theme\"\n",
            "`extends` is not a valid theme id",
        );
        let ok = validate_source(
            "schema_version = 1\nname = \"Test\"\nextends = \"high-contrast\"\n",
            ValidationMode::Strict,
        );
        assert!(ok.is_empty(), "{:?}", messages(&ok));
    }

    #[test]
    fn an_empty_display_name_is_rejected() {
        assert_fatal_in_both_modes(
            "schema_version = 1\nname = \"\"\n",
            "`name` must not be empty",
        );
    }

    #[test]
    fn rgb_channels_must_be_bytes() {
        let source = header("[palette]\nbad = { rgb = [256, -1, 12] }\n");
        let diagnostics = validate_source(&source, ValidationMode::Strict);
        assert_contains(
            &diagnostics,
            "`palette.bad.rgb` channel 256 is out of range",
        );
        assert_contains(&diagnostics, "`palette.bad.rgb` channel -1 is out of range");
        assert_eq!(
            diagnostics.iter().filter(|d| d.is_error()).count(),
            2,
            "{:?}",
            messages(&diagnostics)
        );
    }

    #[test]
    fn brightness_and_opacity_stay_inside_their_ranges() {
        assert_fatal_in_both_modes(
            &header("[palette]\na = { color = \"semantic.text\", brightness = 1.5 }\n"),
            "`palette.a.brightness` must be in -1.0..=1.0",
        );
        assert_fatal_in_both_modes(
            &header("[palette]\na = { color = \"semantic.text\", opacity = 1.5 }\n"),
            "`palette.a.opacity` must be in 0.0..=1.0",
        );
        let ok = validate_source(
            &header(
                "[palette]\n\
                 a = { color = \"semantic.text\", brightness = -1.0 }\n\
                 b = { color = \"semantic.text\", opacity = 0.0, over = \"semantic.canvas\" }\n",
            ),
            ValidationMode::Strict,
        );
        assert!(ok.is_empty(), "{:?}", messages(&ok));
    }

    #[test]
    fn terminal_rejects_every_transform() {
        let source = header(
            "[palette]\n\
             a = { color = \"terminal\", brightness = 0.1, opacity = 0.5, over = \"#101010\" }\n",
        );
        let diagnostics = validate_source(&source, ValidationMode::Strict);
        assert_contains(
            &diagnostics,
            "`palette.a.brightness` is not allowed on `\"terminal\"`",
        );
        assert_contains(
            &diagnostics,
            "`palette.a.opacity` is not allowed on `\"terminal\"`",
        );
        assert_contains(
            &diagnostics,
            "`palette.a.over` is not allowed on `\"terminal\"`",
        );
    }

    #[test]
    fn over_requires_opacity_and_an_opaque_ground() {
        assert_fatal_in_both_modes(
            &header("[palette]\na = { color = \"semantic.text\", over = \"#101010\" }\n"),
            "`palette.a.over` requires `opacity`",
        );
        assert_fatal_in_both_modes(
            &header(
                "[palette]\na = { color = \"semantic.text\", opacity = 0.5, over = \"terminal\" }\n",
            ),
            "`palette.a.over` must be an opaque colour",
        );
    }

    #[test]
    fn semantic_background_must_not_use_opacity() {
        assert_fatal_in_both_modes(
            &header("[semantic]\nbackground = { color = \"palette.x\", opacity = 0.5 }\n"),
            "`semantic.background` must not use `opacity`",
        );
        let ok = validate_source(
            &header("[semantic]\ncanvas = { color = \"palette.x\", opacity = 0.5 }\n"),
            ValidationMode::Strict,
        );
        assert!(ok.is_empty(), "{:?}", messages(&ok));
    }

    #[test]
    fn the_color_key_only_takes_a_reference() {
        assert_fatal_in_both_modes(
            &header("[palette]\na = { color = \"#101010\", brightness = 0.1 }\n"),
            "`palette.a.color` must be a `palette.<name>` or `semantic.<name>` reference",
        );
        assert_fatal_in_both_modes(
            &header("[palette]\na = { color = \"terminal\" }\n"),
            "`palette.a.color` must be a `palette.<name>` or `semantic.<name>` reference",
        );
        let ok = validate_source(
            &header("[palette]\na = \"#101010\"\nb = \"terminal\"\nc = { rgb = [1, 2, 3] }\n"),
            ValidationMode::Strict,
        );
        assert!(ok.is_empty(), "{:?}", messages(&ok));
    }

    #[test]
    fn gradient_directions_are_spell_checked() {
        let diagnostics = validate_source(
            &header(
                "[gradients.ring]\ndirection = \"horziontal\"\n\
                 stops = [{ at = 0.0, color = \"#000000\" }, { at = 1.0, color = \"#ffffff\" }]\n",
            ),
            ValidationMode::Strict,
        );
        assert_contains(&diagnostics, "unknown gradient direction `horziontal`");
        assert_help(&diagnostics, "did you mean `horizontal`?");

        assert_fatal_in_both_modes(
            &header(
                "[gradients.ring]\n\
                 stops = [{ at = 0.0, color = \"#000000\" }, { at = 1.0, color = \"#ffffff\" }]\n",
            ),
            "`gradients.ring` is missing `direction`",
        );
    }

    #[test]
    fn gradient_stops_must_be_complete_ordered_and_bounded() {
        assert_fatal_in_both_modes(
            &header(
                "[gradients.ring]\ndirection = \"vertical\"\n\
                 stops = [{ at = 0.0, color = \"#000000\" }]\n",
            ),
            "`gradients.ring.stops` needs at least two stops, found 1",
        );
        assert_fatal_in_both_modes(
            &header(
                "[gradients.ring]\ndirection = \"vertical\"\n\
                 stops = [{ at = 0.0, color = \"#000000\" }, { at = 0.4, color = \"#111111\" }, \
                 { at = 0.4, color = \"#222222\" }, { at = 1.0, color = \"#ffffff\" }]\n",
            ),
            "`gradients.ring.stops[2].at` must be greater than the previous stop",
        );
        assert_fatal_in_both_modes(
            &header(
                "[gradients.ring]\ndirection = \"vertical\"\n\
                 stops = [{ at = 0.2, color = \"#000000\" }, { at = 0.9, color = \"#ffffff\" }]\n",
            ),
            "`gradients.ring.stops` must start at 0.0 and end at 1.0",
        );
        assert_fatal_in_both_modes(
            &header(
                "[gradients.ring]\ndirection = \"vertical\"\n\
                 stops = [{ at = 0.0, color = \"#000000\" }, { at = 1.5, color = \"#ffffff\" }]\n",
            ),
            "`gradients.ring.stops[1].at` must be in 0.0..=1.0",
        );
        assert_fatal_in_both_modes(
            &header(
                "[gradients.ring]\ndirection = \"vertical\"\n\
                 stops = [{ color = \"#000000\" }, { at = 1.0, color = \"#ffffff\" }]\n",
            ),
            "`gradients.ring.stops[0]` is missing `at`",
        );
        assert_fatal_in_both_modes(
            &header(
                "[gradients.ring]\ndirection = \"vertical\"\n\
                 stops = [{ at = 0.0 }, { at = 1.0, color = \"#ffffff\" }]\n",
            ),
            "`gradients.ring.stops[0]` is missing `color`",
        );
    }

    #[test]
    fn gradient_stops_reject_non_rgb_colours() {
        assert_fatal_in_both_modes(
            &header(
                "[gradients.ring]\ndirection = \"vertical\"\n\
                 stops = [{ at = 0.0, color = \"terminal\" }, { at = 1.0, color = \"#ffffff\" }]\n",
            ),
            "`gradients.ring.stops[0].color` must be an RGB colour",
        );
    }

    #[test]
    fn perimeter_is_rejected_on_roles_without_a_closed_frame() {
        let gradient = "[gradients.ring]\ndirection = \"perimeter\"\n\
             stops = [{ at = 0.0, color = \"#000000\" }, { at = 1.0, color = \"#ffffff\" }]\n";
        assert_fatal_in_both_modes(
            &header(&format!(
                "{gradient}[components.footer]\nbackground = {{ gradient = \"gradients.ring\" }}\n"
            )),
            "`components.footer.background` is not a closed frame",
        );
        let ok = validate_source(
            &header(&format!(
                "{gradient}[components.popup]\nborder = {{ gradient = \"gradients.ring\" }}\n"
            )),
            ValidationMode::Strict,
        );
        assert!(ok.is_empty(), "{:?}", messages(&ok));
    }

    #[test]
    fn modifier_names_are_checked_and_spell_checked() {
        let diagnostics = validate_source(
            &header("[components.footer.key]\nmodifiers = [\"bold\", \"itallic\"]\n"),
            ValidationMode::Strict,
        );
        assert_contains(&diagnostics, "unknown modifier `itallic`");
        assert_help(&diagnostics, "did you mean `italic`?");

        let ok = validate_source(
            &header(
                "[components.footer.key]\n\
                 modifiers = [\"bold\", \"dim\", \"italic\", \"underlined\", \"reversed\", \
                 \"crossed_out\"]\n",
            ),
            ValidationMode::Strict,
        );
        assert!(ok.is_empty(), "{:?}", messages(&ok));
    }

    #[test]
    fn auto_is_only_a_component_sentinel() {
        assert_fatal_in_both_modes(
            &header("[palette]\na = \"auto\"\n"),
            "`palette.a` is not a colour value",
        );
        assert_fatal_in_both_modes(
            &header("[semantic]\naccent = \"auto\"\n"),
            "`semantic.accent` is not a colour value",
        );
        let ok = validate_source(
            &header("[components.dashboard.host_list]\nborder = \"auto\"\n"),
            ValidationMode::Strict,
        );
        assert!(ok.is_empty(), "{:?}", messages(&ok));
    }

    #[test]
    fn native_is_only_valid_on_tint_roles() {
        assert_fatal_in_both_modes(
            &header("[components.dashboard.host_list]\nborder = \"native\"\n"),
            "`components.dashboard.host_list.border` is not a colour value",
        );
        let ok = validate_source(
            &header("[components.os_logo]\ntint = \"native\"\n"),
            ValidationMode::Strict,
        );
        assert!(ok.is_empty(), "{:?}", messages(&ok));
    }

    #[test]
    fn the_role_type_matrix_stays_fatal_in_compatible_mode() {
        assert_fatal_in_both_modes(
            &header("[components.text.primary]\nforeground = { gradient = \"gradients.ring\" }\n"),
            "does not support gradients",
        );
        assert_fatal_in_both_modes(
            &header("[components.status]\nsuccess = { gradient = \"gradients.ring\" }\n"),
            "does not support gradients",
        );
        // A scalar under a known section is a shape error, not a newer role.
        assert_fatal_in_both_modes(
            &header("[components]\nfooter = 5\n"),
            "`components.footer` must be a table of roles",
        );
    }

    #[test]
    fn a_role_section_reported_as_an_unknown_role_stays_fatal() {
        // Defensive: the parser descends known sections, so this shape cannot
        // come from a file — but the policy must not depend on that.
        let mut definition = parse_ok(&header(""));
        definition.components.push(ComponentEntry {
            path: Spanned::new("components.footer".to_string(), 0..17),
            value: ComponentValue::Unknown { value_span: 0..17 },
        });
        for mode in [ValidationMode::Strict, ValidationMode::Compatible] {
            let diagnostics = validate_definition(&definition, mode);
            assert!(
                diagnostics
                    .iter()
                    .any(|d| d.is_error() && d.message.contains("is a role section, not a role")),
                "{mode:?}: {:?}",
                messages(&diagnostics)
            );
        }
    }

    #[test]
    fn unknown_roles_suggest_the_closest_catalogue_path() {
        let diagnostics = validate_source(
            &header("[components.dashboard.host_list]\nbordr = \"#101010\"\n"),
            ValidationMode::Strict,
        );
        assert_contains(
            &diagnostics,
            "unknown component role `components.dashboard.host_list.bordr`",
        );
        assert_help(
            &diagnostics,
            "did you mean `components.dashboard.host_list.border`?",
        );
    }

    #[test]
    fn a_genuinely_new_role_gets_no_suggestion() {
        // Role paths share long prefixes, so a suggestion must be scored on the
        // diverging tail — otherwise `future` is "close" to any role at all.
        for body in [
            "[components.future]\nglow = \"#ffffff\"\n",
            "[components.sftp]\nzzzzz = \"#ffffff\"\n",
            "[components]\nquantum_flux_capacitor = { x = 1 }\n",
        ] {
            let diagnostics = validate_source(&header(body), ValidationMode::Strict);
            assert!(
                diagnostics
                    .iter()
                    .any(|d| d.message.starts_with("unknown component role")),
                "{body}: {:?}",
                messages(&diagnostics)
            );
            assert!(
                diagnostics.iter().all(|d| d.help.is_none()),
                "{body}: {:?}",
                helps(&diagnostics)
            );
        }
    }

    #[test]
    fn the_sentinel_pass_only_touches_colour_string_diagnostics() {
        // `natiev` sits in a value the parser rejects on type, not on grammar;
        // its help must stay the parser's own.
        let diagnostics = validate_source(
            &header("[components.footer.key]\nmodifiers = \"natiev\"\n"),
            ValidationMode::Strict,
        );
        assert!(
            diagnostics
                .iter()
                .all(|d| d.help.as_deref() != Some("did you mean `native`?")),
            "{:?}",
            helps(&diagnostics)
        );
    }

    #[test]
    fn size_and_count_limits_are_enforced() {
        let mut palette = String::from("[palette]\n");
        for index in 0..=MAX_PALETTE_ENTRIES {
            palette.push_str(&format!("c{index} = \"#101010\"\n"));
        }
        assert_fatal_in_both_modes(&header(&palette), "at most 256 palette entries");

        let mut gradients = String::new();
        for index in 0..=MAX_GRADIENTS {
            gradients.push_str(&format!(
                "[gradients.g{index}]\ndirection = \"vertical\"\n\
                 stops = [{{ at = 0.0, color = \"#000000\" }}, {{ at = 1.0, color = \"#ffffff\" }}]\n"
            ));
        }
        assert_fatal_in_both_modes(&header(&gradients), "at most 128 gradients");

        let mut stops = String::from("[gradients.ring]\ndirection = \"vertical\"\nstops = [\n");
        let count = MAX_GRADIENT_STOPS + 1;
        for index in 0..count {
            let at = index as f64 / (count - 1) as f64;
            stops.push_str(&format!("  {{ at = {at}, color = \"#000000\" }},\n"));
        }
        stops.push_str("]\n");
        assert_fatal_in_both_modes(&header(&stops), "at most 32 stops");
    }

    #[test]
    fn diagnostics_are_sorted_by_source_position() {
        let diagnostics = validate_source(
            &header("[palette]\nb = { rgb = [300, 0, 0] }\nc = \"nonsense\"\n"),
            ValidationMode::Strict,
        );
        let spans: Vec<_> = diagnostics.iter().filter_map(|d| d.span.clone()).collect();
        assert!(
            spans.windows(2).all(|pair| pair[0].start <= pair[1].start),
            "{spans:?}"
        );
    }

    #[test]
    fn every_catalogue_role_accepts_its_own_sentinel() {
        // Guards the validator against a role kind losing its reset path.
        for spec in ROLE_SPECS {
            let (section, key) = spec.path.rsplit_once('.').expect("role paths are dotted");
            let body = match spec.role {
                RoleRef::Style(_) => format!("[{section}]\n{key} = {{ auto = true }}\n"),
                _ => format!("[{section}]\n{key} = \"auto\"\n"),
            };
            let source = header(&body);
            let diagnostics = validate_source(&source, ValidationMode::Strict);
            assert!(
                diagnostics.is_empty(),
                "{}: {:?}",
                spec.path,
                messages(&diagnostics)
            );
        }
    }
}
