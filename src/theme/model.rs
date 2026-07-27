//! Theme data model: identifiers, span-carrying definitions and the fully
//! resolved runtime theme.
//!
//! The file has two halves that must not be mixed up:
//!
//! * the *definition* side ([`ThemeDefinition`] and friends) is what a theme
//!   file literally says, with the byte range of every value kept so that a
//!   diagnostic can point at `file:line:column`. Nothing here is resolved,
//!   range-checked or merged.
//! * the *resolved* side ([`ResolvedTheme`]) is the end state of the pipeline
//!   (parse → validate → inherit → resolve). It carries no optional values and
//!   no unresolved strings: every semantic slot and every component role is a
//!   concrete ratatui value, so renderers never fall back at draw time.

use std::borrow::Cow;
use std::fmt;
use std::ops::Range;
use std::path::PathBuf;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::theme::catalog::{
    ColorRole, PaintRole, SemanticSlot, SemanticStyle, StyleRole, TintRole, SEMANTIC_SPECS,
};

/// Technical identifier of a theme.
///
/// For user themes this is the file stem of `themes/<id>.toml`, which is why
/// the accepted character set is deliberately narrower than the display name:
/// an id is used to build a path, so anything that could escape the themes
/// directory or vary by filesystem case folding is rejected.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThemeId(String);

/// Why a string is not a usable [`ThemeId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeIdError {
    Empty,
    InvalidCharacter(char),
}

impl fmt::Display for ThemeIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "theme id must not be empty"),
            Self::InvalidCharacter(c) => write!(
                f,
                "invalid character {c:?} in theme id (allowed: a-z, 0-9, '-', '_')"
            ),
        }
    }
}

impl std::error::Error for ThemeIdError {}

impl ThemeId {
    /// Parse and validate a theme id.
    pub fn parse(raw: &str) -> Result<Self, ThemeIdError> {
        if raw.is_empty() {
            return Err(ThemeIdError::Empty);
        }
        if let Some(bad) = raw
            .chars()
            .find(|c| !matches!(c, 'a'..='z' | '0'..='9' | '-' | '_'))
        {
            return Err(ThemeIdError::InvalidCharacter(bad));
        }
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ThemeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How strictly a theme file is checked.
///
/// The CLI checker runs [`ValidationMode::Strict`] so authors see every
/// problem; the runtime runs [`ValidationMode::Compatible`] so a theme written
/// against a newer SSHub, which may name component roles this build does not
/// know yet, still loads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationMode {
    Strict,
    Compatible,
}

// ---------------------------------------------------------------------------
// Definition side — the theme file as written, with source spans.
// ---------------------------------------------------------------------------

/// Where a theme definition came from.
///
/// Diagnostics carry this so a message can name the offending file, and so
/// diagnostics collected from several themes still sort deterministically.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeOrigin {
    /// One of the theme assets compiled into the binary.
    BuiltIn,
    /// A file below `config_dir()/themes`.
    User(PathBuf),
}

impl ThemeOrigin {
    /// Stable label used in diagnostics and as the primary sort key.
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::BuiltIn => Cow::Borrowed("<built-in>"),
            Self::User(path) => path.to_string_lossy(),
        }
    }
}

/// A value plus the byte range it occupies in the source it was parsed from.
///
/// Ranges are byte offsets into the original file text, which is what
/// `toml_edit` reports and what a `line:column` renderer needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Range<usize>,
}

impl<T> Spanned<T> {
    pub fn new(value: T, span: Range<usize>) -> Self {
        Self { value, span }
    }
}

/// The two halves of a qualified colour reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceScope {
    Palette,
    Semantic,
}

impl ReferenceScope {
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Palette => "palette",
            Self::Semantic => "semantic",
        }
    }
}

/// A `palette.<name>` or `semantic.<name>` reference, still unresolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorReference {
    pub scope: ReferenceScope,
    pub name: String,
}

/// The base of a colour value, before brightness and opacity are applied.
///
/// `Rgb` keeps the raw integers: range checking is the validator's job, and
/// it needs the per-channel span to point at the offending number.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorBase {
    /// The `"terminal"` sentinel — the emulator's own default colour.
    Terminal,
    /// `#RRGGBB`.
    Hex([u8; 3]),
    /// `rgb = [r, g, b]`, unchecked.
    Rgb([Spanned<i64>; 3]),
    Reference(ColorReference),
}

/// A colour as written in a theme file.
///
/// The transforms stay separate and unevaluated so the validator can reject
/// combinations the spec forbids (brightness on `"terminal"`, `over` without
/// `opacity`) while still pointing at the exact field.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorValue {
    pub base: ColorBase,
    /// Span of the base alone — the bare string, or the `rgb`/`color` value.
    pub base_span: Range<usize>,
    /// Whether the base was written as `color = …` rather than as a bare value
    /// or `rgb = …`. The spec restricts `color` to qualified references, and
    /// that rule cannot be recovered from the parsed value alone: a hex base
    /// looks the same either way.
    pub base_from_color_key: bool,
    pub brightness: Option<Spanned<f64>>,
    pub opacity: Option<Spanned<f64>>,
    pub over: Option<Box<Spanned<ColorValue>>>,
}

/// One `[palette]` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteEntry {
    pub name: Spanned<String>,
    pub value: Spanned<ColorValue>,
}

/// One `[semantic]` entry that names a known slot.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticEntry {
    pub slot: SemanticSlot,
    pub key: Spanned<String>,
    pub value: Spanned<ColorValue>,
}

/// One stop of a `[gradients.<name>]` definition.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientStopDefinition {
    pub at: Option<Spanned<f64>>,
    pub color: Option<Spanned<ColorValue>>,
    pub span: Range<usize>,
}

/// A named gradient as written.
///
/// `direction` stays a raw string: an unknown direction has to survive parsing
/// so the validator can suggest the closest supported one.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientDefinition {
    pub name: Spanned<String>,
    pub direction: Option<Spanned<String>>,
    pub stops: Vec<GradientStopDefinition>,
    /// Span of the `stops` array, or of the gradient table when absent.
    pub stops_span: Range<usize>,
    pub span: Range<usize>,
}

/// A `Color` role's value: a colour or the `"auto"` reset sentinel.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorSlot {
    Auto,
    Color(ColorValue),
}

/// A `Paint` role's value.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintSlot {
    Auto,
    Color(ColorValue),
    /// `{ gradient = "gradients.<name>" }`; the span covers the reference.
    Gradient(Spanned<String>),
}

/// A `Tint` role's value. `Native` keeps an asset's own colours.
#[derive(Clone, Debug, PartialEq)]
pub enum TintSlot {
    Auto,
    Native,
    Color(ColorValue),
}

/// The `modifiers` field of a style. `Auto` restores the inherited list,
/// an empty `List` clears it.
#[derive(Clone, Debug, PartialEq)]
pub enum ModifierList {
    Auto,
    List(Vec<Spanned<String>>),
}

/// A `Style` role's value. Every field is optional so a theme can override a
/// single half of an inherited style.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleValue {
    /// `{ auto = true }` — reset the whole role to its semantic fallback.
    pub auto: Option<Spanned<bool>>,
    pub foreground: Option<Spanned<ColorSlot>>,
    pub background: Option<Spanned<ColorSlot>>,
    pub modifiers: Option<Spanned<ModifierList>>,
}

/// The value of one `components.*` assignment, typed by its catalogue role.
///
/// A role this build does not know cannot be typed, so it is kept as
/// [`ComponentValue::Unknown`]. The parser never decides whether that is an
/// error — that is the validator's `Strict` vs `Compatible` policy.
#[derive(Clone, Debug, PartialEq)]
pub enum ComponentValue {
    Color {
        role: ColorRole,
        value: Spanned<ColorSlot>,
    },
    /// Boxed because a partial style is by far the largest component value and
    /// would otherwise set the size of every entry in the definition.
    Style {
        role: StyleRole,
        value: Box<Spanned<StyleValue>>,
    },
    Paint {
        role: PaintRole,
        value: Spanned<PaintSlot>,
    },
    Tint {
        role: TintRole,
        value: Spanned<TintSlot>,
    },
    Unknown {
        value_span: Range<usize>,
    },
}

/// One assignment below `[components]`, addressed by its full role path.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentEntry {
    /// Full literal path, e.g. `components.footer.key`. The span covers the
    /// key that completed the path, not the whole path.
    pub path: Spanned<String>,
    pub value: ComponentValue,
}

impl ComponentEntry {
    /// Whether this build's catalogue knows the role.
    pub fn is_known(&self) -> bool {
        !matches!(self.value, ComponentValue::Unknown { .. })
    }
}

/// A key the schema does not define, kept verbatim.
///
/// Retaining these is what lets the validator apply its mode policy and offer
/// a spelling suggestion instead of the parser silently dropping the key.
#[derive(Clone, Debug, PartialEq)]
pub struct UnknownField {
    /// Full dotted path of the key, e.g. `semantic.accnt`.
    pub path: Spanned<String>,
    pub value_span: Range<usize>,
}

/// A theme file turned into data, with every value still traceable to source.
///
/// Ordering of the vectors follows the file, so diagnostics and `theme show`
/// stay stable.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeDefinition {
    pub id: ThemeId,
    pub origin: ThemeOrigin,
    /// `None` when the key is missing or not an integer; the exact value is
    /// checked by the validator, which must reject anything but `1`.
    pub schema_version: Option<Spanned<i64>>,
    /// Required. A missing `name` is reported by the parser and leaves an
    /// empty value with an empty span behind.
    pub name: Spanned<String>,
    pub extends: Option<Spanned<String>>,
    pub description: Option<Spanned<String>>,
    pub author: Option<Spanned<String>>,
    pub palette: Vec<PaletteEntry>,
    pub semantic: Vec<SemanticEntry>,
    pub gradients: Vec<GradientDefinition>,
    pub components: Vec<ComponentEntry>,
    pub unknown_fields: Vec<UnknownField>,
}

/// How bad a diagnostic is. Only errors stop a theme from being used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

/// One problem found in a theme file.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeDiagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub help: Option<String>,
    /// `None` for problems that belong to the file as a whole.
    pub span: Option<Range<usize>>,
    pub origin: ThemeOrigin,
}

impl ThemeDiagnostic {
    pub fn error(
        origin: ThemeOrigin,
        span: Option<Range<usize>>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: DiagnosticSeverity::Error,
            message: message.into(),
            help: None,
            span,
            origin,
        }
    }

    pub fn warning(
        origin: ThemeOrigin,
        span: Option<Range<usize>>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: DiagnosticSeverity::Warning,
            message: message.into(),
            help: None,
            span,
            origin,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == DiagnosticSeverity::Error
    }

    pub fn is_warning(&self) -> bool {
        self.severity == DiagnosticSeverity::Warning
    }

    /// Total order used to present diagnostics: by file, then by source
    /// position, then errors before warnings, then by message. File-wide
    /// diagnostics (no span) sort to the top of their file.
    pub fn sort_key(&self) -> (Cow<'_, str>, usize, usize, DiagnosticSeverity, &str) {
        let (start, end) = match &self.span {
            Some(span) => (span.start, span.end),
            None => (0, 0),
        };
        (
            self.origin.label(),
            start,
            end,
            self.severity,
            self.message.as_str(),
        )
    }
}

// ---------------------------------------------------------------------------
// Resolved side — the immutable runtime theme.
// ---------------------------------------------------------------------------

/// Index of a resolved gradient inside [`ResolvedTheme::gradients`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GradientId(usize);

impl GradientId {
    pub fn new(index: usize) -> Self {
        Self(index)
    }

    pub fn index(self) -> usize {
        self.0
    }
}

/// Direction a gradient is sampled along, relative to the component rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientDirection {
    Horizontal,
    Vertical,
    DiagonalDown,
    DiagonalUp,
    Perimeter,
}

impl GradientDirection {
    /// The five V1 directions, keyed by their literal spelling in a theme file.
    pub const KEYS: [&'static str; 5] = [
        "horizontal",
        "vertical",
        "diagonal_down",
        "diagonal_up",
        "perimeter",
    ];

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "horizontal" => Some(Self::Horizontal),
            "vertical" => Some(Self::Vertical),
            "diagonal_down" => Some(Self::DiagonalDown),
            "diagonal_up" => Some(Self::DiagonalUp),
            "perimeter" => Some(Self::Perimeter),
            _ => None,
        }
    }
}

/// The six text modifiers a theme may name, in the order the spec lists them.
///
/// Kept next to [`GradientDirection::KEYS`] for the same reason: the validator
/// needs the literal spellings to suggest a correction, and the resolver needs
/// the mapping — neither may keep its own copy of the list.
pub const MODIFIER_KEYS: [&str; 6] = [
    "bold",
    "dim",
    "italic",
    "underlined",
    "reversed",
    "crossed_out",
];

/// The [`Modifier`] a theme file's modifier name stands for.
pub fn modifier_from_key(key: &str) -> Option<Modifier> {
    match key {
        "bold" => Some(Modifier::BOLD),
        "dim" => Some(Modifier::DIM),
        "italic" => Some(Modifier::ITALIC),
        "underlined" => Some(Modifier::UNDERLINED),
        "reversed" => Some(Modifier::REVERSED),
        "crossed_out" => Some(Modifier::CROSSED_OUT),
        _ => None,
    }
}

/// A paint role's resolved value: a single colour or a named gradient.
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedPaint {
    Solid(Color),
    Gradient(GradientId),
}

/// A tint role's resolved value. `Native` keeps the asset's own colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedTint {
    Native,
    Color(Color),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedGradientStop {
    pub position: f32,
    pub color: Color,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedGradient {
    pub direction: GradientDirection,
    pub stops: Vec<ResolvedGradientStop>,
}

impl ResolvedGradient {
    /// Colour at relative position `t`, clamped to `0.0..=1.0`.
    ///
    /// Channels are interpolated in sRGB and rounded per the V1 colour rules,
    /// so a gradient renders identically on every platform.
    pub fn sample(&self, t: f32) -> Color {
        let Some(first) = self.stops.first() else {
            return Color::Reset;
        };
        let t = t.clamp(0.0, 1.0);
        let mut lower = first;
        for stop in &self.stops {
            if stop.position <= t {
                lower = stop;
            } else {
                let span = stop.position - lower.position;
                let local = if span <= f32::EPSILON {
                    0.0
                } else {
                    (t - lower.position) / span
                };
                return mix_srgb(lower.color, stop.color, local);
            }
        }
        lower.color
    }
}

/// Linear per-channel sRGB mix; non-RGB endpoints cannot be interpolated and
/// snap to the nearer end instead of silently producing black.
fn mix_srgb(from: Color, to: Color, t: f32) -> Color {
    let (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) = (from, to) else {
        return if t < 0.5 { from } else { to };
    };
    let channel = |a: u8, b: u8| -> u8 {
        let value = a as f32 + (b as f32 - a as f32) * t;
        value.clamp(0.0, 255.0).round() as u8
    };
    Color::Rgb(channel(r0, r1), channel(g0, g1), channel(b0, b1))
}

/// The fixed semantic core of schema version 1 — exactly 23 slots.
///
/// Component fallbacks only ever reference these names, so overriding one
/// semantic slot re-tints every component that inherits from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedSemantic {
    pub background: Color,
    pub canvas: Color,
    pub surface: Color,
    pub surface_raised: Color,
    pub border: Color,
    pub border_focus: Color,
    pub border_popup: Color,
    pub text: Color,
    pub text_bright: Color,
    pub text_highlight: Color,
    pub text_muted: Color,
    pub text_dim: Color,
    pub text_inverse: Color,
    pub accent: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub info: Color,
    pub connecting: Color,
    pub exited: Color,
    pub unknown: Color,
}

/// Number of slots in the fixed semantic core, derived from the catalogue so a
/// slot added there cannot silently leave a hole in [`ResolvedSemantic`].
pub const SEMANTIC_SLOT_COUNT: usize = SEMANTIC_SPECS.len();

impl ResolvedSemantic {
    /// Build the core from slot-indexed colours, in [`SemanticSlot`] order.
    ///
    /// The resolver works on an indexed array (it fills slots by catalogue
    /// order), while renderers want named fields; this is the single crossing
    /// point between the two views.
    pub fn from_slots(slots: [Color; SEMANTIC_SLOT_COUNT]) -> Self {
        Self {
            background: slots[SemanticSlot::Background as usize],
            canvas: slots[SemanticSlot::Canvas as usize],
            surface: slots[SemanticSlot::Surface as usize],
            surface_raised: slots[SemanticSlot::SurfaceRaised as usize],
            border: slots[SemanticSlot::Border as usize],
            border_focus: slots[SemanticSlot::BorderFocus as usize],
            border_popup: slots[SemanticSlot::BorderPopup as usize],
            text: slots[SemanticSlot::Text as usize],
            text_bright: slots[SemanticSlot::TextBright as usize],
            text_highlight: slots[SemanticSlot::TextHighlight as usize],
            text_muted: slots[SemanticSlot::TextMuted as usize],
            text_dim: slots[SemanticSlot::TextDim as usize],
            text_inverse: slots[SemanticSlot::TextInverse as usize],
            accent: slots[SemanticSlot::Accent as usize],
            selection_bg: slots[SemanticSlot::SelectionBg as usize],
            selection_fg: slots[SemanticSlot::SelectionFg as usize],
            success: slots[SemanticSlot::Success as usize],
            warning: slots[SemanticSlot::Warning as usize],
            error: slots[SemanticSlot::Error as usize],
            info: slots[SemanticSlot::Info as usize],
            connecting: slots[SemanticSlot::Connecting as usize],
            exited: slots[SemanticSlot::Exited as usize],
            unknown: slots[SemanticSlot::Unknown as usize],
        }
    }

    /// The colour of one semantic slot.
    pub fn slot(&self, slot: SemanticSlot) -> Color {
        match slot {
            SemanticSlot::Background => self.background,
            SemanticSlot::Canvas => self.canvas,
            SemanticSlot::Surface => self.surface,
            SemanticSlot::SurfaceRaised => self.surface_raised,
            SemanticSlot::Border => self.border,
            SemanticSlot::BorderFocus => self.border_focus,
            SemanticSlot::BorderPopup => self.border_popup,
            SemanticSlot::Text => self.text,
            SemanticSlot::TextBright => self.text_bright,
            SemanticSlot::TextHighlight => self.text_highlight,
            SemanticSlot::TextMuted => self.text_muted,
            SemanticSlot::TextDim => self.text_dim,
            SemanticSlot::TextInverse => self.text_inverse,
            SemanticSlot::Accent => self.accent,
            SemanticSlot::SelectionBg => self.selection_bg,
            SemanticSlot::SelectionFg => self.selection_fg,
            SemanticSlot::Success => self.success,
            SemanticSlot::Warning => self.warning,
            SemanticSlot::Error => self.error,
            SemanticSlot::Info => self.info,
            SemanticSlot::Connecting => self.connecting,
            SemanticSlot::Exited => self.exited,
            SemanticSlot::Unknown => self.unknown,
        }
    }
}

/// The [`Style`] a [`SemanticStyle`] recipe stands for, given a resolved core.
///
/// The recipes are declared once as doc comments on [`SemanticStyle`]; this is
/// their executable form, and the only place a style fallback is spelled out.
pub fn semantic_style(semantic: &ResolvedSemantic, recipe: SemanticStyle) -> Style {
    let fg = |color: Color| Style::default().fg(color);
    let pair = |fg: Color, bg: Color| Style::default().fg(fg).bg(bg);
    match recipe {
        SemanticStyle::Text => fg(semantic.text),
        SemanticStyle::TextBright => fg(semantic.text_bright),
        SemanticStyle::TextBrightBold => fg(semantic.text_bright).add_modifier(Modifier::BOLD),
        SemanticStyle::TextBrightUnderlinedBold => {
            fg(semantic.text_bright).add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
        }
        SemanticStyle::TextMuted => fg(semantic.text_muted),
        SemanticStyle::TextDim => fg(semantic.text_dim),
        SemanticStyle::TextOnSurfaceRaised => pair(semantic.text, semantic.surface_raised),
        SemanticStyle::HighlightOnSelection => pair(semantic.text_highlight, semantic.selection_bg),
        SemanticStyle::Selection => pair(semantic.selection_fg, semantic.selection_bg),
        SemanticStyle::Inverse => pair(semantic.text_inverse, semantic.text_bright),
        SemanticStyle::InverseOnWarning => pair(semantic.text_inverse, semantic.warning),
        SemanticStyle::Accent => fg(semantic.accent),
        SemanticStyle::Info => fg(semantic.info),
        SemanticStyle::Success => fg(semantic.success),
        SemanticStyle::Warning => fg(semantic.warning),
        SemanticStyle::Error => fg(semantic.error),
    }
}

/// Every component role of the V1 catalogue, indexed by its typed enum.
///
/// Storage is four flat arrays sized by the generated `COUNT` constants: role
/// lookup is an array index, never a string map lookup.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedComponents {
    colors: [Color; ColorRole::COUNT],
    styles: [Style; StyleRole::COUNT],
    paints: [ResolvedPaint; PaintRole::COUNT],
    tints: [ResolvedTint; TintRole::COUNT],
}

impl ResolvedComponents {
    /// Build the component table. Consuming the arrays is what keeps a
    /// resolved theme immutable: there is no other way in or out.
    pub fn new(
        colors: [Color; ColorRole::COUNT],
        styles: [Style; StyleRole::COUNT],
        paints: [ResolvedPaint; PaintRole::COUNT],
        tints: [ResolvedTint; TintRole::COUNT],
    ) -> Self {
        Self {
            colors,
            styles,
            paints,
            tints,
        }
    }
}

/// A validated, fully inherited theme ready to render with.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTheme {
    pub id: ThemeId,
    pub name: String,
    pub description: Option<String>,
    pub semantic: ResolvedSemantic,
    pub gradients: Vec<ResolvedGradient>,
    pub components: ResolvedComponents,
}

impl ResolvedTheme {
    pub fn color(&self, role: ColorRole) -> Color {
        self.components.colors[role as usize]
    }

    pub fn style(&self, role: StyleRole) -> Style {
        self.components.styles[role as usize]
    }

    pub fn paint(&self, role: PaintRole) -> &ResolvedPaint {
        &self.components.paints[role as usize]
    }

    pub fn tint(&self, role: TintRole) -> &ResolvedTint {
        &self.components.tints[role as usize]
    }

    /// The gradient behind a paint role, or `None` when it resolved to a solid
    /// colour. Callers use this to decide whether a cheap solid render needs
    /// the gradient post-pass at all.
    pub fn paint_gradient(&self, role: PaintRole) -> Option<&ResolvedGradient> {
        match self.paint(role) {
            ResolvedPaint::Solid(_) => None,
            ResolvedPaint::Gradient(id) => self.gradients.get(id.index()),
        }
    }

    /// Colour of a paint role at one cell of `area`.
    ///
    /// Solid paints ignore the position; gradients are sampled with the
    /// direction semantics of the V1 spec, where coordinates are always
    /// relative to the component rect rather than the screen.
    pub fn paint_color_at(&self, role: PaintRole, area: Rect, x: u16, y: u16) -> Color {
        match self.paint(role) {
            ResolvedPaint::Solid(color) => *color,
            ResolvedPaint::Gradient(id) => match self.gradients.get(id.index()) {
                Some(gradient) => {
                    gradient.sample(gradient_position(gradient.direction, area, x, y))
                }
                None => Color::Reset,
            },
        }
    }
}

/// Relative sample position of cell `(x, y)` within `area`.
fn gradient_position(direction: GradientDirection, area: Rect, x: u16, y: u16) -> f32 {
    let norm = |value: u16, origin: u16, len: u16| -> f32 {
        if len <= 1 {
            0.0
        } else {
            (value.saturating_sub(origin)) as f32 / (len - 1) as f32
        }
    };
    let hx = norm(x, area.x, area.width);
    let vy = norm(y, area.y, area.height);
    let flat_x = area.height <= 1;
    let flat_y = area.width <= 1;

    match direction {
        GradientDirection::Horizontal => hx,
        GradientDirection::Vertical => vy,
        GradientDirection::DiagonalDown => match (flat_x, flat_y) {
            (true, true) => 0.0,
            (true, false) => hx,
            (false, true) => vy,
            (false, false) => (hx + vy) / 2.0,
        },
        GradientDirection::DiagonalUp => match (flat_x, flat_y) {
            (true, true) => 0.0,
            (true, false) => hx,
            (false, true) => 1.0 - vy,
            (false, false) => (hx + (1.0 - vy)) / 2.0,
        },
        GradientDirection::Perimeter => perimeter_position(area, x, y),
    }
}

/// Position along the clockwise outer ring of `area`, starting at its top-left
/// corner. Degenerate rects fall back to their natural single-line direction.
fn perimeter_position(area: Rect, x: u16, y: u16) -> f32 {
    if area.width <= 1 && area.height <= 1 {
        return 0.0;
    }
    if area.height <= 1 {
        return (x.saturating_sub(area.x)) as f32 / (area.width - 1) as f32;
    }
    if area.width <= 1 {
        return (y.saturating_sub(area.y)) as f32 / (area.height - 1) as f32;
    }

    let w = area.width as u32;
    let h = area.height as u32;
    let dx = x.saturating_sub(area.x) as u32;
    let dy = y.saturating_sub(area.y) as u32;
    let last_x = w - 1;
    let last_y = h - 1;

    let index = if dy == 0 {
        dx
    } else if dx == last_x {
        last_x + dy
    } else if dy == last_y {
        last_x + last_y + (last_x - dx)
    } else if dx == 0 {
        2 * last_x + last_y + (last_y - dy)
    } else {
        // Interior cells are not on the ring; anchor them at the start so a
        // caller that paints too much still gets a defined colour.
        0
    };
    let length = 2 * w + 2 * h - 4;
    index as f32 / (length - 1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_id_accepts_only_v1_filename_characters() {
        for valid in ["default", "high-contrast", "aqua_2", "fire9"] {
            assert!(ThemeId::parse(valid).is_ok(), "{valid}");
        }
        for invalid in ["", "Aqua", "two words", "../aqua", "aqua.toml"] {
            assert!(ThemeId::parse(invalid).is_err(), "{invalid}");
        }
    }
}
