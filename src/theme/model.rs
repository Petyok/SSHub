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
use crate::theme::gradient::{gradient_position, sample_stops};

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

/// Which resolve run produced a [`ResolvedTheme`], and therefore which run's
/// [`GradientId`]s it will answer.
///
/// A theme id is not enough: reloading the same file — after the user edited
/// it, or simply because the watcher fired — produces a new gradient table at
/// the same indices under the same name. A counter that never repeats is what
/// lets the new theme refuse an id captured from the old one instead of
/// silently naming whatever now sits at that index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ThemeGeneration(u64);

impl ThemeGeneration {
    /// The next unused generation.
    ///
    /// `fetch_update` with `checked_add` rather than a plain `fetch_add`: the
    /// whole guarantee is that a generation is never reused, and a silent wrap
    /// back to `1` would hand a stale id a theme that accepts it. `Relaxed` is
    /// enough because the value is only ever compared for equality — it orders
    /// no other memory.
    ///
    /// Exhausting a `u64` is not reachable by any sequence of theme reloads a
    /// process could perform, so the failure is treated as the broken invariant
    /// it would be rather than papered over.
    pub(crate) fn next() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let value = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .expect("theme generations exhausted: u64 resolve runs in one process");
        Self(value)
    }
}

/// Index of a resolved gradient inside one resolve run's gradient table.
///
/// Minting one is the resolver's privilege: an id built from an arbitrary
/// number would name a gradient that does not exist, and every reader of a
/// [`ResolvedTheme`] is entitled to assume it does. Outside the crate, use
/// [`ResolvedTheme::gradient`] or [`ResolvedTheme::paint_gradient`].
///
/// The id carries the generation of the run that minted it, so a theme only
/// answers ids that are actually its own — see [`ThemeGeneration`]. Equality
/// therefore compares runtime identity as well as position: two ids from
/// different runs are different ids even at the same index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GradientId {
    generation: ThemeGeneration,
    index: usize,
}

impl GradientId {
    pub(crate) fn new(generation: ThemeGeneration, index: usize) -> Self {
        Self { generation, index }
    }

    pub(crate) fn index(self) -> usize {
        self.index
    }

    pub(crate) fn generation(self) -> ThemeGeneration {
        self.generation
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

/// One stop of a resolved gradient: an ordered position in `0.0..=1.0` and a
/// concrete colour. Both guarantees come from validation, so the fields are
/// readable but not writable from outside.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedGradientStop {
    pub(crate) position: f64,
    pub(crate) color: Color,
}

impl ResolvedGradientStop {
    pub fn position(&self) -> f64 {
        self.position
    }

    pub fn color(&self) -> Color {
        self.color
    }
}

/// A gradient with at least two stops, sorted by position.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedGradient {
    pub(crate) direction: GradientDirection,
    pub(crate) stops: Vec<ResolvedGradientStop>,
}

impl ResolvedGradient {
    pub fn direction(&self) -> GradientDirection {
        self.direction
    }

    /// The stops in ascending position order.
    pub fn stops(&self) -> &[ResolvedGradientStop] {
        &self.stops
    }

    /// Colour at relative position `t`, clamped to `0.0..=1.0`.
    ///
    /// Channels are interpolated in sRGB and rounded per the V1 colour rules,
    /// so a gradient renders identically on every platform.
    pub fn sample(&self, t: f64) -> Color {
        sample_stops(&self.stops, t.clamp(0.0, 1.0))
    }
}

/// The fixed semantic core of schema version 1 — exactly 23 slots.
///
/// Component fallbacks only ever reference these names, so overriding one
/// semantic slot re-tints every component that inherits from it.
///
/// The slots are filled together or not at all, which is why they are not
/// writable from outside; [`ResolvedSemantic::slot`] reads any of them by its
/// catalogue slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedSemantic {
    pub(crate) background: Color,
    pub(crate) canvas: Color,
    pub(crate) surface: Color,
    pub(crate) surface_raised: Color,
    pub(crate) border: Color,
    pub(crate) border_focus: Color,
    pub(crate) border_popup: Color,
    pub(crate) text: Color,
    pub(crate) text_bright: Color,
    pub(crate) text_highlight: Color,
    pub(crate) text_muted: Color,
    pub(crate) text_dim: Color,
    pub(crate) text_inverse: Color,
    pub(crate) accent: Color,
    pub(crate) selection_bg: Color,
    pub(crate) selection_fg: Color,
    pub(crate) success: Color,
    pub(crate) warning: Color,
    pub(crate) error: Color,
    pub(crate) info: Color,
    pub(crate) connecting: Color,
    pub(crate) exited: Color,
    pub(crate) unknown: Color,
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
    pub(crate) fn from_slots(slots: [Color; SEMANTIC_SLOT_COUNT]) -> Self {
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
        SemanticStyle::TextHighlight => fg(semantic.text_highlight),
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
    /// resolved theme immutable: there is no other way in or out, and only the
    /// resolver may go in.
    pub(crate) fn new(
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
///
/// "Validated, fully inherited" is a property of the type, not a habit of the
/// resolver: the fields are crate-private, so the only way to obtain one is to
/// resolve a theme. Everything a renderer or an export needs to read is
/// available through the accessors below.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTheme {
    pub(crate) id: ThemeId,
    /// The resolve run this theme came out of.
    ///
    /// Cloning keeps it — a clone is the same theme — so a `Rc<ResolvedTheme>`
    /// handed around the app answers exactly the ids its own run minted.
    pub(crate) generation: ThemeGeneration,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    /// The file's `author`, carried through resolution so `theme show
    /// --resolved` can write it back out. A resolved export that silently drops
    /// the credit is not the copyable document the spec asks for.
    pub(crate) author: Option<String>,
    pub(crate) semantic: ResolvedSemantic,
    pub(crate) gradients: Vec<ResolvedGradient>,
    /// The authored name of each gradient, parallel to `gradients`.
    ///
    /// Rendering never needs it — a [`GradientId`] is an index. `theme show
    /// --resolved` does: a dump that says `gradients[0]` where the source says
    /// `[gradients.reef_ring]` is not the copyable document the spec asks for.
    pub(crate) gradient_names: Vec<String>,
    pub(crate) components: ResolvedComponents,
}

impl ResolvedTheme {
    pub fn id(&self) -> &ThemeId {
        &self.id
    }

    /// The display name from the theme file.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The `author` the theme file declared, if any.
    pub fn author(&self) -> Option<&str> {
        self.author.as_deref()
    }

    /// The 23 semantic slots every component fallback is built from.
    pub fn semantic(&self) -> &ResolvedSemantic {
        &self.semantic
    }

    /// The theme's gradients, in the order the resolver numbered them.
    pub fn gradients(&self) -> &[ResolvedGradient] {
        &self.gradients
    }

    /// Everything two themes can share *except* which resolve run produced
    /// them.
    ///
    /// The derived `PartialEq` deliberately includes the generation, because a
    /// `GradientId` is only valid within its own run and two runs really are
    /// two different themes as far as ids are concerned. A round-trip test
    /// comparing an export against its original is asking the other question —
    /// "does it mean the same thing?" — and has to say so explicitly.
    /// `components` is compared **role by role** rather than as a whole,
    /// because a `ResolvedPaint::Gradient` holds a generation-bound
    /// [`GradientId`]: two runs can name the identical gradient and still
    /// compare unequal. A gradient paint is therefore compared through each
    /// theme's *own* lookup — the table itself is compared directly above — and
    /// a solid facing a gradient is a genuine difference either way.
    #[cfg(test)]
    pub(crate) fn semantically_eq(&self, other: &Self) -> bool {
        use crate::theme::catalog::{ColorRole, PaintRole, StyleRole, TintRole, ROLE_SPECS};

        let Self {
            id,
            generation: _,
            name,
            description,
            author,
            semantic,
            gradients,
            gradient_names,
            components: _,
        } = self;
        let head = id == &other.id
            && name == &other.name
            && description == &other.description
            && author == &other.author
            && semantic == &other.semantic
            && gradients == &other.gradients
            && gradient_names == &other.gradient_names;
        if !head {
            return false;
        }

        ROLE_SPECS.iter().all(|spec| match spec.role {
            crate::theme::catalog::RoleRef::Color(role) => {
                let _: ColorRole = role;
                self.color(role) == other.color(role)
            }
            crate::theme::catalog::RoleRef::Style(role) => {
                let _: StyleRole = role;
                self.style(role) == other.style(role)
            }
            crate::theme::catalog::RoleRef::Tint(role) => {
                let _: TintRole = role;
                self.tint(role) == other.tint(role)
            }
            crate::theme::catalog::RoleRef::Paint(role) => {
                let _: PaintRole = role;
                match (self.paint(role), other.paint(role)) {
                    (ResolvedPaint::Solid(a), ResolvedPaint::Solid(b)) => a == b,
                    (ResolvedPaint::Gradient(a), ResolvedPaint::Gradient(b)) => {
                        // Each id is read by the theme that minted it; equal
                        // names over an already-equal table means equal
                        // gradients.
                        let (Some(a), Some(b)) = (self.gradient_name(*a), other.gradient_name(*b))
                        else {
                            return false;
                        };
                        a == b
                    }
                    _ => false,
                }
            }
        })
    }

    /// Whether `id` was minted by this theme's own resolve run.
    ///
    /// Checked *before* any index lookup: an id from another run may well be in
    /// range here, and answering it would name a different gradient under the
    /// same number — exactly the stale-id bug the generation exists to stop.
    fn owns(&self, id: GradientId) -> bool {
        id.generation() == self.generation
    }

    /// The gradient an id names. `None` means the id came from another resolve
    /// run — a different theme, or the same theme before a reload.
    pub fn gradient(&self, id: GradientId) -> Option<&ResolvedGradient> {
        if !self.owns(id) {
            return None;
        }
        self.gradients.get(id.index())
    }

    /// The name the theme file gave a gradient. Reading only — an export needs
    /// to reference `gradients.<name>` the way the author wrote it.
    pub fn gradient_name(&self, id: GradientId) -> Option<&str> {
        if !self.owns(id) {
            return None;
        }
        self.gradient_names.get(id.index()).map(String::as_str)
    }

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
            ResolvedPaint::Gradient(id) => self.checked_gradient(*id),
        }
    }

    /// A gradient this theme's own paint role points at.
    ///
    /// Missing it is a resolver bug rather than a theme the user could write,
    /// so it aborts a debug build instead of quietly discolouring a frame;
    /// release builds still fall back, because a wrong colour beats a crash in
    /// front of a user.
    fn checked_gradient(&self, id: GradientId) -> Option<&ResolvedGradient> {
        // A foreign generation and an out-of-range index are different bugs —
        // the first means an id outlived its resolve run, the second a resolver
        // that numbered past its own table — so the assertion names which.
        debug_assert!(
            self.owns(id),
            "theme `{}` paints with a gradient id from another resolve run \
             ({:?} vs {:?})",
            self.id,
            id.generation(),
            self.generation
        );
        let gradient = self.gradient(id);
        debug_assert!(
            gradient.is_some() || !self.owns(id),
            "theme `{}` paints with gradient {} of {}",
            self.id,
            id.index(),
            self.gradients.len()
        );
        gradient
    }

    /// Colour of a paint role at one cell of `area`.
    ///
    /// Solid paints ignore the position; gradients are sampled with the
    /// direction semantics of the V1 spec, where coordinates are always
    /// relative to the component rect rather than the screen.
    ///
    /// This method is **total**: every coordinate yields a colour, which is
    /// what a caller filling cells one at a time needs. Two cases therefore
    /// have deliberate anchoring rather than "no colour" — see
    /// [`anchored_position`]:
    ///
    /// - a coordinate **outside `area`** is clamped to the nearest cell inside
    ///   it, so overshooting a rect continues the edge colour;
    /// - an **interior cell of a `perimeter`** gradient, which lies off the
    ///   ring entirely, resolves to the ring's seam colour.
    ///
    /// The painters in [`crate::theme::gradient`] deliberately do the opposite
    /// and **skip** both kinds of cell. Prefer them for filling a region: a
    /// `perimeter` role is a frame, so painting its interior with this method
    /// gives a flat block of the seam colour, not a fallback.
    pub fn paint_color_at(&self, role: PaintRole, area: Rect, x: u16, y: u16) -> Color {
        match self.paint(role) {
            ResolvedPaint::Solid(color) => *color,
            ResolvedPaint::Gradient(id) => match self.checked_gradient(*id) {
                Some(gradient) => {
                    gradient.sample(anchored_position(gradient.direction, area, x, y))
                }
                None => Color::Reset,
            },
        }
    }
}

/// [`gradient_position`] made total, for callers that must have a colour for
/// every coordinate.
///
/// Clamping into the rect (rather than falling back to `0.0`) keeps a cell just
/// past an edge the same colour as the edge itself; anchoring a perimeter's
/// interior at `0.0` puts it on the seam, where the resolver guarantees the
/// first and last stop are the same colour, so the choice is not visible as an
/// arbitrary end of the ramp.
fn anchored_position(direction: GradientDirection, area: Rect, x: u16, y: u16) -> f64 {
    if area.is_empty() {
        return 0.0;
    }
    // `right()`/`bottom()` saturate, so a rect touching the end of the
    // coordinate space can report a last cell *before* its own origin. No
    // layout builds such a rect, but `Rect`'s fields are public and this
    // method is public, so the clamp must not be able to invert.
    let x = x.clamp(area.x, area.right().saturating_sub(1).max(area.x));
    let y = y.clamp(area.y, area.bottom().saturating_sub(1).max(area.y));
    gradient_position(direction, area, x, y).unwrap_or(0.0)
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

    #[test]
    fn paint_color_at_clamps_coordinates_outside_the_component_rect() {
        let area = Rect::new(10, 10, 8, 4);
        // Off every edge, the nearest cell inside the rect defines the colour,
        // so a caller that overshoots gets edge continuity rather than a jump
        // back to the start of the ramp.
        for direction in [GradientDirection::Horizontal, GradientDirection::Vertical] {
            assert_eq!(
                anchored_position(direction, area, 99, 99),
                anchored_position(direction, area, 17, 13),
                "{direction:?} past the bottom-right corner"
            );
            assert_eq!(
                anchored_position(direction, area, 0, 0),
                anchored_position(direction, area, 10, 10),
                "{direction:?} before the top-left corner"
            );
        }
        assert_eq!(
            anchored_position(GradientDirection::Horizontal, area, 99, 11),
            1.0
        );
        assert_eq!(
            anchored_position(GradientDirection::Vertical, area, 11, 99),
            1.0
        );
    }

    #[test]
    fn paint_color_at_anchors_perimeter_interiors_at_the_seam() {
        let area = Rect::new(0, 0, 5, 4);
        // Interior cells are not on the ring at all; they resolve to the seam,
        // where a perimeter's first and last stop are the same colour.
        assert_eq!(
            anchored_position(GradientDirection::Perimeter, area, 2, 1),
            0.0
        );
        assert_eq!(
            anchored_position(GradientDirection::Perimeter, area, 0, 0),
            0.0
        );
    }

    #[test]
    fn paint_color_at_survives_an_empty_rect() {
        for empty in [Rect::new(0, 0, 0, 4), Rect::new(0, 0, 4, 0)] {
            for direction in [
                GradientDirection::Horizontal,
                GradientDirection::Vertical,
                GradientDirection::DiagonalDown,
                GradientDirection::DiagonalUp,
                GradientDirection::Perimeter,
            ] {
                assert_eq!(anchored_position(direction, empty, 7, 7), 0.0);
            }
        }
    }

    #[test]
    fn a_resolved_theme_is_fully_readable_through_public_accessors() {
        // The fields are crate-private so nothing outside can build a theme
        // that breaks its own invariants; everything a CLI export or a renderer
        // reads has to stay reachable without them.
        use crate::theme::registry::{ThemeRegistry, ThemeSource};

        let registry = ThemeRegistry::builtins(ValidationMode::Compatible).expect("built-ins load");
        let record = registry.get("aqua").expect("aqua is built in");
        assert_eq!(record.source, ThemeSource::BuiltIn);
        let aqua = record.resolved().expect("aqua resolves").clone();

        assert_eq!(aqua.id().as_str(), "aqua");
        assert_eq!(aqua.name(), record.name);
        assert_eq!(aqua.description(), record.description.as_deref());
        assert_eq!(
            aqua.semantic().accent,
            aqua.semantic().slot(SemanticSlot::Accent)
        );

        let ring = aqua
            .paint_gradient(PaintRole::PopupBorder)
            .expect("aqua rings its popup border");
        let ResolvedPaint::Gradient(id) = aqua.paint(PaintRole::PopupBorder) else {
            panic!("the popup border is a gradient");
        };
        assert_eq!(aqua.gradient(*id), Some(ring));
        assert!(aqua.gradients().contains(ring));
        assert_eq!(ring.direction(), GradientDirection::Perimeter);
        let stops = ring.stops();
        assert_eq!(stops.len(), ring.stops.len());
        assert_eq!(stops[0].position(), 0.0);
        assert_eq!(stops[0].color(), ring.sample(0.0));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "gradient")]
    fn a_gradient_index_that_points_nowhere_trips_a_debug_assertion() {
        // Only reachable by corrupting a theme from inside the crate: a paint
        // role pointing past the gradient table is a resolver bug, not a theme
        // the user can write, so release builds still fall back rather than
        // abort a render.
        use crate::theme::registry::ThemeRegistry;

        let registry = ThemeRegistry::builtins(ValidationMode::Compatible).expect("built-ins load");
        let mut aqua = (*registry
            .resolved(&ThemeId::parse("aqua").expect("valid id"))
            .expect("aqua resolves"))
        .clone();
        aqua.gradients.clear();
        let _ = aqua.paint_color_at(PaintRole::PopupBorder, Rect::new(0, 0, 4, 4), 0, 0);
    }

    #[test]
    fn paint_color_at_survives_a_rect_whose_edge_saturates() {
        // `Rect`'s fields are public, so a rect reaching the end of the
        // coordinate space is constructible even though no layout produces one.
        // `right()`/`bottom()` saturate there, which would put the clamp's
        // upper bound below its lower bound.
        for area in [
            Rect {
                x: u16::MAX,
                y: 0,
                width: 5,
                height: 5,
            },
            Rect {
                x: 0,
                y: u16::MAX,
                width: 5,
                height: 5,
            },
        ] {
            for direction in [
                GradientDirection::Horizontal,
                GradientDirection::Vertical,
                GradientDirection::DiagonalDown,
                GradientDirection::DiagonalUp,
                GradientDirection::Perimeter,
            ] {
                let position = anchored_position(direction, area, 0, 0);
                assert!((0.0..=1.0).contains(&position), "{direction:?}: {position}");
            }
        }
    }

    #[test]
    fn paint_color_at_returns_the_documented_colors_for_a_real_perimeter_theme() {
        use crate::theme::registry::ThemeRegistry;

        let registry = ThemeRegistry::builtins(ValidationMode::Compatible).expect("built-ins load");
        let aqua = registry
            .resolved(&ThemeId::parse("aqua").expect("valid id"))
            .expect("aqua resolves");
        let ring = aqua
            .paint_gradient(PaintRole::PopupBorder)
            .expect("aqua rings its popup border");
        assert_eq!(ring.direction, GradientDirection::Perimeter);
        let seam = ring.stops.first().expect("a stop").color;
        let area = Rect::new(4, 4, 6, 5);

        // Ring cells carry their walk position; the top-left corner is the seam.
        assert_eq!(
            aqua.paint_color_at(PaintRole::PopupBorder, area, 4, 4),
            seam
        );
        // An interior cell and any out-of-rect cell stay defined, never panic,
        // and never return `Color::Reset`.
        let interior = aqua.paint_color_at(PaintRole::PopupBorder, area, 6, 6);
        assert_eq!(interior, seam);
        let far_away = aqua.paint_color_at(PaintRole::PopupBorder, area, 400, 400);
        assert_eq!(
            far_away,
            aqua.paint_color_at(PaintRole::PopupBorder, area, 9, 8)
        );
        assert_ne!(far_away, Color::Reset);

        // A solid role ignores the position entirely.
        let solid = aqua.paint_color_at(PaintRole::AppBackground, area, 0, 0);
        assert_eq!(
            solid,
            aqua.paint_color_at(PaintRole::AppBackground, area, 9, 9)
        );
    }
}
