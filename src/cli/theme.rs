//! The headless `sshub theme check|list|show` commands.
//!
//! These are the only CLI commands that run entirely without a database: they
//! are dispatched in `main.rs` before `CliContext::bootstrap`, because a theme
//! author validating a draft has no reason to open the launcher and metadata
//! databases. Nothing here writes user state either — no `config.toml`, no
//! activation, no file in the themes directory.
//!
//! All validation lives in `crate::theme`; this module only chooses the entry
//! point (`Strict` for `check`, `Compatible` wherever the output mirrors what
//! the running app would do) and renders the result.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::Result;
use ratatui::style::{Color, Style};
use serde::Serialize;

use crate::config;
use crate::theme::catalog::{RoleFallback, RoleRef, SemanticTint, ROLE_SPECS, SEMANTIC_SPECS};
use crate::theme::model::{
    modifier_from_key, semantic_style, DiagnosticSeverity, GradientDirection, ResolvedGradient,
    ResolvedPaint, ResolvedTheme, ResolvedTint, ThemeDiagnostic, ValidationMode, MODIFIER_KEYS,
};
use crate::theme::registry::{ThemeRecord, ThemeRegistry, ThemeSource};

use super::help;
use super::parse::{parse_format, OutputFormat};

/// `theme show` writes a document, not a report, so it has its own format set.
/// `plain` is deliberately absent, and `toml` is deliberately not added to the
/// shared [`OutputFormat`] — its matches are exhaustive across the whole CLI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThemeShowFormat {
    Toml,
    Json,
}

pub fn run(args: &[String]) -> Result<i32> {
    match args.first().map(String::as_str) {
        Some("check") => run_check(&args[1..]),
        Some("list") => run_list(&args[1..]),
        Some("show") => run_show(&args[1..]),
        Some("--help") | Some("-h") => {
            help::print_theme_help(None);
            Ok(0)
        }
        Some(other) => Ok(usage_error(&format!("unknown theme subcommand '{other}'"))),
        None => Ok(usage_error("theme needs check, list, or show")),
    }
}

fn usage_error(msg: &str) -> i32 {
    eprintln!("sshub: {msg}");
    eprintln!("       run `sshub theme --help` for usage");
    2
}

fn failure(msg: &str) -> i32 {
    eprintln!("sshub: {msg}");
    1
}

fn wants_help(args: &[String]) -> bool {
    args.iter().any(|a| a == "--help" || a == "-h")
}

/// The themes directory the running app would read.
///
/// Resolved through [`config::config_dir_path`], never `config_dir()`: every
/// caller here only reads, and asking which themes are installed must not
/// create `~/.config/sshub` or migrate a legacy tree into it.
fn installed_themes_dir() -> Result<PathBuf, String> {
    config::config_dir_path()
        .map(|dir| dir.join("themes"))
        .map_err(|e| format!("no config directory ({e})"))
}

// ---------------------------------------------------------------------------
// theme check
// ---------------------------------------------------------------------------

fn run_check(args: &[String]) -> Result<i32> {
    if wants_help(args) {
        help::print_theme_help(Some("check"));
        return Ok(0);
    }
    let format = match parse_format(args) {
        Ok(format) => format,
        Err(message) => return Ok(usage_error(&message)),
    };
    let positionals = match positionals_without_options(args) {
        Ok(positionals) => positionals,
        Err(code) => return Ok(code),
    };
    let file = match positionals.as_slice() {
        [file] => PathBuf::from(file),
        [] => return Ok(usage_error("theme check needs a file")),
        _ => return Ok(usage_error("theme check takes exactly one file")),
    };

    // Strict: the checker is where an unknown role is a red build, not a
    // runtime compatibility note.
    let registry = match ThemeRegistry::load_check_target(&file, ValidationMode::Strict) {
        Ok(registry) => registry,
        Err(error) => return Ok(failure(&error.to_string())),
    };
    let stem = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let Some(record) = registry.get(stem) else {
        return Ok(failure(&format!(
            "{}: the checked file did not produce a theme",
            file.display()
        )));
    };

    let sources = source_texts(&registry);
    let diagnostics: Vec<DiagnosticJson> = record
        .diagnostics
        .iter()
        .map(|d| DiagnosticJson::new(d, &sources))
        .collect();
    let summary = CheckSummary::of(record);

    match format {
        OutputFormat::Plain => {
            for diagnostic in &diagnostics {
                print!("{}", diagnostic.render());
            }
            println!("{}", summary.render(record));
        }
        OutputFormat::Json => {
            let out = CheckJson {
                id: record.id.to_string(),
                path: file.display().to_string(),
                valid: record.is_valid(),
                extends: summary.parent.clone(),
                colors: summary.colors,
                gradients: summary.gradients,
                overrides: summary.overrides,
                diagnostics,
            };
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
    }

    // Warnings — including the reserved-id note and the sibling-parent note —
    // are not failures: the spec's exit code 0 covers "valid, possibly with
    // warnings".
    Ok(if record.is_valid() { 0 } else { 1 })
}

/// The numbers behind the spec's success line.
struct CheckSummary {
    parent: Option<String>,
    colors: usize,
    gradients: usize,
    overrides: usize,
}

impl CheckSummary {
    fn of(record: &ThemeRecord) -> Self {
        let parent = record.inheritance_chain().get(1).map(ToString::to_string);
        let (colors, gradients, overrides) = match record.resolved() {
            Some(theme) => (
                SEMANTIC_SPECS.len(),
                theme.gradients().len(),
                override_count(theme),
            ),
            None => (0, 0, 0),
        };
        Self {
            parent,
            colors,
            gradients,
            overrides,
        }
    }

    fn render(&self, record: &ThemeRecord) -> String {
        if !record.is_valid() {
            let errors = record.diagnostics.iter().filter(|d| d.is_error()).count();
            let warnings = record.diagnostics.iter().filter(|d| d.is_warning()).count();
            return format!(
                "FAILED: {} — {errors} error(s), {warnings} warning(s)",
                record.id
            );
        }
        let extends = match &self.parent {
            Some(parent) => format!(" (extends {parent})"),
            None => String::new(),
        };
        format!(
            "OK: {}{extends}, {} colors, {} gradients, {} overrides",
            record.id, self.colors, self.gradients, self.overrides
        )
    }
}

/// How many component roles carry a value of their own rather than their
/// semantic fallback. Derived from the resolved theme so inherited overrides
/// count exactly as the running app would see them.
fn override_count(theme: &ResolvedTheme) -> usize {
    ROLE_SPECS
        .iter()
        .filter(|spec| match (spec.role, spec.fallback) {
            (RoleRef::Color(role), RoleFallback::Color(slot)) => {
                theme.color(role) != theme.semantic().slot(slot)
            }
            (RoleRef::Style(role), RoleFallback::Style(recipe)) => {
                theme.style(role) != semantic_style(theme.semantic(), recipe)
            }
            (RoleRef::Paint(role), RoleFallback::Paint(slot)) => {
                *theme.paint(role) != ResolvedPaint::Solid(theme.semantic().slot(slot))
            }
            (RoleRef::Tint(role), RoleFallback::Tint(fallback)) => {
                let fallback = match fallback {
                    SemanticTint::Native => ResolvedTint::Native,
                    SemanticTint::Color(slot) => ResolvedTint::Color(theme.semantic().slot(slot)),
                };
                *theme.tint(role) != fallback
            }
            _ => false,
        })
        .count()
}

// ---------------------------------------------------------------------------
// theme list
// ---------------------------------------------------------------------------

fn run_list(args: &[String]) -> Result<i32> {
    if wants_help(args) {
        help::print_theme_help(Some("list"));
        return Ok(0);
    }
    let format = match parse_format(args) {
        Ok(format) => format,
        Err(message) => return Ok(usage_error(&message)),
    };
    match positionals_without_options(args) {
        Ok(positionals) if positionals.is_empty() => {}
        Ok(positionals) => {
            return Ok(usage_error(&format!(
                "theme list takes no arguments, found '{}'",
                positionals[0]
            )))
        }
        Err(code) => return Ok(code),
    }

    let themes_dir = match installed_themes_dir() {
        Ok(dir) => dir,
        Err(message) => return Ok(failure(&message)),
    };
    // Compatible: `theme list` reports the registry the app itself would build.
    let registry = match ThemeRegistry::load_installed(&themes_dir, ValidationMode::Compatible) {
        Ok(registry) => registry,
        Err(error) => return Ok(failure(&error.to_string())),
    };

    let sources = source_texts(&registry);
    // Every record, never `get()`: a user file squatting a reserved id is never
    // canonical, so `get()` would hide the very file the user is puzzled about.
    let themes: Vec<ThemeEntryJson> = registry
        .records()
        .iter()
        .map(|record| ThemeEntryJson::new(record, &sources))
        .collect();
    // Directory-level problems are warnings, and they are what explains a theme
    // missing from the list entirely — never filtered to errors.
    let directory: Vec<DiagnosticJson> = registry
        .diagnostics()
        .iter()
        .map(|d| DiagnosticJson::new(d, &sources))
        .collect();

    match format {
        OutputFormat::Plain => {
            // `id`, `name` and `source` are all user controlled, so the table is
            // built from escaped copies — and the column widths are measured on
            // those, since an escape sequence is wider than the byte it stood
            // for and measuring the raw value would skew every following column.
            let rows: Vec<(String, String, &str, String)> = themes
                .iter()
                .map(|t| {
                    (
                        sanitize_plain(&t.id),
                        sanitize_plain(&t.name),
                        t.state,
                        sanitize_plain(&t.source),
                    )
                })
                .collect();
            let width = rows
                .iter()
                .map(|(id, ..)| id.len())
                .max()
                .unwrap_or(2)
                .max(2);
            let name_width = rows
                .iter()
                .map(|(_, name, ..)| name.len())
                .max()
                .unwrap_or(4)
                .max(4);
            println!(
                "{:<width$}  {:<name_width$}  {:<9}  SOURCE",
                "ID", "NAME", "STATE"
            );
            for (id, name, state, source) in &rows {
                println!("{id:<width$}  {name:<name_width$}  {state:<9}  {source}");
            }
            let diagnostics: Vec<&DiagnosticJson> = directory
                .iter()
                .chain(themes.iter().flat_map(|t| t.diagnostics.iter()))
                .collect();
            if !diagnostics.is_empty() {
                println!();
                for diagnostic in diagnostics {
                    print!("{}", diagnostic.render());
                }
            }
        }
        OutputFormat::Json => {
            let out = ListJson {
                themes_dir: themes_dir.display().to_string(),
                themes,
                diagnostics: directory,
            };
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
    }

    // A readable registry is always a success, whatever the state of the themes
    // inside it — their state is the output.
    Ok(0)
}

// ---------------------------------------------------------------------------
// theme show
// ---------------------------------------------------------------------------

fn run_show(args: &[String]) -> Result<i32> {
    if wants_help(args) {
        help::print_theme_help(Some("show"));
        return Ok(0);
    }
    let format = match parse_show_format(args) {
        Ok(format) => format,
        Err(message) => return Ok(usage_error(&message)),
    };
    let mut resolved = false;
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--format" => index += 2,
            "--resolved" => {
                resolved = true;
                index += 1;
            }
            other if other.starts_with('-') => {
                return Ok(usage_error(&format!("unknown option '{other}'")))
            }
            other => {
                positionals.push(other.to_string());
                index += 1;
            }
        }
    }
    let id = match positionals.as_slice() {
        [id] => id.clone(),
        [] => return Ok(usage_error("theme show needs a theme id")),
        _ => return Ok(usage_error("theme show takes exactly one theme id")),
    };

    let themes_dir = match installed_themes_dir() {
        Ok(dir) => dir,
        Err(message) => return Ok(failure(&message)),
    };
    let registry = match ThemeRegistry::load_installed(&themes_dir, ValidationMode::Compatible) {
        Ok(registry) => registry,
        Err(error) => return Ok(failure(&error.to_string())),
    };
    let Some(record) = registry.get(&id) else {
        return Ok(failure(&format!(
            "unknown theme '{id}'; run `sshub theme list` for the installed ones"
        )));
    };

    if resolved {
        let Some(theme) = record.resolved() else {
            return Ok(failure(&format!(
                "theme '{id}' does not resolve; run `sshub theme list` for the reason"
            )));
        };
        match format {
            ThemeShowFormat::Toml => print!("{}", resolved_toml(theme)),
            ThemeShowFormat::Json => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&ResolvedJson::new(theme))?
                )
            }
        }
        return Ok(0);
    }

    if !record.is_valid() {
        return Ok(failure(&format!(
            "theme '{id}' is invalid; run `sshub theme list` for the reason"
        )));
    }

    match format {
        ThemeShowFormat::Toml => {
            // The file verbatim, never re-serialised: the comments are the
            // greater half of what makes a built-in worth copying.
            println!(
                "# copied from theme '{id}'; change `name` before installing under a new filename"
            );
            print!("{}", record.toml_source);
        }
        ThemeShowFormat::Json => {
            let out = ShowJson {
                id: record.id.to_string(),
                name: record.name.clone(),
                description: record.description.clone(),
                source: source_label(record),
                state: state_label(record),
                toml_source: record.toml_source.clone(),
            };
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
    }
    Ok(0)
}

fn parse_show_format(args: &[String]) -> Result<ThemeShowFormat, String> {
    let mut format = ThemeShowFormat::Toml;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--format" {
            let value = args.get(index + 1).ok_or("--format requires a value")?;
            format = match value.as_str() {
                "toml" => ThemeShowFormat::Toml,
                "json" => ThemeShowFormat::Json,
                other => {
                    return Err(format!(
                        "unknown format '{other}'; `theme show` writes toml or json"
                    ))
                }
            };
            index += 2;
        } else {
            index += 1;
        }
    }
    Ok(format)
}

/// Collect positional arguments, rejecting any option this command does not
/// define. `--format VALUE` is consumed here because [`parse_format`] has
/// already validated it.
fn positionals_without_options(args: &[String]) -> Result<Vec<String>, i32> {
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--format" => index += 2,
            other if other.starts_with('-') => {
                return Err(usage_error(&format!("unknown option '{other}'")))
            }
            other => {
                positionals.push(other.to_string());
                index += 1;
            }
        }
    }
    Ok(positionals)
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

/// Escape terminal control characters for plain-text output.
///
/// Theme ids, names, source paths and diagnostic strings all come from a
/// user-authored file, and plain output goes straight to a terminal — a raw ESC
/// there is a cursor-moving, colour-setting instruction rather than text, and a
/// raw newline breaks the list table apart. C0 (`U+0000..U+001F`) and DEL become
/// a visible `\u{001b}`; every other character, including all of Unicode above
/// DEL, is left exactly as written. The JSON and TOML formats need none of this:
/// serde escapes control characters there already.
fn sanitize_plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch <= '\u{001f}' || ch == '\u{007f}' {
            let _ = write!(out, "\\u{{{:04x}}}", ch as u32);
        } else {
            out.push(ch);
        }
    }
    out
}

fn source_label(record: &ThemeRecord) -> String {
    match &record.source {
        ThemeSource::BuiltIn => "built-in".to_string(),
        ThemeSource::User(path) => path.display().to_string(),
    }
}

fn state_label(record: &ThemeRecord) -> &'static str {
    if !record.is_valid() {
        "invalid"
    } else if record.diagnostics.iter().any(ThemeDiagnostic::is_warning) {
        "warning"
    } else {
        "ok"
    }
}

/// The file text behind each origin, so a byte span can be turned into a
/// `line:column`. Built-ins share one label and are therefore not mapped: a
/// span inside an embedded asset would be a build failure, not a user problem.
fn source_texts(registry: &ThemeRegistry) -> BTreeMap<String, String> {
    registry
        .records()
        .iter()
        .filter_map(|record| match &record.source {
            ThemeSource::User(path) => {
                Some((path.display().to_string(), record.toml_source.clone()))
            }
            ThemeSource::BuiltIn => None,
        })
        .collect()
}

/// 1-based line and column of a byte offset.
fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .count()
        + 1;
    (line, column)
}

#[derive(Debug, Serialize)]
struct DiagnosticJson {
    severity: &'static str,
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    column: Option<usize>,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    help: Option<String>,
}

impl DiagnosticJson {
    fn new(diagnostic: &ThemeDiagnostic, sources: &BTreeMap<String, String>) -> Self {
        let file = diagnostic.origin.label().into_owned();
        let position = diagnostic
            .span
            .as_ref()
            .zip(sources.get(&file))
            .map(|(span, text)| line_column(text, span.start));
        Self {
            severity: match diagnostic.severity {
                DiagnosticSeverity::Error => "error",
                DiagnosticSeverity::Warning => "warning",
            },
            file,
            line: position.map(|(line, _)| line),
            column: position.map(|(_, column)| column),
            message: diagnostic.message.clone(),
            help: diagnostic.help.clone(),
        }
    }

    /// The plain rendering of one diagnostic.
    ///
    /// `file`, `message` and `help` are all traceable to a user-authored theme
    /// file, so each goes through [`sanitize_plain`] here — this is the one
    /// place diagnostics become terminal output, in `theme check` and in
    /// `theme list` alike.
    fn render(&self) -> String {
        let file = sanitize_plain(&self.file);
        let message = sanitize_plain(&self.message);
        let mut out = match (self.line, self.column) {
            (Some(line), Some(column)) => {
                format!("{file}:{line}:{column} {}: {message}\n", self.severity)
            }
            _ => format!("{file} {}: {message}\n", self.severity),
        };
        if let Some(help) = &self.help {
            let _ = writeln!(out, "  help: {}", sanitize_plain(help));
        }
        out
    }
}

#[derive(Debug, Serialize)]
struct ThemeEntryJson {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    source: String,
    state: &'static str,
    diagnostics: Vec<DiagnosticJson>,
}

impl ThemeEntryJson {
    fn new(record: &ThemeRecord, sources: &BTreeMap<String, String>) -> Self {
        Self {
            id: record.id.to_string(),
            name: record.name.clone(),
            description: record.description.clone(),
            source: source_label(record),
            state: state_label(record),
            diagnostics: record
                .diagnostics
                .iter()
                .map(|d| DiagnosticJson::new(d, sources))
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
struct ListJson {
    themes_dir: String,
    themes: Vec<ThemeEntryJson>,
    diagnostics: Vec<DiagnosticJson>,
}

#[derive(Debug, Serialize)]
struct CheckJson {
    id: String,
    path: String,
    valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    extends: Option<String>,
    colors: usize,
    gradients: usize,
    overrides: usize,
    diagnostics: Vec<DiagnosticJson>,
}

#[derive(Debug, Serialize)]
struct ShowJson {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    source: String,
    state: &'static str,
    toml_source: String,
}

// ---------------------------------------------------------------------------
// Resolved export
// ---------------------------------------------------------------------------

/// A resolved value as an export writes it. Keeping the DTOs here is what keeps
/// Ratatui types out of the serialised surface: `Color` has no stable
/// serialisation, and a theme file spells colours as `#rrggbb` or `"terminal"`.
#[derive(Debug, Serialize)]
struct NamedValueJson {
    key: String,
    value: String,
}

#[derive(Debug, Serialize)]
struct GradientStopJson {
    at: f64,
    color: String,
}

#[derive(Debug, Serialize)]
struct GradientJson {
    name: String,
    direction: &'static str,
    stops: Vec<GradientStopJson>,
}

#[derive(Debug, Serialize)]
struct ComponentJson {
    path: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gradient: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    foreground: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modifiers: Option<Vec<&'static str>>,
}

#[derive(Debug, Serialize)]
struct ResolvedJson {
    schema_version: u32,
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    semantic: Vec<NamedValueJson>,
    gradients: Vec<GradientJson>,
    components: Vec<ComponentJson>,
}

impl ResolvedJson {
    fn new(theme: &ResolvedTheme) -> Self {
        Self {
            schema_version: 1,
            id: theme.id().to_string(),
            name: theme.name().to_string(),
            description: theme.description().map(str::to_string),
            author: theme.author().map(str::to_string),
            semantic: SEMANTIC_SPECS
                .iter()
                .map(|spec| NamedValueJson {
                    key: spec.key.to_string(),
                    value: color_literal(theme.semantic().slot(spec.slot)),
                })
                .collect(),
            gradients: gradient_exports(theme)
                .into_iter()
                .map(|(name, gradient)| GradientJson {
                    name,
                    direction: direction_key(gradient.direction()),
                    stops: gradient
                        .stops()
                        .iter()
                        .map(|stop| GradientStopJson {
                            at: stop.position(),
                            color: color_literal(stop.color()),
                        })
                        .collect(),
                })
                .collect(),
            components: ROLE_SPECS
                .iter()
                .map(|spec| {
                    let path = spec.path.to_string();
                    match spec.role {
                        RoleRef::Color(role) => ComponentJson {
                            path,
                            kind: "color",
                            value: Some(color_literal(theme.color(role))),
                            gradient: None,
                            foreground: None,
                            background: None,
                            modifiers: None,
                        },
                        RoleRef::Paint(role) => match theme.paint(role) {
                            ResolvedPaint::Solid(color) => ComponentJson {
                                path,
                                kind: "paint",
                                value: Some(color_literal(*color)),
                                gradient: None,
                                foreground: None,
                                background: None,
                                modifiers: None,
                            },
                            ResolvedPaint::Gradient(id) => ComponentJson {
                                path,
                                kind: "paint",
                                value: None,
                                gradient: Some(
                                    theme.gradient_name(*id).unwrap_or_default().to_string(),
                                ),
                                foreground: None,
                                background: None,
                                modifiers: None,
                            },
                        },
                        RoleRef::Tint(role) => match theme.tint(role) {
                            ResolvedTint::Native => ComponentJson {
                                path,
                                kind: "tint",
                                value: Some("native".to_string()),
                                gradient: None,
                                foreground: None,
                                background: None,
                                modifiers: None,
                            },
                            ResolvedTint::Color(color) => ComponentJson {
                                path,
                                kind: "tint",
                                value: Some(color_literal(*color)),
                                gradient: None,
                                foreground: None,
                                background: None,
                                modifiers: None,
                            },
                        },
                        RoleRef::Style(role) => {
                            let style = theme.style(role);
                            ComponentJson {
                                path,
                                kind: "style",
                                value: None,
                                gradient: None,
                                foreground: style.fg.map(color_literal),
                                background: style.bg.map(color_literal),
                                modifiers: Some(modifier_keys(style)),
                            }
                        }
                    }
                })
                .collect(),
        }
    }
}

/// The theme's gradients paired with the names their author gave them.
fn gradient_exports(theme: &ResolvedTheme) -> Vec<(String, &ResolvedGradient)> {
    theme
        .gradients()
        .iter()
        .enumerate()
        .map(|(index, gradient)| {
            let name = theme
                .gradient_names
                .get(index)
                .cloned()
                // A nameless gradient cannot be referenced, so a synthetic name
                // keeps the export re-parsable instead of silently lossy.
                .unwrap_or_else(|| format!("gradient_{index}"));
            (name, gradient)
        })
        .collect()
}

fn direction_key(direction: GradientDirection) -> &'static str {
    let index = match direction {
        GradientDirection::Horizontal => 0,
        GradientDirection::Vertical => 1,
        GradientDirection::DiagonalDown => 2,
        GradientDirection::DiagonalUp => 3,
        GradientDirection::Perimeter => 4,
    };
    GradientDirection::KEYS[index]
}

fn modifier_keys(style: Style) -> Vec<&'static str> {
    MODIFIER_KEYS
        .iter()
        .filter(|key| {
            modifier_from_key(key).is_some_and(|modifier| style.add_modifier.contains(modifier))
        })
        .copied()
        .collect()
}

/// A colour as a theme file spells it. Resolution only ever produces `Rgb` or
/// the `Reset` that `"terminal"` stands for.
fn color_literal(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => "terminal".to_string(),
    }
}

/// One complete TOML string literal, quotes and escapes included.
///
/// Delegated to `toml_edit` rather than hand-rolled: the previous three
/// `replace` calls covered `\`, `"` and `\n` and silently emitted every other
/// control character raw, which TOML forbids inside a basic string. The crate
/// is already a dependency and owns the full escape table, so there is no
/// second spelling of the rules to keep in sync.
///
/// Which literal form comes back is `toml_edit`'s choice — a value containing a
/// quote or a backslash is shorter as a single-quoted *literal string* and is
/// emitted that way. Both forms are valid wherever this is used, as a value and
/// as a quoted key alike, so callers must not assume a leading `"`.
fn toml_string(value: &str) -> String {
    toml_edit::Value::from(value).to_string().trim().to_string()
}

/// One key segment of a TOML table header or dotted key.
///
/// A bare key may only be ASCII letters, digits, `_` and `-`; anything else —
/// a space, a dot, a quote, a control byte, any non-ASCII letter — has to be
/// written as a quoted key, which is the same literal a value would use. Simple
/// names stay bare so `[gradients.reef_ring]` keeps reading the way its author
/// wrote it.
fn toml_key_segment(value: &str) -> String {
    let bare = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        value.to_string()
    } else {
        toml_string(value)
    }
}

fn toml_float(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        format!("{value}")
    }
}

/// The whole theme as a standalone TOML document.
///
/// No `extends` and no references survive: every semantic slot, every gradient
/// and every component role is written out, so re-reading the file — where the
/// implicit `default` parent applies again — yields the same runtime theme.
fn resolved_toml(theme: &ResolvedTheme) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# fully resolved export of theme '{}'; every value is written out, so this\n\
         # file stands on its own. Change `name` before installing it under a new\n\
         # filename.",
        theme.id()
    );
    let _ = writeln!(out, "schema_version = 1");
    let _ = writeln!(out, "name = {}", toml_string(theme.name()));
    if let Some(description) = theme.description() {
        let _ = writeln!(out, "description = {}", toml_string(description));
    }
    if let Some(author) = theme.author() {
        let _ = writeln!(out, "author = {}", toml_string(author));
    }

    let _ = writeln!(out, "\n[semantic]");
    for spec in SEMANTIC_SPECS {
        let _ = writeln!(
            out,
            "{} = {}",
            spec.key,
            toml_string(&color_literal(theme.semantic().slot(spec.slot)))
        );
    }

    for (name, gradient) in gradient_exports(theme) {
        let _ = writeln!(out, "\n[gradients.{}]", toml_key_segment(&name));
        let _ = writeln!(
            out,
            "direction = {}",
            toml_string(direction_key(gradient.direction()))
        );
        let _ = writeln!(out, "stops = [");
        for stop in gradient.stops() {
            let _ = writeln!(
                out,
                "  {{ at = {}, color = {} }},",
                toml_float(stop.position()),
                toml_string(&color_literal(stop.color()))
            );
        }
        let _ = writeln!(out, "]");
    }

    // Roles are grouped by their section so every `[components.…]` table is
    // written exactly once; ROLE_SPECS order is kept inside a section.
    let mut sections: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for spec in ROLE_SPECS {
        let Some((section, leaf)) = spec.path.rsplit_once('.') else {
            continue;
        };
        let line = match spec.role {
            RoleRef::Color(role) => format!(
                "{leaf} = {}",
                toml_string(&color_literal(theme.color(role)))
            ),
            RoleRef::Paint(role) => match theme.paint(role) {
                ResolvedPaint::Solid(color) => {
                    format!("{leaf} = {}", toml_string(&color_literal(*color)))
                }
                ResolvedPaint::Gradient(id) => {
                    // The whole reference is one logical string, serialised as
                    // one value. Interpolating the name inside hand-written
                    // quotes let a name containing a quote close the literal
                    // early and produce a file that no longer parsed.
                    let name = theme.gradient_name(*id).unwrap_or_default();
                    let reference = format!("gradients.{name}");
                    format!("{leaf} = {{ gradient = {} }}", toml_string(&reference))
                }
            },
            RoleRef::Tint(role) => match theme.tint(role) {
                ResolvedTint::Native => format!("{leaf} = \"native\""),
                ResolvedTint::Color(color) => {
                    format!("{leaf} = {}", toml_string(&color_literal(*color)))
                }
            },
            RoleRef::Style(role) => {
                let style = theme.style(role);
                let mut fields = Vec::new();
                if let Some(fg) = style.fg {
                    fields.push(format!("foreground = {}", toml_string(&color_literal(fg))));
                }
                if let Some(bg) = style.bg {
                    fields.push(format!("background = {}", toml_string(&color_literal(bg))));
                }
                let modifiers: Vec<String> =
                    modifier_keys(style).into_iter().map(toml_string).collect();
                fields.push(format!("modifiers = [{}]", modifiers.join(", ")));
                format!("{leaf} = {{ {} }}", fields.join(", "))
            }
        };
        sections.entry(section).or_default().push(line);
    }
    for (section, lines) in sections {
        let _ = writeln!(out, "\n[{section}]");
        for line in lines {
            let _ = writeln!(out, "{line}");
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin(id: &str) -> std::rc::Rc<ResolvedTheme> {
        ThemeRegistry::builtins(ValidationMode::Strict)
            .unwrap()
            .resolved(&crate::theme::model::ThemeId::parse(id).unwrap())
            .unwrap()
    }

    /// The spec's round-trip requirement: the resolved export must parse again
    /// and be *semantically equal* — same semantic core, same gradients under
    /// the same names, same value for every component role.
    #[test]
    fn a_resolved_export_reparses_to_the_same_theme() {
        for id in ["default", "summer", "aqua", "fire", "high-contrast"] {
            let original = builtin(id);
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(format!("{id}.toml"));
            std::fs::write(&path, resolved_toml(&original)).unwrap();

            let registry = ThemeRegistry::load_check_target(&path, ValidationMode::Strict).unwrap();
            let record = registry.get(id).expect("the export is registered");
            assert!(
                record.is_valid(),
                "{id} export is invalid: {:#?}",
                record.diagnostics
            );
            let reparsed = record.resolved().expect("the export resolves");
            // `semantically_eq`, not `==`: the export is a second resolve run,
            // so the two carry different generations by design. What has to
            // match is everything else.
            assert!(
                reparsed.semantically_eq(&original),
                "{id} export does not round-trip semantically"
            );
        }
    }

    /// `author` has to survive resolution *and* both exports.
    ///
    /// The round-trip above cannot catch its loss on its own: if the field is
    /// dropped during resolution it is already gone from both sides of the
    /// `ResolvedTheme == ResolvedTheme` comparison. So this test starts from a
    /// serialised document that carries one, and checks the value in the
    /// resolved theme, in the TOML export and in the JSON export.
    #[test]
    fn a_resolved_export_keeps_the_author() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credited.toml");
        std::fs::write(
            &path,
            "schema_version = 1\nname = \"Credited\"\nauthor = \"Ada Lovelace\"\n\
             description = \"has a credit line\"\n\n[semantic]\naccent = \"#123456\"\n",
        )
        .unwrap();
        let registry = ThemeRegistry::load_check_target(&path, ValidationMode::Strict).unwrap();
        let theme = registry
            .get("credited")
            .and_then(|record| record.resolved())
            .expect("the file resolves")
            .clone();
        assert_eq!(theme.author(), Some("Ada Lovelace"));

        let toml = resolved_toml(&theme);
        assert!(
            toml.contains("author = \"Ada Lovelace\""),
            "the TOML export dropped the author:\n{toml}"
        );
        let json = serde_json::to_string(&ResolvedJson::new(&theme)).unwrap();
        assert!(
            json.contains("\"author\":\"Ada Lovelace\""),
            "the JSON export dropped the author:\n{json}"
        );

        // And the export still round-trips, now with the credit on both sides.
        let round = dir.path().join("credited-export.toml");
        std::fs::write(&round, &toml).unwrap();
        let registry = ThemeRegistry::load_check_target(&round, ValidationMode::Strict).unwrap();
        let reparsed = registry
            .get("credited-export")
            .and_then(|record| record.resolved())
            .expect("the export resolves");
        assert_eq!(reparsed.author(), Some("Ada Lovelace"));
    }

    /// A gradient whose authored name is not a bare key still round-trips.
    ///
    /// The export writes both a table header and a paint reference from that
    /// name. A hand-rolled escaper that only knew `\`, `"` and `\n` produced an
    /// invalid header for a name containing a space or a dot, and the reference
    /// was interpolated *inside* its own quotes — so a name with a quote in it
    /// closed the string early. Either way the export no longer parsed.
    #[test]
    fn a_resolved_export_round_trips_a_quoted_gradient_key() {
        // Space, dot, double quote and backslash: each breaks a different part
        // of the old writer.
        const NAME: &str = "odd .\"\\ name";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quoted.toml");
        std::fs::write(
            &path,
            "schema_version = 1\nname = \"Quoted\"\n\n\
             [gradients.\"odd .\\\"\\\\ name\"]\ndirection = \"horizontal\"\n\
             stops = [ { at = 0.0, color = \"#102030\" }, { at = 1.0, color = \"#405060\" } ]\n\n\
             [components.app]\nbackground = { gradient = \"gradients.odd .\\\"\\\\ name\" }\n",
        )
        .unwrap();

        let registry = ThemeRegistry::load_check_target(&path, ValidationMode::Strict).unwrap();
        let original = registry
            .get("quoted")
            .and_then(|record| record.resolved())
            .expect("the fixture resolves")
            .clone();
        assert_eq!(
            original.gradient_names,
            vec![NAME.to_string()],
            "the fixture really carries the awkward name"
        );

        // The export has to parse again — under `Strict`, so an unknown role or
        // a broken reference is an error rather than a warning.
        let exported = resolved_toml(&original);
        let round = dir.path().join("quoted-export.toml");
        std::fs::write(&round, &exported).unwrap();
        let registry = ThemeRegistry::load_check_target(&round, ValidationMode::Strict).unwrap();
        let record = registry.get("quoted-export").expect("the export registers");
        assert!(
            record.is_valid(),
            "the export does not parse:\n{exported}\n{:#?}",
            record.diagnostics
        );
        let reparsed = record.resolved().expect("the export resolves");

        // Same name, and the paint role still points at that same gradient.
        assert_eq!(reparsed.gradient_names, vec![NAME.to_string()]);
        let ResolvedPaint::Gradient(id) =
            reparsed.paint(crate::theme::catalog::PaintRole::AppBackground)
        else {
            panic!("the reparsed app background is no longer a gradient");
        };
        assert_eq!(reparsed.gradient_name(*id), Some(NAME));
        assert_eq!(
            reparsed.gradient(*id).map(|g| g.stops().to_vec()),
            original.gradients().first().map(|g| g.stops().to_vec()),
            "the reference points at a different gradient after the round trip"
        );
    }

    /// Two stops a hair apart stay two stops, from validation through the
    /// runtime to the export and back.
    ///
    /// `0.5` and `0.50000001` are distinct `f64` values and equal `f32` ones,
    /// so storing positions as `f32` silently merged them: validation accepted
    /// the file, then the resolver collapsed the pair, `sample` saw a zero-width
    /// span, and the export wrote the same number twice — a theme that no longer
    /// round-tripped and a gradient the author could not explain.
    #[test]
    fn near_identical_gradient_stops_survive_validation_runtime_and_export() {
        const NEAR: f64 = 0.50000001;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hairline.toml");
        std::fs::write(
            &path,
            "schema_version = 1\nname = \"Hairline\"\n\n\
             [gradients.hair]\ndirection = \"horizontal\"\n\
             stops = [ { at = 0.0, color = \"#000000\" }, { at = 0.5, color = \"#204060\" }, \
             { at = 0.50000001, color = \"#a0c0e0\" }, { at = 1.0, color = \"#ffffff\" } ]\n\n\
             [components.app]\nbackground = { gradient = \"gradients.hair\" }\n",
        )
        .unwrap();

        let registry = ThemeRegistry::load_check_target(&path, ValidationMode::Strict).unwrap();
        let record = registry.get("hairline").expect("the fixture registers");
        assert!(
            record.is_valid(),
            "validation rejected the hairline stops: {:#?}",
            record.diagnostics
        );
        let theme = record.resolved().expect("the fixture resolves").clone();

        // Runtime: four stops, strictly ascending. Under `f32` the middle pair
        // compared equal here.
        let stops = theme.gradients()[0].stops().to_vec();
        assert_eq!(stops.len(), 4, "a stop went missing");
        for pair in stops.windows(2) {
            assert!(
                pair[0].position() < pair[1].position(),
                "stops collapsed onto one position: {:?} then {:?}",
                pair[0].position(),
                pair[1].position()
            );
        }
        assert_eq!(stops[2].position(), NEAR);

        // Sampling: each of the two near stops still yields its *own* colour.
        // A collapsed pair has a zero-width span, so both sampled as the first.
        let gradient = &theme.gradients()[0];
        assert_eq!(gradient.sample(0.5), Color::Rgb(0x20, 0x40, 0x60), "at 0.5");
        assert_eq!(
            gradient.sample(NEAR),
            Color::Rgb(0xa0, 0xc0, 0xe0),
            "at {NEAR}"
        );

        // Export: the two positions are written apart and come back apart.
        let exported = resolved_toml(&theme);
        assert!(
            exported.contains("at = 0.50000001"),
            "the export lost the hairline position:\n{exported}"
        );
        let round = dir.path().join("hairline-export.toml");
        std::fs::write(&round, &exported).unwrap();
        let registry = ThemeRegistry::load_check_target(&round, ValidationMode::Strict).unwrap();
        let reparsed = registry
            .get("hairline-export")
            .and_then(|record| record.resolved())
            .expect("the export resolves");
        let reparsed_stops = reparsed.gradients()[0].stops().to_vec();
        assert_eq!(
            reparsed_stops
                .iter()
                .map(|s| s.position())
                .collect::<Vec<_>>(),
            vec![0.0, 0.5, NEAR, 1.0],
            "the round trip moved a stop"
        );

        // And the JSON export carries the same value, since it is the same f64.
        let json = serde_json::to_string(&ResolvedJson::new(&theme)).unwrap();
        assert!(
            json.contains("0.50000001"),
            "the JSON export rounded:\n{json}"
        );
    }

    /// The bare-key rule the gradient table header is written with.
    #[test]
    fn toml_key_segments_are_bare_only_when_they_may_be() {
        // ASCII letters, digits, `_` and `-` are the whole bare-key alphabet.
        for bare in ["reef_ring", "a", "Ring-2", "0", "_x-9Z"] {
            assert_eq!(toml_key_segment(bare), bare, "`{bare}` may stay bare");
        }
        // Everything else goes through the same serialiser as a value, so a
        // dot, a space, a quote, a control byte or any non-ASCII letter is
        // represented rather than pasted in raw. `toml_edit` picks whichever
        // literal is shortest, so the quote character is its choice — a value
        // containing `"` comes back as a single-quoted literal string, which is
        // just as valid a TOML key.
        for quoted in [
            "",
            "has.dot",
            "a b",
            "q\"uote",
            "back\\slash",
            "n\u{00fc}ance",
            "tab\there",
            "ctl\u{0007}",
        ] {
            let out = toml_key_segment(quoted);
            assert_eq!(out, toml_string(quoted), "`{quoted:?}` must be quoted");
            let first = out.chars().next().expect("a literal is never empty");
            assert!(
                (first == '"' || first == '\'') && out.ends_with(first) && out.len() >= 2,
                "`{quoted:?}` did not produce a complete string literal: {out}"
            );
            // And it really is that value: parsing the literal back yields the
            // original, which is the property the export depends on.
            let parsed: toml_edit::Value = out.parse().expect("a valid TOML value");
            assert_eq!(
                parsed.as_str(),
                Some(quoted),
                "{out} does not mean {quoted:?}"
            );
        }
    }

    #[test]
    fn a_resolved_export_names_its_gradients_the_way_the_author_did() {
        let aqua = builtin("aqua");
        let toml = resolved_toml(&aqua);
        assert!(toml.contains("[gradients.reef_ring]"), "{toml}");
        assert!(
            toml.contains("{ gradient = \"gradients.reef_ring\" }"),
            "a gradient role must reference the gradient by name:\n{toml}"
        );
        assert!(!toml.contains("extends"), "a resolved export has no parent");
    }

    /// A diagnostic quotes a file path, a message and a help line that all
    /// ultimately come from a user-authored theme file, so none of them may
    /// carry a raw control byte into the operator's terminal.
    #[test]
    fn a_rendered_diagnostic_escapes_control_characters() {
        let diagnostic = DiagnosticJson {
            severity: "error",
            file: "/tmp/ev\u{1b}[31mil.toml".to_string(),
            line: Some(3),
            column: Some(7),
            message: "bad\nvalue\u{7f}".to_string(),
            help: Some("try\u{1b}]0;pwned\u{7}this".to_string()),
        };

        let out = diagnostic.render();
        assert!(
            !out.chars().any(|c| c.is_control() && c != '\n'),
            "a raw control byte survived rendering: {out:?}"
        );
        assert!(out.contains("ev\\u{001b}[31mil.toml"), "{out}");
        assert!(out.contains("bad\\u{000a}value\\u{007f}"), "{out}");
        assert!(out.contains("try\\u{001b}]0;pwned\\u{0007}this"), "{out}");
        // Only the two line terminators the renderer writes itself remain.
        assert_eq!(out.matches('\n').count(), 2, "{out:?}");
    }

    #[test]
    fn line_column_counts_from_one() {
        let text = "abc\ndef\n";
        assert_eq!(line_column(text, 0), (1, 1));
        assert_eq!(line_column(text, 4), (2, 1));
        assert_eq!(line_column(text, 6), (2, 3));
    }
}
