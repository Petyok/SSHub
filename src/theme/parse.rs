//! Span-preserving theme file parser.
//!
//! Turns theme file text into a [`ThemeDefinition`] while remembering the byte
//! range of every value, so later stages can point a diagnostic at
//! `file:line:column`.
//!
//! The parser deliberately reports *shape* only: a value it cannot represent
//! produces a diagnostic, everything it can represent is retained — including
//! unknown keys and unknown component roles. Whether those are fatal is the
//! validator's `Strict`/`Compatible` policy, not the parser's.
//!
//! Parsing is purely in-memory: no filesystem and no environment access.

use std::ops::Range;

use toml_edit::{Array, ImDocument, InlineTable, Item, Key, Table, Value};

use crate::theme::catalog::{RoleRef, RoleSpec, ROLE_SPECS, SEMANTIC_SPECS};
use crate::theme::model::{
    ColorBase, ColorReference, ColorSlot, ColorValue, ComponentEntry, ComponentValue,
    GradientDefinition, GradientStopDefinition, ModifierList, PaintSlot, PaletteEntry,
    ReferenceScope, SemanticEntry, Spanned, StyleValue, ThemeDefinition, ThemeDiagnostic, ThemeId,
    ThemeOrigin, TintSlot, UnknownField,
};

/// Result of parsing one theme file.
///
/// A definition and diagnostics are not exclusive: everything that could be
/// represented is returned even when parts of the file were rejected, so one
/// run reports every independent problem instead of only the first.
pub struct ParseOutcome {
    pub definition: Option<ThemeDefinition>,
    pub diagnostics: Vec<ThemeDiagnostic>,
}

impl ParseOutcome {
    /// The file is not TOML at all, so there is nothing to keep.
    pub fn syntax_error(origin: ThemeOrigin, error: toml_edit::TomlError) -> Self {
        let span = error.span();
        Self {
            definition: None,
            diagnostics: vec![ThemeDiagnostic::error(
                origin,
                span,
                format!("invalid TOML: {}", error.message()),
            )],
        }
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(ThemeDiagnostic::is_error)
    }
}

/// Parse `source` as the theme `id`.
pub fn parse_theme(id: ThemeId, origin: ThemeOrigin, source: &str) -> ParseOutcome {
    // `source.parse::<DocumentMut>()` throws the spans away (`into_mut` calls
    // `despan`), so the immutable document is the only one that can back a
    // positioned diagnostic.
    let document = match ImDocument::parse(source) {
        Ok(document) => document,
        Err(error) => return ParseOutcome::syntax_error(origin, error),
    };
    DefinitionParser::new(id, origin).parse(document.as_table())
}

/// A table entry or an inline-table entry, so both spellings of every section
/// can be walked by the same code.
#[derive(Clone, Copy)]
enum Node<'t> {
    Item(&'t Item),
    Value(&'t Value),
}

impl<'t> Node<'t> {
    fn span(self) -> Option<Range<usize>> {
        match self {
            Self::Item(item) => item.span(),
            Self::Value(value) => value.span(),
        }
    }

    fn as_value(self) -> Option<&'t Value> {
        match self {
            Self::Item(Item::Value(value)) | Self::Value(value) => Some(value),
            Self::Item(_) => None,
        }
    }

    fn as_str(self) -> Option<&'t str> {
        self.as_value().and_then(Value::as_str)
    }

    fn as_bool(self) -> Option<bool> {
        self.as_value().and_then(Value::as_bool)
    }

    fn as_integer(self) -> Option<i64> {
        self.as_value().and_then(Value::as_integer)
    }

    /// TOML has no single number type; both spellings are accepted wherever
    /// the schema asks for a factor.
    fn as_number(self) -> Option<f64> {
        let value = self.as_value()?;
        value
            .as_float()
            .or_else(|| value.as_integer().map(|integer| integer as f64))
    }

    fn as_array(self) -> Option<&'t Array> {
        self.as_value().and_then(Value::as_array)
    }

    fn entries(self) -> Option<Vec<(&'t Key, Node<'t>)>> {
        match self {
            Self::Item(Item::Table(table)) => Some(table_entries(table)),
            _ => self
                .as_value()
                .and_then(Value::as_inline_table)
                .map(inline_entries),
        }
    }

    fn type_name(self) -> &'static str {
        match self {
            Self::Item(item) => item.type_name(),
            Self::Value(value) => value.type_name(),
        }
    }
}

fn table_entries(table: &Table) -> Vec<(&Key, Node<'_>)> {
    table
        .iter()
        .filter_map(|(name, item)| table.key(name).map(|key| (key, Node::Item(item))))
        .collect()
}

fn inline_entries(table: &InlineTable) -> Vec<(&Key, Node<'_>)> {
    table
        .iter()
        .filter_map(|(name, value)| table.key(name).map(|key| (key, Node::Value(value))))
        .collect()
}

/// Catalogue lookup of a full role path such as `components.footer.key`.
pub(crate) fn role_by_path(path: &str) -> Option<&'static RoleSpec> {
    ROLE_SPECS.iter().find(|spec| spec.path == path)
}

/// Whether `path` is a proper prefix of a known role, i.e. a section that has
/// to be descended into rather than reported as an unknown role.
pub(crate) fn is_role_prefix(path: &str) -> bool {
    ROLE_SPECS.iter().any(|spec| {
        spec.path.len() > path.len()
            && spec.path.as_bytes()[path.len()] == b'.'
            && spec.path.starts_with(path)
    })
}

fn is_hex_literal(raw: &str) -> bool {
    raw.len() == 7 && raw.starts_with('#') && raw[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn hex_channels(raw: &str) -> [u8; 3] {
    let byte = |index: usize| u8::from_str_radix(&raw[index..index + 2], 16).unwrap_or(0);
    [byte(1), byte(3), byte(5)]
}

/// Split a qualified reference such as `palette.deep_sea`.
fn parse_reference(raw: &str) -> Option<ColorReference> {
    let (prefix, name) = raw.split_once('.')?;
    let scope = match prefix {
        "palette" => ReferenceScope::Palette,
        "semantic" => ReferenceScope::Semantic,
        _ => return None,
    };
    if name.is_empty() || name.contains('.') {
        return None;
    }
    Some(ColorReference {
        scope,
        name: name.to_string(),
    })
}

struct DefinitionParser {
    origin: ThemeOrigin,
    diagnostics: Vec<ThemeDiagnostic>,
    definition: ThemeDefinition,
    saw_name: bool,
    saw_schema_version: bool,
}

impl DefinitionParser {
    fn new(id: ThemeId, origin: ThemeOrigin) -> Self {
        Self {
            origin: origin.clone(),
            diagnostics: Vec::new(),
            definition: ThemeDefinition {
                id,
                origin,
                schema_version: None,
                name: Spanned::new(String::new(), 0..0),
                extends: None,
                description: None,
                author: None,
                palette: Vec::new(),
                semantic: Vec::new(),
                gradients: Vec::new(),
                components: Vec::new(),
                unknown_fields: Vec::new(),
            },
            saw_name: false,
            saw_schema_version: false,
        }
    }

    fn parse(mut self, root: &Table) -> ParseOutcome {
        for (key, node) in table_entries(root) {
            match key.get() {
                "schema_version" => {
                    self.saw_schema_version = true;
                    self.definition.schema_version =
                        self.expect_integer("schema_version", key, node);
                }
                "name" => {
                    self.saw_name = true;
                    if let Some(name) = self.expect_string("name", key, node) {
                        self.definition.name = name;
                    }
                }
                "extends" => self.definition.extends = self.expect_string("extends", key, node),
                "description" => {
                    self.definition.description = self.expect_string("description", key, node)
                }
                "author" => self.definition.author = self.expect_string("author", key, node),
                "palette" => self.parse_palette(node),
                "semantic" => self.parse_semantic(node),
                "gradients" => self.parse_gradients(node),
                "components" => self.parse_components(node),
                other => self.push_unknown_field(other.to_string(), key, node),
            }
        }

        // Required metadata is reported up front so the diagnostics of a file
        // stay ordered by source position.
        let mut missing = Vec::new();
        if !self.saw_name {
            missing.push(
                ThemeDiagnostic::error(self.origin.clone(), None, "missing required field `name`")
                    .with_help("add a display name, e.g. `name = \"My Theme\"`"),
            );
        }
        if !self.saw_schema_version {
            missing.push(
                ThemeDiagnostic::error(
                    self.origin.clone(),
                    None,
                    "missing required field `schema_version`",
                )
                .with_help("schema version 1 files start with `schema_version = 1`"),
            );
        }
        self.diagnostics.splice(0..0, missing);

        ParseOutcome {
            definition: Some(self.definition),
            diagnostics: self.diagnostics,
        }
    }

    fn error(&mut self, span: Option<Range<usize>>, message: impl Into<String>) {
        self.diagnostics
            .push(ThemeDiagnostic::error(self.origin.clone(), span, message));
    }

    fn error_with_help(
        &mut self,
        span: Option<Range<usize>>,
        message: impl Into<String>,
        help: impl Into<String>,
    ) {
        self.diagnostics
            .push(ThemeDiagnostic::error(self.origin.clone(), span, message).with_help(help));
    }

    fn type_error(&mut self, path: &str, node: Node<'_>, expected: &str) {
        let found = node.type_name();
        self.error(
            node.span(),
            format!("`{path}` must be {expected}, found {found}"),
        );
    }

    fn push_unknown_field(&mut self, path: String, key: &Key, node: Node<'_>) {
        let key_span = key.span().unwrap_or(0..0);
        let value_span = node.span().unwrap_or_else(|| key_span.clone());
        self.definition.unknown_fields.push(UnknownField {
            path: Spanned::new(path, key_span),
            value_span,
        });
    }

    fn expect_string(&mut self, path: &str, key: &Key, node: Node<'_>) -> Option<Spanned<String>> {
        self.expect_string_at(path, node, key.span().unwrap_or(0..0))
    }

    /// Like [`Self::expect_string`] but for values whose key is not at hand.
    fn expect_string_at(
        &mut self,
        path: &str,
        node: Node<'_>,
        fallback: Range<usize>,
    ) -> Option<Spanned<String>> {
        match node.as_str() {
            Some(text) => Some(Spanned::new(
                text.to_string(),
                node.span().unwrap_or(fallback),
            )),
            None => {
                self.type_error(path, node, "a string");
                None
            }
        }
    }

    fn expect_integer(&mut self, path: &str, key: &Key, node: Node<'_>) -> Option<Spanned<i64>> {
        match node.as_integer() {
            Some(value) => Some(Spanned::new(
                value,
                node.span().unwrap_or_else(|| key.span().unwrap_or(0..0)),
            )),
            None => {
                self.type_error(path, node, "an integer");
                None
            }
        }
    }

    fn expect_number(&mut self, path: &str, node: Node<'_>) -> Option<Spanned<f64>> {
        match node.as_number() {
            Some(value) => Some(Spanned::new(value, node.span().unwrap_or(0..0))),
            None => {
                self.type_error(path, node, "a number");
                None
            }
        }
    }

    fn expect_entries<'t>(
        &mut self,
        path: &str,
        node: Node<'t>,
    ) -> Option<Vec<(&'t Key, Node<'t>)>> {
        match node.entries() {
            Some(entries) => Some(entries),
            None => {
                self.type_error(path, node, "a table");
                None
            }
        }
    }

    // -- sections ----------------------------------------------------------

    fn parse_palette(&mut self, node: Node<'_>) {
        let Some(entries) = self.expect_entries("palette", node) else {
            return;
        };
        for (name, value) in entries {
            let path = format!("palette.{}", name.get());
            if let Some(color) = self.parse_color(&path, value) {
                self.definition.palette.push(PaletteEntry {
                    name: Spanned::new(name.get().to_string(), name.span().unwrap_or(0..0)),
                    value: color,
                });
            }
        }
    }

    fn parse_semantic(&mut self, node: Node<'_>) {
        let Some(entries) = self.expect_entries("semantic", node) else {
            return;
        };
        for (name, value) in entries {
            let path = format!("semantic.{}", name.get());
            let Some(spec) = SEMANTIC_SPECS.iter().find(|spec| spec.key == name.get()) else {
                self.push_unknown_field(path, name, value);
                continue;
            };
            if let Some(color) = self.parse_color(&path, value) {
                self.definition.semantic.push(SemanticEntry {
                    slot: spec.slot,
                    key: Spanned::new(name.get().to_string(), name.span().unwrap_or(0..0)),
                    value: color,
                });
            }
        }
    }

    fn parse_gradients(&mut self, node: Node<'_>) {
        let Some(entries) = self.expect_entries("gradients", node) else {
            return;
        };
        for (name, value) in entries {
            self.parse_gradient(name, value);
        }
    }

    fn parse_gradient(&mut self, name: &Key, node: Node<'_>) {
        let path = format!("gradients.{}", name.get());
        let key_span = name.span().unwrap_or(0..0);
        let table_span = node.span().unwrap_or_else(|| key_span.clone());
        let Some(entries) = self.expect_entries(&path, node) else {
            return;
        };

        let mut direction = None;
        let mut stops = Vec::new();
        let mut stops_span = table_span.clone();

        for (field, value) in entries {
            match field.get() {
                "direction" => {
                    direction = self.expect_string(&format!("{path}.direction"), field, value)
                }
                "stops" => {
                    stops_span = value.span().unwrap_or_else(|| table_span.clone());
                    let Some(array) = value.as_array() else {
                        self.type_error(&format!("{path}.stops"), value, "an array");
                        continue;
                    };
                    for (index, stop) in array.iter().enumerate() {
                        if let Some(stop) = self.parse_gradient_stop(&path, index, stop) {
                            stops.push(stop);
                        }
                    }
                }
                other => self.push_unknown_field(format!("{path}.{other}"), field, value),
            }
        }

        self.definition.gradients.push(GradientDefinition {
            name: Spanned::new(name.get().to_string(), key_span),
            direction,
            stops,
            stops_span,
            span: table_span,
        });
    }

    fn parse_gradient_stop(
        &mut self,
        gradient_path: &str,
        index: usize,
        stop: &Value,
    ) -> Option<GradientStopDefinition> {
        let path = format!("{gradient_path}.stops[{index}]");
        let node = Node::Value(stop);
        let span = node.span().unwrap_or(0..0);
        let entries = self.expect_entries(&path, node)?;

        let mut at = None;
        let mut color = None;
        for (field, value) in entries {
            match field.get() {
                "at" => at = self.expect_number(&format!("{path}.at"), value),
                "color" => color = self.parse_color(&format!("{path}.color"), value),
                other => self.push_unknown_field(format!("{path}.{other}"), field, value),
            }
        }
        Some(GradientStopDefinition { at, color, span })
    }

    fn parse_components(&mut self, node: Node<'_>) {
        let Some(entries) = self.expect_entries("components", node) else {
            return;
        };
        for (name, value) in entries {
            self.walk_component(format!("components.{}", name.get()), name, value);
        }
    }

    /// Descend `[components]` until a path either names a catalogue role or
    /// cannot lead to one.
    fn walk_component(&mut self, path: String, key: &Key, node: Node<'_>) {
        if let Some(spec) = role_by_path(&path) {
            self.parse_role(path, key, spec, node);
            return;
        }
        if is_role_prefix(&path) {
            match node.entries() {
                Some(entries) => {
                    for (name, value) in entries {
                        self.walk_component(format!("{path}.{}", name.get()), name, value);
                    }
                }
                // A known section holding a scalar is a wrong shape, not an
                // unknown role — it must stay fatal in `Compatible` mode too.
                None => self.type_error(&path, node, "a table of roles"),
            }
            return;
        }
        self.push_unknown_role(path, key, node);
    }

    /// Keep a role this build does not know: its value cannot be typed, so
    /// only its span survives for the validator to point at.
    fn push_unknown_role(&mut self, path: String, key: &Key, node: Node<'_>) {
        let key_span = key.span().unwrap_or(0..0);
        let value_span = node.span().unwrap_or_else(|| key_span.clone());
        self.definition.components.push(ComponentEntry {
            path: Spanned::new(path, key_span),
            value: ComponentValue::Unknown { value_span },
        });
    }

    fn parse_role(&mut self, path: String, key: &Key, spec: &'static RoleSpec, node: Node<'_>) {
        let key_span = key.span().unwrap_or(0..0);
        let span = node.span().unwrap_or_else(|| key_span.clone());
        let value = match spec.role {
            RoleRef::Color(role) => {
                self.parse_color_slot(&path, node)
                    .map(|slot| ComponentValue::Color {
                        role,
                        value: Spanned::new(slot, span.clone()),
                    })
            }
            RoleRef::Paint(role) => {
                self.parse_paint_slot(&path, node)
                    .map(|slot| ComponentValue::Paint {
                        role,
                        value: Spanned::new(slot, span.clone()),
                    })
            }
            RoleRef::Tint(role) => {
                self.parse_tint_slot(&path, node)
                    .map(|slot| ComponentValue::Tint {
                        role,
                        value: Spanned::new(slot, span.clone()),
                    })
            }
            RoleRef::Style(role) => {
                self.parse_style(&path, node)
                    .map(|style| ComponentValue::Style {
                        role,
                        value: Box::new(Spanned::new(style, span.clone())),
                    })
            }
        };
        if let Some(value) = value {
            self.definition.components.push(ComponentEntry {
                path: Spanned::new(path, key_span),
                value,
            });
        }
    }

    fn parse_color_slot(&mut self, path: &str, node: Node<'_>) -> Option<ColorSlot> {
        if node.as_str() == Some("auto") {
            return Some(ColorSlot::Auto);
        }
        Some(ColorSlot::Color(self.parse_color(path, node)?.value))
    }

    fn parse_paint_slot(&mut self, path: &str, node: Node<'_>) -> Option<PaintSlot> {
        if node.as_str() == Some("auto") {
            return Some(PaintSlot::Auto);
        }
        // A gradient is the one component value that is not a colour, so it is
        // recognised before the colour grammar gets a chance to reject it.
        if let Some(entries) = node.entries() {
            if entries.iter().any(|(key, _)| key.get() == "gradient") {
                if entries
                    .iter()
                    .any(|(key, _)| matches!(key.get(), "color" | "rgb"))
                {
                    self.error_with_help(
                        node.span(),
                        format!("`{path}` sets both `gradient` and a colour base"),
                        "a paint has exactly one base: either `{ gradient = \"gradients.<name>\" }` or a colour",
                    );
                    return None;
                }
                let mut reference = None;
                for (key, value) in entries {
                    match key.get() {
                        "gradient" => {
                            reference =
                                self.parse_gradient_reference(&format!("{path}.gradient"), value)
                        }
                        other => self.push_unknown_field(format!("{path}.{other}"), key, value),
                    }
                }
                return reference.map(PaintSlot::Gradient);
            }
        }
        Some(PaintSlot::Color(self.parse_color(path, node)?.value))
    }

    fn parse_gradient_reference(&mut self, path: &str, node: Node<'_>) -> Option<Spanned<String>> {
        let raw = self.expect_string_at(path, node, 0..0)?;
        // Exactly one fixed `gradients.` prefix; everything after it is the raw
        // name, dots included. A dot cannot introduce a sub-hierarchy here
        // because the runtime model is a flat name → gradient map, and a name
        // written as a quoted TOML key (`[gradients."has.dot"]`) is one segment
        // by TOML's own rules — refusing to reference it made such a gradient
        // definable but unusable. A missing prefix or an empty rest stays an
        // error: neither names anything.
        match raw.value.strip_prefix("gradients.") {
            Some(name) if !name.is_empty() => Some(Spanned::new(name.to_string(), raw.span)),
            _ => {
                self.error_with_help(
                    Some(raw.span),
                    format!("`{path}` is not a gradient reference"),
                    "gradients are referenced fully qualified, e.g. `\"gradients.panel_border\"`",
                );
                None
            }
        }
    }

    fn parse_tint_slot(&mut self, path: &str, node: Node<'_>) -> Option<TintSlot> {
        match node.as_str() {
            Some("auto") => Some(TintSlot::Auto),
            Some("native") => Some(TintSlot::Native),
            _ => Some(TintSlot::Color(self.parse_color(path, node)?.value)),
        }
    }

    fn parse_style(&mut self, path: &str, node: Node<'_>) -> Option<StyleValue> {
        let entries = self.expect_entries(path, node)?;
        let mut style = StyleValue::default();
        for (key, value) in entries {
            let field_path = format!("{path}.{}", key.get());
            match key.get() {
                "auto" => match value.as_bool() {
                    Some(flag) => {
                        style.auto = Some(Spanned::new(flag, value.span().unwrap_or(0..0)))
                    }
                    None => self.type_error(&field_path, value, "a boolean"),
                },
                "foreground" => {
                    style.foreground = self.parse_style_color(&field_path, value);
                }
                "background" => {
                    style.background = self.parse_style_color(&field_path, value);
                }
                "modifiers" => style.modifiers = self.parse_modifiers(&field_path, value),
                other => self.push_unknown_field(format!("{path}.{other}"), key, value),
            }
        }
        Some(style)
    }

    fn parse_style_color(&mut self, path: &str, node: Node<'_>) -> Option<Spanned<ColorSlot>> {
        let span = node.span().unwrap_or(0..0);
        self.parse_color_slot(path, node)
            .map(|slot| Spanned::new(slot, span))
    }

    fn parse_modifiers(&mut self, path: &str, node: Node<'_>) -> Option<Spanned<ModifierList>> {
        let span = node.span().unwrap_or(0..0);
        if node.as_str() == Some("auto") {
            return Some(Spanned::new(ModifierList::Auto, span));
        }
        let Some(array) = node.as_array() else {
            self.type_error(path, node, "an array of modifier names or `\"auto\"`");
            return None;
        };
        // Names stay raw: the validator owns spelling suggestions.
        let mut names = Vec::new();
        for entry in array.iter() {
            match entry.as_str() {
                Some(name) => {
                    names.push(Spanned::new(name.to_string(), entry.span().unwrap_or(0..0)))
                }
                None => self.type_error(path, Node::Value(entry), "a modifier name"),
            }
        }
        Some(Spanned::new(ModifierList::List(names), span))
    }

    // -- colours -----------------------------------------------------------

    fn parse_color(&mut self, path: &str, node: Node<'_>) -> Option<Spanned<ColorValue>> {
        let span = node.span().unwrap_or(0..0);
        if let Some(raw) = node.as_str() {
            let base = self.parse_color_string(path, raw, &span)?;
            return Some(Spanned::new(
                ColorValue {
                    base,
                    base_span: span.clone(),
                    base_from_color_key: false,
                    brightness: None,
                    opacity: None,
                    over: None,
                },
                span,
            ));
        }

        let Some(entries) = node.entries() else {
            self.type_error(path, node, "a colour string or table");
            return None;
        };

        let mut base = None;
        let mut base_span = span.clone();
        let mut saw_rgb = false;
        let mut saw_color = false;
        let mut saw_gradient = false;
        let mut brightness = None;
        let mut opacity = None;
        let mut over = None;

        for (key, value) in entries {
            let field_path = format!("{path}.{}", key.get());
            match key.get() {
                "rgb" => {
                    saw_rgb = true;
                    if let Some(channels) = self.parse_rgb(&field_path, value) {
                        base = Some(ColorBase::Rgb(channels));
                        base_span = value.span().unwrap_or_else(|| span.clone());
                    }
                }
                "color" => {
                    saw_color = true;
                    let value_span = value.span().unwrap_or_else(|| span.clone());
                    if let Some(raw) = value.as_str() {
                        if let Some(parsed) = self.parse_color_string(&field_path, raw, &value_span)
                        {
                            base = Some(parsed);
                            base_span = value_span;
                        }
                    } else {
                        self.type_error(&field_path, value, "a colour string");
                    }
                }
                "brightness" => brightness = self.expect_number(&field_path, value),
                "opacity" => opacity = self.expect_number(&field_path, value),
                "over" => over = self.parse_color(&field_path, value).map(Box::new),
                // Recognised only to be rejected: a gradient here means the
                // author picked a role that cannot take one, and that is worth
                // saying instead of "no colour base".
                "gradient" => saw_gradient = true,
                other => self.push_unknown_field(format!("{path}.{other}"), key, value),
            }
        }

        if saw_gradient {
            self.error_with_help(
                Some(span.clone()),
                format!("`{path}` does not support gradients"),
                "gradients are only valid on paint roles such as borders and backgrounds",
            );
            return None;
        }
        if saw_rgb && saw_color {
            self.error_with_help(
                Some(span.clone()),
                format!("`{path}` sets both `rgb` and `color`"),
                "a colour has exactly one base: either `rgb` or `color`",
            );
            return None;
        }
        if !saw_rgb && !saw_color {
            self.error_with_help(
                Some(span.clone()),
                format!("`{path}` has no colour base"),
                "add either `color = \"palette.<name>\"` or `rgb = [r, g, b]`",
            );
            return None;
        }

        Some(Spanned::new(
            ColorValue {
                base: base?,
                base_span,
                base_from_color_key: saw_color,
                brightness,
                opacity,
                over,
            },
            span,
        ))
    }

    fn parse_rgb(&mut self, path: &str, node: Node<'_>) -> Option<[Spanned<i64>; 3]> {
        let Some(array) = node.as_array() else {
            self.type_error(path, node, "an array of three integers");
            return None;
        };
        if array.len() != 3 {
            self.error(
                node.span(),
                format!(
                    "`{path}` needs exactly three channels, found {}",
                    array.len()
                ),
            );
            return None;
        }
        let mut channels = Vec::with_capacity(3);
        for entry in array.iter() {
            // Ranges are the validator's job; the parser only needs integers.
            let Some(value) = entry.as_integer() else {
                self.type_error(path, Node::Value(entry), "an array of three integers");
                return None;
            };
            channels.push(Spanned::new(value, entry.span().unwrap_or(0..0)));
        }
        let mut channels = channels.into_iter();
        Some([channels.next()?, channels.next()?, channels.next()?])
    }

    fn parse_color_string(
        &mut self,
        path: &str,
        raw: &str,
        span: &Range<usize>,
    ) -> Option<ColorBase> {
        if raw == "terminal" {
            return Some(ColorBase::Terminal);
        }
        if raw.starts_with('#') {
            if is_hex_literal(raw) {
                return Some(ColorBase::Hex(hex_channels(raw)));
            }
            self.error_with_help(
                Some(span.clone()),
                format!("`{path}` is not a valid hex colour"),
                "hex colours are written as `#RRGGBB`",
            );
            return None;
        }
        if let Some(reference) = parse_reference(raw) {
            return Some(ColorBase::Reference(reference));
        }
        let help = match raw {
            "auto" => "`\"auto\"` is a reset sentinel and only valid below `[components]`",
            "native" => "`\"native\"` is only valid on tint roles",
            _ => "use `#RRGGBB`, `\"terminal\"`, `palette.<name>` or `semantic.<name>`",
        };
        self.error_with_help(
            Some(span.clone()),
            format!("`{path}` is not a colour value"),
            help,
        );
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::catalog::{PaintRole, SemanticSlot, StyleRole, TintRole};
    use crate::theme::model::*;
    use std::path::PathBuf;

    fn parse_user(source: &str) -> ParseOutcome {
        parse_theme(
            ThemeId::parse("test").unwrap(),
            ThemeOrigin::User(PathBuf::from("test.toml")),
            source,
        )
    }

    fn definition_of(source: &str) -> ThemeDefinition {
        let parsed = parse_user(source);
        assert!(
            parsed.diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            parsed.diagnostics
        );
        parsed.definition.expect("definition")
    }

    fn header(body: &str) -> String {
        format!("schema_version = 1\nname = \"T\"\n{body}")
    }

    fn palette_value<'a>(definition: &'a ThemeDefinition, name: &str) -> &'a ColorValue {
        &definition
            .palette
            .iter()
            .find(|entry| entry.name.value == name)
            .unwrap_or_else(|| panic!("palette entry {name}"))
            .value
            .value
    }

    fn component<'a>(definition: &'a ThemeDefinition, path: &str) -> &'a ComponentEntry {
        definition
            .components
            .iter()
            .find(|entry| entry.path.value == path)
            .unwrap_or_else(|| panic!("component entry {path}"))
    }

    #[test]
    fn parses_minimal_theme_and_retains_source_spans() {
        let source = "schema_version = 1\nname = \"Ocean\"\n";
        let parsed = parse_theme(
            ThemeId::parse("ocean").unwrap(),
            ThemeOrigin::User(PathBuf::from("ocean.toml")),
            source,
        );
        let definition = parsed.definition.expect("definition");
        assert_eq!(definition.name.value, "Ocean");
        assert_eq!(&source[definition.name.span.clone()], "\"Ocean\"");
        assert!(parsed.diagnostics.is_empty());
    }

    #[test]
    fn collects_unknown_fields_and_multiple_shape_errors() {
        let parsed = parse_user(
            "schema_version = 1\nname = \"Bad\"\nunknown = 1\n\
             [semantic]\naccent = { rgb = [1, 2], color = \"palette.x\" }\n\
             [components.footer]\nbordr = \"#ffffff\"\n",
        );
        assert!(parsed.diagnostics.len() >= 2);
        assert_eq!(parsed.definition.unwrap().unknown_fields.len(), 1);
    }

    #[test]
    fn parses_all_metadata_fields_with_spans() {
        let source = "schema_version = 1\nname = \"Ocean\"\nextends = \"default\"\n\
                      description = \"Deep\"\nauthor = \"someone\"\n";
        let definition = definition_of(source);
        assert_eq!(definition.schema_version.as_ref().unwrap().value, 1);
        assert_eq!(definition.extends.as_ref().unwrap().value, "default");
        assert_eq!(definition.description.as_ref().unwrap().value, "Deep");
        assert_eq!(definition.author.as_ref().unwrap().value, "someone");
        let span = definition.extends.as_ref().unwrap().span.clone();
        assert_eq!(&source[span], "\"default\"");
    }

    #[test]
    fn missing_required_metadata_is_reported() {
        let parsed = parse_user("description = \"nothing else\"\n");
        assert_eq!(
            parsed.diagnostics.iter().filter(|d| d.is_error()).count(),
            2
        );
        let definition = parsed.definition.expect("definition");
        assert_eq!(definition.name.value, "");
        assert!(definition.schema_version.is_none());
    }

    #[test]
    fn parses_hex_rgb_reference_and_terminal_colors() {
        let source = header(
            "[palette]\ndeep_sea = \"#08202a\"\nwarning = { rgb = [245, 180, 60] }\n\
             [semantic]\nbackground = \"terminal\"\naccent = \"palette.deep_sea\"\n\
             text = \"semantic.accent\"\n",
        );
        let definition = definition_of(&source);

        assert_eq!(
            palette_value(&definition, "deep_sea").base,
            ColorBase::Hex([0x08, 0x20, 0x2a])
        );
        let ColorBase::Rgb(channels) = &palette_value(&definition, "warning").base else {
            panic!("rgb base");
        };
        assert_eq!(
            channels.iter().map(|c| c.value).collect::<Vec<_>>(),
            vec![245, 180, 60]
        );
        assert_eq!(&source[channels[0].span.clone()], "245");

        let background = &definition.semantic[0];
        assert_eq!(background.slot, SemanticSlot::Background);
        assert_eq!(background.value.value.base, ColorBase::Terminal);
        assert_eq!(&source[background.key.span.clone()], "background");

        assert_eq!(
            definition.semantic[1].value.value.base,
            ColorBase::Reference(ColorReference {
                scope: ReferenceScope::Palette,
                name: "deep_sea".to_string(),
            })
        );
        assert_eq!(
            definition.semantic[2].value.value.base,
            ColorBase::Reference(ColorReference {
                scope: ReferenceScope::Semantic,
                name: "accent".to_string(),
            })
        );
    }

    #[test]
    fn parses_brightness_opacity_and_over_transforms() {
        let source = header(
            "[palette]\nsurface = { color = \"palette.deep_sea\", brightness = -0.12 }\n\
             soft = { color = \"palette.accent\", opacity = 0.35, over = \"palette.surface\" }\n",
        );
        let definition = definition_of(&source);

        let surface = palette_value(&definition, "surface");
        assert_eq!(surface.brightness.as_ref().unwrap().value, -0.12);
        assert!(surface.opacity.is_none());
        assert_eq!(&source[surface.base_span.clone()], "\"palette.deep_sea\"");

        let soft = palette_value(&definition, "soft");
        assert_eq!(soft.opacity.as_ref().unwrap().value, 0.35);
        let over = soft.over.as_ref().expect("over");
        assert_eq!(
            over.value.base,
            ColorBase::Reference(ColorReference {
                scope: ReferenceScope::Palette,
                name: "surface".to_string(),
            })
        );
        assert_eq!(&source[over.span.clone()], "\"palette.surface\"");
    }

    #[test]
    fn rejects_malformed_color_shapes_and_keeps_going() {
        let parsed = parse_user(&header(
            "[palette]\na = \"reddish\"\nb = \"#abc\"\nc = { rgb = [1, 2, \"x\"] }\n\
             d = \"auto\"\ne = { color = \"palette.a\", tint = 1 }\n",
        ));
        // Four independent shape errors, one retained unknown key (`tint`).
        assert_eq!(
            parsed.diagnostics.iter().filter(|d| d.is_error()).count(),
            4
        );
        let definition = parsed.definition.expect("definition");
        assert_eq!(definition.unknown_fields.len(), 1);
        assert_eq!(definition.unknown_fields[0].path.value, "palette.e.tint");
        // Only the well-shaped entry survives.
        assert_eq!(definition.palette.len(), 1);
        assert_eq!(definition.palette[0].name.value, "e");
    }

    #[test]
    fn parses_partial_styles_and_modifiers() {
        let source = header(
            "[components.footer.key]\nforeground = \"#f8fff9\"\n\
             background = \"semantic.accent\"\nmodifiers = [\"bold\", \"italic\"]\n\
             [components.footer.label]\nbackground = \"auto\"\nmodifiers = []\n\
             [components.text.primary]\nmodifiers = \"auto\"\n",
        );
        let definition = definition_of(&source);

        let ComponentValue::Style { role, value } =
            &component(&definition, "components.footer.key").value
        else {
            panic!("style value");
        };
        assert_eq!(*role, StyleRole::FooterKey);
        assert!(value.value.auto.is_none());
        assert!(matches!(
            value.value.foreground.as_ref().unwrap().value,
            ColorSlot::Color(_)
        ));
        let ModifierList::List(modifiers) = &value.value.modifiers.as_ref().unwrap().value else {
            panic!("modifier list");
        };
        assert_eq!(
            modifiers
                .iter()
                .map(|m| m.value.as_str())
                .collect::<Vec<_>>(),
            vec!["bold", "italic"]
        );
        assert_eq!(&source[modifiers[0].span.clone()], "\"bold\"");

        let ComponentValue::Style { value, .. } =
            &component(&definition, "components.footer.label").value
        else {
            panic!("style value");
        };
        assert!(matches!(
            value.value.background.as_ref().unwrap().value,
            ColorSlot::Auto
        ));
        assert!(matches!(
            value.value.modifiers.as_ref().unwrap().value,
            ModifierList::List(ref list) if list.is_empty()
        ));

        let ComponentValue::Style { value, .. } =
            &component(&definition, "components.text.primary").value
        else {
            panic!("style value");
        };
        assert!(matches!(
            value.value.modifiers.as_ref().unwrap().value,
            ModifierList::Auto
        ));
    }

    #[test]
    fn parses_whole_style_reset() {
        let definition = definition_of(&header("[components.footer]\nkey = { auto = true }\n"));
        let ComponentValue::Style { value, .. } =
            &component(&definition, "components.footer.key").value
        else {
            panic!("style value");
        };
        assert!(value.value.auto.as_ref().unwrap().value);
        assert!(value.value.foreground.is_none());
    }

    /// A quoted gradient key is one TOML segment, and a reference to it is one
    /// `gradients.` prefix plus the whole raw name.
    ///
    /// Dots included: the runtime model is a flat name → gradient map, so a dot
    /// introduces no sub-hierarchy. Rejecting it made a gradient that TOML
    /// itself accepts as a single key definable but permanently unreferenceable.
    #[test]
    fn a_quoted_gradient_key_can_be_referenced_verbatim() {
        const NAME: &str = "odd .\"\\ name";
        let source = header(
            "[gradients.\"odd .\\\"\\\\ name\"]\ndirection = \"horizontal\"\n\
             stops = [ { at = 0.0, color = \"#102030\" }, { at = 1.0, color = \"#405060\" } ]\n\
             [components.app]\nbackground = { gradient = \"gradients.odd .\\\"\\\\ name\" }\n",
        );
        let definition = definition_of(&source);

        assert_eq!(definition.gradients[0].name.value, NAME);
        let ComponentValue::Paint { value, .. } =
            &component(&definition, "components.app.background").value
        else {
            panic!("paint value");
        };
        let PaintSlot::Gradient(reference) = &value.value else {
            panic!("gradient reference");
        };
        assert_eq!(
            reference.value, NAME,
            "the reference must carry the whole name after the one fixed prefix"
        );
    }

    /// The two shapes that still are not references: a different prefix, and a
    /// prefix with nothing after it. Neither names a gradient.
    #[test]
    fn a_gradient_reference_needs_the_prefix_and_a_nonempty_name() {
        for bad in ["gradients.", "gradient.g", "g", "", "Gradients.g"] {
            let source = header(&format!(
                "[gradients.g]\ndirection = \"horizontal\"\n\
                 stops = [ {{ at = 0.0, color = \"#102030\" }}, {{ at = 1.0, color = \"#405060\" }} ]\n\
                 [components.app]\nbackground = {{ gradient = \"{bad}\" }}\n"
            ));
            let parsed = parse_user(&source);
            assert!(
                parsed
                    .diagnostics
                    .iter()
                    .any(|d| d.message.contains("is not a gradient reference")),
                "`{bad}` was accepted as a reference: {:?}",
                parsed.diagnostics
            );
        }
    }

    #[test]
    fn parses_color_paint_and_tint_slots() {
        let source = header(
            "[gradients.panel_border]\ndirection = \"perimeter\"\n\
             stops = [ { at = 0.0, color = \"semantic.accent\" }, { at = 1.0, color = \"#a0ffe0\" } ]\n\
             [components.os_logo]\ntint = \"native\"\n\
             [components.dashboard.host_list]\nborder = { gradient = \"gradients.panel_border\" }\n\
             background = \"terminal\"\n\
             [components.app]\nbackground = \"auto\"\n",
        );
        let definition = definition_of(&source);

        let gradient = &definition.gradients[0];
        assert_eq!(gradient.name.value, "panel_border");
        assert_eq!(gradient.direction.as_ref().unwrap().value, "perimeter");
        assert_eq!(gradient.stops.len(), 2);
        assert_eq!(gradient.stops[0].at.as_ref().unwrap().value, 0.0);
        assert_eq!(
            gradient.stops[1].color.as_ref().unwrap().value.base,
            ColorBase::Hex([0xa0, 0xff, 0xe0])
        );

        let ComponentValue::Tint { role, value } =
            &component(&definition, "components.os_logo.tint").value
        else {
            panic!("tint value");
        };
        assert_eq!(*role, TintRole::OsLogoTint);
        assert!(matches!(value.value, TintSlot::Native));

        let ComponentValue::Paint { role, value } =
            &component(&definition, "components.dashboard.host_list.border").value
        else {
            panic!("paint value");
        };
        assert_eq!(*role, PaintRole::DashboardHostListBorder);
        let PaintSlot::Gradient(reference) = &value.value else {
            panic!("gradient reference");
        };
        assert_eq!(reference.value, "panel_border");
        assert_eq!(
            &source[reference.span.clone()],
            "\"gradients.panel_border\""
        );

        let ComponentValue::Paint { value, .. } =
            &component(&definition, "components.dashboard.host_list.background").value
        else {
            panic!("paint value");
        };
        assert!(matches!(
            value.value,
            PaintSlot::Color(ColorValue {
                base: ColorBase::Terminal,
                ..
            })
        ));

        let ComponentValue::Paint { value, .. } =
            &component(&definition, "components.app.background").value
        else {
            panic!("paint value");
        };
        assert!(matches!(value.value, PaintSlot::Auto));
    }

    #[test]
    fn keeps_unknown_component_roles_without_deciding_severity() {
        let source = header("[components.footer]\nbordr = \"#ffffff\"\n[components.nope]\nx = 1\n");
        let parsed = parse_user(&source);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let definition = parsed.definition.expect("definition");
        assert!(definition.unknown_fields.is_empty());

        let unknown: Vec<_> = definition
            .components
            .iter()
            .filter(|entry| !entry.is_known())
            .map(|entry| entry.path.value.as_str())
            .collect();
        assert_eq!(unknown, vec!["components.footer.bordr", "components.nope"]);

        let ComponentValue::Unknown { value_span } =
            &component(&definition, "components.footer.bordr").value
        else {
            panic!("unknown value");
        };
        assert_eq!(&source[value_span.clone()], "\"#ffffff\"");
    }

    #[test]
    fn scalar_under_a_known_section_is_a_shape_error() {
        // Known sections holding a scalar must not be downgraded to "unknown
        // role", which `Compatible` mode would silently ignore.
        let parsed = parse_user(&header("[components]\nfooter = 5\ndashboard = \"x\"\n"));
        let errors: Vec<_> = parsed.diagnostics.iter().filter(|d| d.is_error()).collect();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors[0].message.contains("components.footer"),
            "{errors:?}"
        );
        let definition = parsed.definition.expect("definition");
        assert!(definition.components.is_empty());
        assert!(definition.unknown_fields.is_empty());
    }

    #[test]
    fn gradient_on_a_non_paint_role_names_the_real_problem() {
        let parsed = parse_user(&header(
            "[components.footer.key]\nforeground = { gradient = \"gradients.g\" }\n\
             [components.os_logo]\ntint = { gradient = \"gradients.g\" }\n",
        ));
        let errors: Vec<_> = parsed.diagnostics.iter().filter(|d| d.is_error()).collect();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors
                .iter()
                .all(|d| d.message.contains("does not support gradients")),
            "{errors:?}"
        );
        let definition = parsed.definition.expect("definition");
        assert!(definition.unknown_fields.is_empty());
        // The tint role is dropped; the style survives without a foreground.
        let paths: Vec<_> = definition
            .components
            .iter()
            .map(|entry| entry.path.value.as_str())
            .collect();
        assert_eq!(paths, vec!["components.footer.key"]);
        let ComponentValue::Style { value, .. } =
            &component(&definition, "components.footer.key").value
        else {
            panic!("style value");
        };
        assert!(value.value.foreground.is_none());
    }

    #[test]
    fn gradient_and_colour_base_on_a_paint_role_conflict() {
        let parsed = parse_user(&header(
            "[components.dashboard.host_list]\n\
             border = { gradient = \"gradients.g\", color = \"semantic.accent\" }\n",
        ));
        let errors: Vec<_> = parsed.diagnostics.iter().filter(|d| d.is_error()).collect();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0]
                .message
                .contains("sets both `gradient` and a colour base"),
            "{errors:?}"
        );
        let definition = parsed.definition.expect("definition");
        assert!(definition.components.is_empty());
        assert!(definition.unknown_fields.is_empty());
    }

    #[test]
    fn spans_survive_non_ascii_content() {
        // Spans are byte ranges, so multi-byte characters ahead of a value
        // must shift it by bytes rather than by characters.
        let source = "schema_version = 1\nname = \"Océan ✨\"\ndescription = \"Tiefsee — grün\"\n\
                      [palette]\naccent = \"#0a1b2c\"\n";
        let definition = definition_of(source);
        assert_eq!(definition.name.value, "Océan ✨");
        assert_eq!(&source[definition.name.span.clone()], "\"Océan ✨\"");
        let description = definition.description.as_ref().unwrap();
        assert_eq!(&source[description.span.clone()], "\"Tiefsee — grün\"");
        let entry = &definition.palette[0];
        assert_eq!(&source[entry.name.span.clone()], "accent");
        assert_eq!(&source[entry.value.span.clone()], "\"#0a1b2c\"");
        assert_eq!(&source[entry.value.value.base_span.clone()], "\"#0a1b2c\"");
    }

    #[test]
    fn spans_survive_crlf_line_endings() {
        let source = "schema_version = 1\r\nname = \"Ocean\"\r\n[semantic]\r\n\
                      accent = \"palette.x\"\r\ntext = \"nope\"\r\n";
        let parsed = parse_user(source);
        let definition = parsed.definition.expect("definition");
        assert_eq!(&source[definition.name.span.clone()], "\"Ocean\"");
        let accent = &definition.semantic[0];
        assert_eq!(&source[accent.key.span.clone()], "accent");
        assert_eq!(&source[accent.value.span.clone()], "\"palette.x\"");
        let span = parsed.diagnostics[0].span.clone().expect("span");
        assert_eq!(&source[span], "\"nope\"");
    }

    #[test]
    fn reports_unknown_sections_and_semantic_keys_without_dropping_the_rest() {
        let source =
            header("[bogus]\nx = 1\n[semantic]\naccnt = \"#ffffff\"\naccent = \"#000000\"\n");
        let parsed = parse_user(&source);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let definition = parsed.definition.expect("definition");
        let unknown: Vec<_> = definition
            .unknown_fields
            .iter()
            .map(|field| field.path.value.as_str())
            .collect();
        assert_eq!(unknown, vec!["bogus", "semantic.accnt"]);
        assert_eq!(definition.semantic.len(), 1);
        assert_eq!(definition.semantic[0].slot, SemanticSlot::Accent);
    }

    #[test]
    fn reports_malformed_toml_with_its_source_position() {
        let source = "schema_version = 1\nname = \"Broken\n";
        let parsed = parse_user(source);
        assert!(parsed.definition.is_none());
        assert_eq!(parsed.diagnostics.len(), 1);
        let diagnostic = &parsed.diagnostics[0];
        assert!(diagnostic.is_error());
        let span = diagnostic.span.clone().expect("span");
        assert!(span.start >= "schema_version = 1\n".len(), "{span:?}");
        assert!(span.end <= source.len());
    }

    #[test]
    fn reports_wrong_toml_types_for_known_sections() {
        let parsed = parse_user("schema_version = \"1\"\nname = 7\npalette = 3\ngradients = 5\n");
        assert_eq!(
            parsed.diagnostics.iter().filter(|d| d.is_error()).count(),
            4
        );
        let definition = parsed.definition.expect("definition");
        assert!(definition.palette.is_empty());
        assert!(definition.gradients.is_empty());
        assert!(definition.unknown_fields.is_empty());
    }

    #[test]
    fn reports_gradient_shape_errors_per_stop() {
        let parsed = parse_user(&header(
            "[gradients.bad]\ndirection = 1\nstops = [ { at = \"x\", color = \"#ffffff\" }, 3 ]\n",
        ));
        assert_eq!(
            parsed.diagnostics.iter().filter(|d| d.is_error()).count(),
            3
        );
        let definition = parsed.definition.expect("definition");
        let gradient = &definition.gradients[0];
        assert!(gradient.direction.is_none());
        // The malformed stop entry is dropped, the salvageable one is kept.
        assert_eq!(gradient.stops.len(), 1);
        assert!(gradient.stops[0].at.is_none());
        assert!(gradient.stops[0].color.is_some());
    }

    #[test]
    fn diagnostics_sort_by_source_position() {
        let parsed = parse_user(&header(
            "[palette]\nb = \"nope\"\n[semantic]\naccent = \"nope\"\n",
        ));
        let mut sorted = parsed.diagnostics.clone();
        sorted.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        assert_eq!(sorted, parsed.diagnostics);
        assert!(sorted
            .iter()
            .all(|d| d.origin == ThemeOrigin::User(PathBuf::from("test.toml"))));
    }
}
