//! Discovery, validation and isolation of built-in and installed themes.
//!
//! The registry is the start-up order of the V1 spec turned into one type:
//! register the embedded built-ins, read `themes/*.toml` lexicographically,
//! validate every file on its own, then resolve inheritance and references
//! across all of them at once.
//!
//! Its second job is isolation. A broken theme file must never keep SSHub from
//! starting and must never invalidate its neighbours, so every failure ends up
//! as a diagnostic on one record instead of an error out of the loader. Invalid
//! records — including a user file squatting on a reserved built-in id — stay
//! listed so `theme list` can explain what happened; only records that resolved
//! cleanly can be activated.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::theme::builtins::{self, BUILT_IN_THEMES};
use crate::theme::model::{
    ResolvedTheme, ThemeDefinition, ThemeDiagnostic, ThemeId, ThemeIdError, ThemeOrigin,
    ValidationMode,
};
use crate::theme::resolve::resolve_theme;
use crate::theme::validate::parse_and_validate;

/// The two V1 limits the loader owns. The remaining ones live where the code
/// that can exceed them lives: entry, gradient and stop counts in `validate`,
/// inheritance and colour-reference depth in `resolve`.
///
/// A theme file is a few kilobytes of hand-written TOML; a megabyte is three
/// orders of magnitude of headroom and still bounds what a generated or hostile
/// file can make the parser allocate.
pub const MAX_THEME_FILE_BYTES: u64 = 1024 * 1024;

/// Upper bound on `*.toml` files read from the themes directory.
pub const MAX_USER_THEME_FILES: usize = 256;

/// Where a record's theme came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeSource {
    /// Embedded in the binary; its id is reserved.
    BuiltIn,
    /// A file the user installed or handed to `theme check`.
    User(PathBuf),
}

/// Whether a record can be activated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeStatus {
    /// Validated and resolved; [`ThemeRegistry::resolved`] returns it.
    Valid,
    /// Rejected. The record stays listed so the reason is reportable.
    Invalid,
}

/// One theme the registry knows about, valid or not.
#[derive(Clone, Debug)]
pub struct ThemeRecord {
    pub id: ThemeId,
    /// Display name from the file; empty when the file could not be parsed.
    pub name: String,
    pub description: Option<String>,
    pub source: ThemeSource,
    pub origin: ThemeOrigin,
    /// The file verbatim. `theme show` prints this, so it must not be
    /// re-serialised from the parsed definition: comments are the greater half
    /// of what makes a built-in copyable.
    pub toml_source: String,
    pub status: ThemeStatus,
    /// File diagnostics and, where resolution ran, resolution diagnostics —
    /// merged into one presentation-ordered list.
    pub diagnostics: Vec<ThemeDiagnostic>,
    resolved: Option<Rc<ResolvedTheme>>,
    inheritance_chain: Vec<ThemeId>,
}

impl ThemeRecord {
    pub fn resolved(&self) -> Option<&Rc<ResolvedTheme>> {
        self.resolved.as_ref()
    }

    /// The themes that were merged into this one, child first and root last.
    /// Empty when the record never reached resolution.
    pub fn inheritance_chain(&self) -> &[ThemeId] {
        &self.inheritance_chain
    }

    pub fn is_valid(&self) -> bool {
        self.status == ThemeStatus::Valid
    }
}

/// A failure that stops the registry from being built at all.
///
/// Deliberately rare: a missing themes directory, an unreadable file and a
/// malformed theme are all normal states that produce diagnostics instead.
#[derive(Debug)]
pub enum ThemeRegistryError {
    Io { path: PathBuf, source: io::Error },
    InvalidFileName { path: PathBuf, source: ThemeIdError },
    TooLarge { path: PathBuf, length: u64 },
}

impl fmt::Display for ThemeRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::InvalidFileName { path, source } => {
                write!(f, "{}: {source}", path.display())
            }
            Self::TooLarge { path, length } => write!(
                f,
                "{}: theme file is {length} bytes; the limit is 1 MiB",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ThemeRegistryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidFileName { source, .. } => Some(source),
            Self::TooLarge { .. } => None,
        }
    }
}

/// A theme source that has been located but not yet parsed.
struct Candidate {
    id: ThemeId,
    source: ThemeSource,
    origin: ThemeOrigin,
    text: String,
}

/// Built-in and installed themes, with everything needed to report on them.
pub struct ThemeRegistry {
    records: Vec<ThemeRecord>,
    /// The record that owns an id. A later candidate claiming a taken id is a
    /// collision and is never registered here, which is what keeps a user file
    /// from replacing a built-in.
    canonical: BTreeMap<ThemeId, usize>,
    /// Canonical records that resolved without errors.
    resolvable: BTreeMap<ThemeId, usize>,
    /// Problems that belong to the directory rather than to one theme.
    diagnostics: Vec<ThemeDiagnostic>,
}

impl ThemeRegistry {
    /// Only the embedded themes. Used by tests and by every code path that must
    /// work without a config directory.
    pub fn builtins(mode: ValidationMode) -> Result<Self, ThemeRegistryError> {
        Ok(Self::build(builtin_candidates(), Vec::new(), mode))
    }

    /// The built-ins plus every `*.toml` directly inside `themes_dir`.
    ///
    /// A missing directory is the normal state before the first user theme, so
    /// it yields the built-ins rather than an error.
    pub fn load_installed(
        themes_dir: &Path,
        mode: ValidationMode,
    ) -> Result<Self, ThemeRegistryError> {
        let mut diagnostics = Vec::new();
        let user = read_user_directory(themes_dir, &mut diagnostics)?;
        let mut candidates = builtin_candidates();
        candidates.extend(user);
        Ok(Self::build(candidates, diagnostics, mode))
    }

    /// One explicitly named file, checked against the built-ins and against
    /// the other themes lying next to it.
    ///
    /// The file need not live in the themes directory, which is the point:
    /// `theme check` runs on a draft before it is installed. Its own directory
    /// stands in for the installed one, so a portable package whose child
    /// extends a sibling can be checked as the package it is; a parent that
    /// actually came from a sibling is reported, because installing the child
    /// alone would leave it unresolvable.
    pub fn load_check_target(
        file: &Path,
        mode: ValidationMode,
    ) -> Result<Self, ThemeRegistryError> {
        let stem = file
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        let id = ThemeId::parse(stem).map_err(|source| ThemeRegistryError::InvalidFileName {
            path: file.to_path_buf(),
            source,
        })?;
        let length = fs::metadata(file)
            .map_err(|source| ThemeRegistryError::Io {
                path: file.to_path_buf(),
                source,
            })?
            .len();
        if length > MAX_THEME_FILE_BYTES {
            return Err(ThemeRegistryError::TooLarge {
                path: file.to_path_buf(),
                length,
            });
        }
        let text = fs::read_to_string(file).map_err(|source| ThemeRegistryError::Io {
            path: file.to_path_buf(),
            source,
        })?;

        let reserved = builtins::is_reserved(id.as_str());
        // A draft of a built-in is a legitimate thing to check. Dropping the
        // built-in of the same id lets the file be checked as its author wrote
        // it, instead of being dismissed as a collision and reported with none
        // of its actual schema problems. This registry is throwaway and never
        // activates a theme, so nothing can inherit the shadowing by accident.
        let mut candidates: Vec<Candidate> = builtin_candidates()
            .into_iter()
            .filter(|candidate| candidate.id != id)
            .collect();
        // The neighbourhood is read with the loader's own limits and order, so
        // a package behaves here exactly as it will once installed. Its
        // directory-level *diagnostics* are dropped on purpose: they belong to
        // files this command was not asked about, and a parent lost to one of
        // them still surfaces as `unknown parent theme` on the target itself.
        //
        // The `Result`, in contrast, must not be swallowed. A target can be
        // readable while its directory cannot be listed (POSIX `0111`), and in
        // that state it is precisely unknown whether the siblings this theme
        // needs exist — reporting it valid would be a guess dressed up as a
        // pass.
        let mut neighbourhood = Vec::new();
        let target_key = same_file_key(file);
        candidates.extend(
            read_user_directory(dir_of(file), &mut neighbourhood)?
                .into_iter()
                // The explicit target is the check target, whatever its
                // directory also calls it; reading it twice would only make it
                // collide with itself.
                .filter(|candidate| match &candidate.source {
                    ThemeSource::User(path) => same_file_key(path) != target_key,
                    ThemeSource::BuiltIn => true,
                }),
        );
        candidates.push(Candidate {
            id: id.clone(),
            source: ThemeSource::User(file.to_path_buf()),
            origin: ThemeOrigin::User(file.to_path_buf()),
            text,
        });

        let mut registry = Self::build(candidates, Vec::new(), mode);
        registry.warn_about_sibling_parents(&id, file);
        if reserved {
            if let Some(&index) = registry.canonical.get(&id) {
                let record = &mut registry.records[index];
                record.diagnostics.push(
                    ThemeDiagnostic::warning(
                        ThemeOrigin::User(file.to_path_buf()),
                        None,
                        format!("theme id `{id}` is reserved for a built-in theme"),
                    )
                    .with_help(format!(
                        "installing this file as `{id}.toml` would be rejected; save it as \
                         `{id}-custom.toml` and set `active_theme = \"{id}-custom\"`"
                    )),
                );
                sort_diagnostics(&mut record.diagnostics);
            }
        }
        Ok(registry)
    }

    /// Flag every parent of the check target that was supplied by a file next
    /// to it rather than by a built-in.
    ///
    /// The chain is authoritative here: only what resolution actually merged
    /// counts, so a sibling that merely exists is never reported.
    fn warn_about_sibling_parents(&mut self, target: &ThemeId, file: &Path) {
        let Some(&index) = self.canonical.get(target) else {
            return;
        };
        let target_key = same_file_key(file);
        let child = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| target.to_string());
        let warnings: Vec<ThemeDiagnostic> = self.records[index]
            .inheritance_chain
            .iter()
            .filter(|ancestor| *ancestor != target)
            .filter_map(|ancestor| {
                let &parent_index = self.canonical.get(ancestor)?;
                let ThemeSource::User(path) = &self.records[parent_index].source else {
                    return None;
                };
                if same_file_key(path) == target_key {
                    return None;
                }
                let sibling = path.file_name()?.to_string_lossy().into_owned();
                Some(
                    ThemeDiagnostic::warning(
                        ThemeOrigin::User(file.to_path_buf()),
                        None,
                        format!(
                            "parent theme `{ancestor}` comes from the sibling file `{sibling}`"
                        ),
                    )
                    .with_help(format!(
                        "install `{sibling}` together with `{child}`; on its own `{target}` cannot \
                         resolve"
                    )),
                )
            })
            .collect();
        if warnings.is_empty() {
            return;
        }
        let record = &mut self.records[index];
        record.diagnostics.extend(warnings);
        sort_diagnostics(&mut record.diagnostics);
    }

    /// Every known theme: built-ins in registration order, then user themes in
    /// file-name order, including the invalid ones.
    pub fn records(&self) -> &[ThemeRecord] {
        &self.records
    }

    /// The record that owns `id`, which for a reserved id is always the
    /// built-in — never a user file claiming the same name.
    pub fn get(&self, id: &str) -> Option<&ThemeRecord> {
        let id = ThemeId::parse(id).ok()?;
        self.canonical.get(&id).map(|&index| &self.records[index])
    }

    /// The runtime theme for `id`, or `None` when it is unknown or invalid.
    pub fn resolved(&self, id: &ThemeId) -> Option<Rc<ResolvedTheme>> {
        let index = *self.resolvable.get(id)?;
        self.records[index].resolved.clone()
    }

    /// Diagnostics about the themes directory itself — an unusable file name,
    /// an unreadable or oversized file, or more files than V1 loads.
    pub fn diagnostics(&self) -> &[ThemeDiagnostic] {
        &self.diagnostics
    }

    fn build(
        candidates: Vec<Candidate>,
        diagnostics: Vec<ThemeDiagnostic>,
        mode: ValidationMode,
    ) -> Self {
        let mut records: Vec<ThemeRecord> = Vec::with_capacity(candidates.len());
        let mut canonical: BTreeMap<ThemeId, usize> = BTreeMap::new();
        // Only definitions that are canonical *and* free of file errors may take
        // part in inheritance: a half-parsed parent would turn one broken file
        // into a broken subtree.
        let mut definitions: BTreeMap<ThemeId, ThemeDefinition> = BTreeMap::new();

        for candidate in candidates {
            let index = records.len();
            let outcome = parse_and_validate(
                candidate.id.clone(),
                candidate.origin.clone(),
                &candidate.text,
                mode,
            );
            let mut record_diagnostics = outcome.diagnostics;
            let mut usable = !record_diagnostics.iter().any(ThemeDiagnostic::is_error);

            if canonical.contains_key(&candidate.id) {
                usable = false;
                record_diagnostics.push(collision_diagnostic(&candidate));
                sort_diagnostics(&mut record_diagnostics);
            } else {
                canonical.insert(candidate.id.clone(), index);
            }

            let (name, description) = match &outcome.definition {
                Some(definition) => (
                    definition.name.value.clone(),
                    definition
                        .description
                        .as_ref()
                        .map(|spanned| spanned.value.clone()),
                ),
                None => (String::new(), None),
            };
            if usable {
                if let Some(definition) = outcome.definition {
                    definitions.insert(candidate.id.clone(), definition);
                }
            }

            records.push(ThemeRecord {
                id: candidate.id,
                name,
                description,
                source: candidate.source,
                origin: candidate.origin,
                toml_source: candidate.text,
                status: if usable {
                    ThemeStatus::Valid
                } else {
                    ThemeStatus::Invalid
                },
                diagnostics: record_diagnostics,
                resolved: None,
                inheritance_chain: Vec::new(),
            });
        }

        // Resolution needs the whole ancestry at once — the resolver never looks
        // a missing parent up on its own — so it can only run after every file
        // has been read. That is also what lets a theme extend a sibling that
        // sorts after it.
        let view: BTreeMap<ThemeId, &ThemeDefinition> = definitions
            .iter()
            .map(|(id, definition)| (id.clone(), definition))
            .collect();
        let mut resolvable = BTreeMap::new();
        for (id, &index) in &canonical {
            if !definitions.contains_key(id) {
                continue;
            }
            let outcome = resolve_theme(id, &view);
            let record = &mut records[index];
            record.diagnostics.extend(outcome.diagnostics);
            record.inheritance_chain = outcome.inheritance_chain;
            sort_diagnostics(&mut record.diagnostics);
            match outcome.theme {
                Some(theme) => {
                    record.resolved = Some(Rc::new(theme));
                    resolvable.insert(id.clone(), index);
                }
                None => record.status = ThemeStatus::Invalid,
            }
        }

        Self {
            records,
            canonical,
            resolvable,
            diagnostics,
        }
    }
}

/// The directory a check target's neighbourhood is read from. A bare file name
/// has no parent component, and its neighbourhood is the working directory.
fn dir_of(file: &Path) -> &Path {
    match file.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// Identity of a file for "is this the same file", tolerant of the target being
/// named relatively while the directory walk yields joined paths. Canonicalising
/// can fail on a dangling symlink, where the literal path is the best answer
/// available.
fn same_file_key(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The embedded themes as loader input, in registration order.
fn builtin_candidates() -> Vec<Candidate> {
    BUILT_IN_THEMES
        .iter()
        .map(|theme| Candidate {
            // Built-in ids are compile-time literals covered by the asset tests;
            // an unparsable one is a broken build, not a runtime condition.
            id: ThemeId::parse(theme.id).expect("built-in theme id is a valid id"),
            source: ThemeSource::BuiltIn,
            origin: ThemeOrigin::BuiltIn,
            text: theme.source.to_string(),
        })
        .collect()
}

/// Locate and read the installable themes, newest problems first as
/// diagnostics rather than as a failed load.
fn read_user_directory(
    dir: &Path,
    diagnostics: &mut Vec<ThemeDiagnostic>,
) -> Result<Vec<Candidate>, ThemeRegistryError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        // No themes directory simply means no user themes.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(ThemeRegistryError::Io {
                path: dir.to_path_buf(),
                source,
            })
        }
    };

    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| ThemeRegistryError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        // Anything not named `*.toml` is not a theme and is not the loader's
        // business — no diagnostic, however odd it looks.
        if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }
        // `is_file` follows symlinks on purpose: symlinking one theme file into
        // several machines' config directories is a normal way to share it.
        // Subdirectories are ignored — discovery is one level deep.
        if !path.is_file() {
            // A directory or a dangling symlink named `*.toml` still looks like
            // an installed theme to whoever put it there, so skipping it in
            // silence leaves "my theme disappeared from the list" unexplained.
            diagnostics.push(
                ThemeDiagnostic::warning(
                    ThemeOrigin::User(path.clone()),
                    None,
                    "not a readable theme file, so it was skipped".to_string(),
                )
                .with_help(
                    "a theme is a single `.toml` file; check whether this is a directory or a \
                     symlink whose target no longer exists",
                ),
            );
            continue;
        }
        files.push(path);
    }
    // `read_dir` yields in filesystem order, which is arbitrary. The spec's
    // lexicographic order is what makes diagnostics and the picker stable.
    files.sort();

    if files.len() > MAX_USER_THEME_FILES {
        diagnostics.push(
            ThemeDiagnostic::warning(
                ThemeOrigin::User(dir.to_path_buf()),
                None,
                format!(
                    "{} theme files found; only the first {MAX_USER_THEME_FILES} are loaded",
                    files.len()
                ),
            )
            .with_help("move the themes you do not use out of the themes directory"),
        );
        files.truncate(MAX_USER_THEME_FILES);
    }

    let mut candidates = Vec::with_capacity(files.len());
    for path in files {
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        let id = match ThemeId::parse(stem) {
            Ok(id) => id,
            Err(error) => {
                diagnostics.push(
                    ThemeDiagnostic::error(
                        ThemeOrigin::User(path.clone()),
                        None,
                        format!("file name is not a usable theme id: {error}"),
                    )
                    .with_help(
                        "a theme's id is its file name without `.toml`; use only lowercase \
                         letters, digits, `-` and `_`",
                    ),
                );
                continue;
            }
        };

        let length = match fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            Err(error) => {
                diagnostics.push(unreadable(&path, &error));
                continue;
            }
        };
        if length > MAX_THEME_FILE_BYTES {
            diagnostics.push(
                ThemeDiagnostic::error(
                    ThemeOrigin::User(path.clone()),
                    None,
                    format!("theme file is {length} bytes; the limit is 1 MiB"),
                )
                .with_help("split the theme, or move the file out of the themes directory"),
            );
            continue;
        }

        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                diagnostics.push(unreadable(&path, &error));
                continue;
            }
        };
        candidates.push(Candidate {
            id,
            source: ThemeSource::User(path.clone()),
            origin: ThemeOrigin::User(path),
            text,
        });
    }
    Ok(candidates)
}

fn unreadable(path: &Path, error: &io::Error) -> ThemeDiagnostic {
    ThemeDiagnostic::error(
        ThemeOrigin::User(path.to_path_buf()),
        None,
        format!("theme file cannot be read: {error}"),
    )
}

/// The reserved-id message, which has to name a concrete replacement id: a user
/// who reads it should be able to fix the file without consulting the docs.
fn collision_diagnostic(candidate: &Candidate) -> ThemeDiagnostic {
    let id = candidate.id.as_str();
    ThemeDiagnostic::error(
        candidate.origin.clone(),
        None,
        format!("theme id `{id}` is already taken and cannot be replaced"),
    )
    .with_help(format!(
        "rename the file to `{id}-custom.toml` and set `active_theme = \"{id}-custom\"`"
    ))
}

/// The presentation order every diagnostic list in the theme system uses.
/// `sort_key` borrows from the diagnostic, so this cannot be `sort_by_key`.
fn sort_diagnostics(diagnostics: &mut Vec<ThemeDiagnostic>) {
    diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    // The same violation can reach a record twice — once from the file's own
    // validation, once from resolution — with identical wording and span.
    diagnostics.dedup();
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use ratatui::style::Color;

    use super::{
        ThemeRegistry, ThemeRegistryError, ThemeSource, ThemeStatus, MAX_THEME_FILE_BYTES,
        MAX_USER_THEME_FILES,
    };
    use crate::theme::model::{ThemeId, ValidationMode};

    fn minimal(name: &str) -> String {
        format!("schema_version = 1\nname = \"{name}\"\n")
    }

    fn write(dir: &Path, file: &str, body: &str) {
        fs::write(dir.join(file), body).unwrap();
    }

    fn user_ids(registry: &ThemeRegistry) -> Vec<String> {
        registry
            .records()
            .iter()
            .filter(|r| r.source != ThemeSource::BuiltIn)
            .map(|r| r.id.to_string())
            .collect()
    }

    #[test]
    fn reserved_user_collision_is_listed_but_cannot_replace_builtin() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "aqua.toml", "schema_version=1\nname=\"Fake\"\n");
        let registry =
            ThemeRegistry::load_installed(dir.path(), ValidationMode::Compatible).unwrap();

        assert_eq!(registry.get("aqua").unwrap().source, ThemeSource::BuiltIn);
        assert!(registry
            .records()
            .iter()
            .any(|r| { r.name == "Fake" && r.status == ThemeStatus::Invalid }));
        // The built-in itself still resolves, so a squatting file cannot take
        // the theme with it.
        assert!(registry
            .resolved(&ThemeId::parse("aqua").unwrap())
            .is_some());
    }

    #[test]
    fn installing_over_a_reserved_id_suggests_a_concrete_new_id() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "aqua.toml", &minimal("Fake"));
        let registry =
            ThemeRegistry::load_installed(dir.path(), ValidationMode::Compatible).unwrap();

        let squatter = registry
            .records()
            .iter()
            .find(|r| r.source != ThemeSource::BuiltIn)
            .unwrap();
        let help = squatter
            .diagnostics
            .iter()
            .find_map(|d| d.help.clone())
            .unwrap_or_default();
        assert!(help.contains("aqua-custom"), "{help}");
    }

    #[test]
    fn a_missing_themes_directory_leaves_only_the_builtins() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("themes");
        let registry = ThemeRegistry::load_installed(&missing, ValidationMode::Compatible).unwrap();

        assert!(user_ids(&registry).is_empty());
        assert_eq!(registry.records().len(), 5);
        assert!(registry.diagnostics().is_empty());
    }

    #[test]
    fn user_themes_are_discovered_lexicographically_and_only_at_the_top_level() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "beta.toml", &minimal("Beta"));
        write(dir.path(), "alpha.toml", &minimal("Alpha"));
        write(dir.path(), "notes.txt", "not a theme");
        fs::create_dir(dir.path().join("nested")).unwrap();
        write(&dir.path().join("nested"), "deep.toml", &minimal("Deep"));

        let registry =
            ThemeRegistry::load_installed(dir.path(), ValidationMode::Compatible).unwrap();
        assert_eq!(user_ids(&registry), ["alpha", "beta"]);
    }

    #[test]
    fn an_invalid_user_theme_does_not_invalidate_its_siblings() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "broken.toml", "schema_version = 2\nname = 4\n");
        write(dir.path(), "sound.toml", &minimal("Sound"));

        let registry =
            ThemeRegistry::load_installed(dir.path(), ValidationMode::Compatible).unwrap();
        assert_eq!(registry.get("broken").unwrap().status, ThemeStatus::Invalid);
        assert_eq!(registry.get("sound").unwrap().status, ThemeStatus::Valid);
        assert!(registry
            .resolved(&ThemeId::parse("broken").unwrap())
            .is_none());
        assert!(registry
            .resolved(&ThemeId::parse("sound").unwrap())
            .is_some());
        assert!(registry
            .resolved(&ThemeId::parse("default").unwrap())
            .is_some());
    }

    #[test]
    fn a_theme_that_cannot_resolve_is_isolated_from_its_siblings() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "orphan.toml",
            "schema_version = 1\nname = \"Orphan\"\nextends = \"nowhere\"\n",
        );
        write(dir.path(), "sound.toml", &minimal("Sound"));

        let registry =
            ThemeRegistry::load_installed(dir.path(), ValidationMode::Compatible).unwrap();
        let orphan = registry.get("orphan").unwrap();
        assert_eq!(orphan.status, ThemeStatus::Invalid);
        assert!(orphan.diagnostics.iter().any(|d| d.is_error()));
        assert_eq!(registry.get("sound").unwrap().status, ThemeStatus::Valid);
    }

    #[test]
    fn a_theme_file_above_one_mebibyte_is_skipped_with_a_diagnostic() {
        let dir = tempfile::tempdir().unwrap();
        let mut oversized = minimal("Huge");
        oversized.push_str(&format!(
            "# {}\n",
            "p".repeat(MAX_THEME_FILE_BYTES as usize)
        ));
        write(dir.path(), "huge.toml", &oversized);
        write(dir.path(), "small.toml", &minimal("Small"));

        let registry =
            ThemeRegistry::load_installed(dir.path(), ValidationMode::Compatible).unwrap();
        assert_eq!(user_ids(&registry), ["small"]);
        assert!(registry
            .diagnostics()
            .iter()
            .any(|d| d.is_error() && d.message.contains("1 MiB")));
    }

    #[test]
    fn at_most_the_file_limit_of_themes_is_loaded() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..MAX_USER_THEME_FILES + 8 {
            write(dir.path(), &format!("t{index:04}.toml"), &minimal("T"));
        }

        let registry =
            ThemeRegistry::load_installed(dir.path(), ValidationMode::Compatible).unwrap();
        let ids = user_ids(&registry);
        assert_eq!(ids.len(), MAX_USER_THEME_FILES);
        // The cut is lexicographic, so the survivors are a stable prefix.
        assert_eq!(ids.first().unwrap(), "t0000");
        assert_eq!(
            ids.last().unwrap(),
            &format!("t{:04}", MAX_USER_THEME_FILES - 1)
        );
        assert!(registry
            .diagnostics()
            .iter()
            .any(|d| d.message.contains("256")));
    }

    #[test]
    fn a_theme_file_of_exactly_one_mebibyte_is_accepted() {
        // The published limit is inclusive. Without this the `>` in the loader
        // could become `>=` and quietly tighten a documented boundary.
        let dir = tempfile::tempdir().unwrap();
        let head = "schema_version = 1\nname = \"Exact\"\n# ";
        let mut body = String::from(head);
        body.push_str(&"p".repeat(MAX_THEME_FILE_BYTES as usize - head.len() - 1));
        body.push('\n');
        assert_eq!(body.len() as u64, MAX_THEME_FILE_BYTES);
        write(dir.path(), "exact.toml", &body);

        let registry =
            ThemeRegistry::load_installed(dir.path(), ValidationMode::Compatible).unwrap();
        assert_eq!(user_ids(&registry), ["exact"]);
        assert_eq!(registry.get("exact").unwrap().status, ThemeStatus::Valid);
        assert!(registry.diagnostics().is_empty());
    }

    #[test]
    fn exactly_the_file_limit_of_themes_loads_untruncated() {
        // Same boundary on the count: 256 files are loaded, and only the 257th
        // would be dropped.
        let dir = tempfile::tempdir().unwrap();
        for index in 0..MAX_USER_THEME_FILES {
            write(dir.path(), &format!("t{index:04}.toml"), &minimal("T"));
        }

        let registry =
            ThemeRegistry::load_installed(dir.path(), ValidationMode::Compatible).unwrap();
        assert_eq!(user_ids(&registry).len(), MAX_USER_THEME_FILES);
        assert!(registry.diagnostics().is_empty());
    }

    #[test]
    fn a_toml_path_that_is_not_a_readable_file_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "usable.toml", &minimal("Usable"));
        // A directory that looks like a theme.
        fs::create_dir(dir.path().join("bundle.toml")).unwrap();
        // Not a theme and not the loader's business: no diagnostic for this one.
        fs::create_dir(dir.path().join("backup")).unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("gone.toml"), dir.path().join("ghost.toml"))
            .unwrap();
        #[cfg(unix)]
        let expected = 2;
        #[cfg(not(unix))]
        let expected = 1;

        let registry =
            ThemeRegistry::load_installed(dir.path(), ValidationMode::Compatible).unwrap();
        assert_eq!(user_ids(&registry), ["usable"]);

        let skipped: Vec<_> = registry
            .diagnostics()
            .iter()
            .filter(|d| d.message.contains("not a readable theme file"))
            .collect();
        assert_eq!(skipped.len(), expected, "{skipped:#?}");
        assert!(skipped.iter().all(|d| d.is_warning()));
        assert!(skipped
            .iter()
            .any(|d| d.origin.label().ends_with("bundle.toml")));
    }

    #[test]
    fn a_filename_that_is_not_a_valid_theme_id_is_reported_and_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "My Theme.toml", &minimal("My Theme"));
        write(dir.path(), "usable.toml", &minimal("Usable"));

        let registry =
            ThemeRegistry::load_installed(dir.path(), ValidationMode::Compatible).unwrap();
        assert_eq!(user_ids(&registry), ["usable"]);
        assert!(registry
            .diagnostics()
            .iter()
            .any(|d| d.is_error() && d.message.contains("file name")));
    }

    #[test]
    fn a_user_theme_can_extend_a_sibling_user_theme() {
        let dir = tempfile::tempdir().unwrap();
        // `child` sorts before `parent`, so this only passes if resolution runs
        // after the whole directory has been read.
        write(
            dir.path(),
            "child.toml",
            "schema_version = 1\nname = \"Child\"\nextends = \"parent\"\n",
        );
        write(
            dir.path(),
            "parent.toml",
            "schema_version = 1\nname = \"Parent\"\n\n[semantic]\naccent = \"#ff0000\"\n",
        );

        let registry =
            ThemeRegistry::load_installed(dir.path(), ValidationMode::Compatible).unwrap();
        let child = registry
            .resolved(&ThemeId::parse("child").unwrap())
            .expect("child resolves through its sibling parent");
        assert_eq!(child.semantic.accent, Color::Rgb(0xff, 0, 0));
    }

    #[test]
    fn every_record_keeps_the_original_toml_source() {
        let dir = tempfile::tempdir().unwrap();
        let body = minimal("Kept");
        write(dir.path(), "kept.toml", &body);

        let registry =
            ThemeRegistry::load_installed(dir.path(), ValidationMode::Compatible).unwrap();
        assert_eq!(registry.get("kept").unwrap().toml_source, body);
        for record in registry.records() {
            assert!(
                record.toml_source.contains("schema_version"),
                "{} lost its source",
                record.id
            );
        }
        assert!(registry
            .get("aqua")
            .unwrap()
            .toml_source
            .contains("extends = \"default\""));
    }

    #[test]
    fn a_check_target_outside_the_themes_directory_is_validated_against_the_builtins() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("draft.toml");
        fs::write(
            &file,
            "schema_version = 1\nname = \"Draft\"\n\n[semantic]\naccent = \"#00ff00\"\n",
        )
        .unwrap();

        let registry = ThemeRegistry::load_check_target(&file, ValidationMode::Strict).unwrap();
        let draft = registry.get("draft").unwrap();
        assert_eq!(draft.status, ThemeStatus::Valid);
        assert_eq!(
            registry
                .resolved(&ThemeId::parse("draft").unwrap())
                .unwrap()
                .semantic
                .accent,
            Color::Rgb(0, 0xff, 0)
        );
    }

    #[test]
    fn a_check_target_resolves_a_sibling_parent_and_warns_about_installing_it() {
        // A portable theme package is checked as a package: the target's own
        // directory is the user registry. Both orders are exercised because the
        // parent may sort before or after the child.
        for parent in ["aaa", "zzz"] {
            let dir = tempfile::tempdir().unwrap();
            write(
                dir.path(),
                &format!("{parent}.toml"),
                "schema_version = 1\nname = \"Parent\"\n\n[semantic]\naccent = \"#ff0000\"\n",
            );
            let file = dir.path().join("mid.toml");
            fs::write(
                &file,
                format!("schema_version = 1\nname = \"Child\"\nextends = \"{parent}\"\n"),
            )
            .unwrap();

            let registry = ThemeRegistry::load_check_target(&file, ValidationMode::Strict).unwrap();
            let target = registry.get("mid").unwrap();
            assert_eq!(target.status, ThemeStatus::Valid, "{parent}");
            assert_eq!(
                registry
                    .resolved(&ThemeId::parse("mid").unwrap())
                    .expect("the target resolves through its sibling")
                    .semantic
                    .accent,
                Color::Rgb(0xff, 0, 0)
            );
            let warning = target
                .diagnostics
                .iter()
                .find(|d| d.is_warning() && d.message.contains("sibling file"))
                .unwrap_or_else(|| panic!("{parent}: {:#?}", target.diagnostics));
            assert!(warning.message.contains(parent), "{warning:#?}");
            assert!(
                warning
                    .help
                    .as_deref()
                    .unwrap_or_default()
                    .contains(&format!("{parent}.toml")),
                "{warning:#?}"
            );
        }
    }

    #[test]
    fn a_check_target_inside_its_own_directory_is_read_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "solo.toml", &minimal("Solo"));
        let file = dir.path().join("solo.toml");

        let registry = ThemeRegistry::load_check_target(&file, ValidationMode::Strict).unwrap();
        assert_eq!(user_ids(&registry), ["solo"]);
        assert_eq!(registry.get("solo").unwrap().status, ThemeStatus::Valid);
    }

    /// A directory can be traversable (`--x`) but not listable (`-r-`), which
    /// makes the target readable by name while its siblings are unknowable.
    /// Guessing "no siblings" there would let a theme that needs a sibling
    /// parent check green, so the I/O error has to reach the caller.
    #[cfg(unix)]
    #[test]
    fn a_check_target_whose_directory_cannot_be_listed_is_an_io_error() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let closed = dir.path().join("closed");
        fs::create_dir(&closed).unwrap();
        let file = closed.join("child.toml");
        fs::write(&file, minimal("Child")).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).unwrap();
        // Executable but not readable: `open(file)` works, `readdir` does not.
        fs::set_permissions(&closed, fs::Permissions::from_mode(0o111)).unwrap();

        // Root ignores the permission bits, so the state under test does not
        // exist there; asserting on it would only produce a flake in CI.
        if fs::read_dir(&closed).is_ok() {
            fs::set_permissions(&closed, fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }

        let outcome = ThemeRegistry::load_check_target(&file, ValidationMode::Strict);

        // Restore before the assertion so the temp dir can always be cleaned up.
        fs::set_permissions(&closed, fs::Permissions::from_mode(0o755)).unwrap();

        match outcome {
            Err(ThemeRegistryError::Io { path, .. }) => assert_eq!(path, closed),
            Err(other) => panic!("expected an I/O error, got {other}"),
            Ok(registry) => panic!(
                "an unlistable directory must not check green: {:?}",
                registry.get("child").map(|record| record.status)
            ),
        }
    }

    #[test]
    fn a_check_target_extending_a_builtin_is_not_reported_as_a_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("draft.toml");
        fs::write(
            &file,
            "schema_version = 1\nname = \"Draft\"\nextends = \"aqua\"\n",
        )
        .unwrap();

        let registry = ThemeRegistry::load_check_target(&file, ValidationMode::Strict).unwrap();
        let target = registry.get("draft").unwrap();
        assert_eq!(target.status, ThemeStatus::Valid);
        assert!(
            !target
                .diagnostics
                .iter()
                .any(|d| d.message.contains("sibling")),
            "{:#?}",
            target.diagnostics
        );
    }

    #[test]
    fn a_sibling_of_the_check_target_cannot_shadow_a_builtin() {
        // Only the target itself may stand in for a built-in of the same id;
        // an ordinary neighbour is the collision it would be once installed.
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "aqua.toml", &minimal("Fake Aqua"));
        let file = dir.path().join("draft.toml");
        fs::write(
            &file,
            "schema_version = 1\nname = \"Draft\"\nextends = \"aqua\"\n",
        )
        .unwrap();

        let registry = ThemeRegistry::load_check_target(&file, ValidationMode::Strict).unwrap();
        assert_eq!(registry.get("aqua").unwrap().source, ThemeSource::BuiltIn);
        assert!(registry
            .records()
            .iter()
            .any(|r| r.name == "Fake Aqua" && r.status == ThemeStatus::Invalid));
        // The target inherits the built-in, so it stays valid and is not told
        // to ship a sibling that could never be installed.
        assert_eq!(registry.get("draft").unwrap().status, ThemeStatus::Valid);
        assert!(!registry
            .get("draft")
            .unwrap()
            .diagnostics
            .iter()
            .any(|d| d.message.contains("sibling")));
    }

    #[test]
    fn a_broken_sibling_does_not_invalidate_the_check_target() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "broken.toml", "schema_version = 2\nname = 4\n");
        let file = dir.path().join("draft.toml");
        fs::write(&file, minimal("Draft")).unwrap();

        let registry = ThemeRegistry::load_check_target(&file, ValidationMode::Strict).unwrap();
        let target = registry.get("draft").unwrap();
        assert_eq!(target.status, ThemeStatus::Valid);
        assert!(target.diagnostics.is_empty(), "{:#?}", target.diagnostics);
        assert_eq!(registry.get("broken").unwrap().status, ThemeStatus::Invalid);
    }

    #[test]
    fn a_check_target_with_a_reserved_id_may_still_parent_a_sibling_check() {
        // Checking a copy of a built-in keeps displacing that built-in, and the
        // displacement must not leak into how a sibling of the target resolves.
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "leaf.toml",
            "schema_version = 1\nname = \"Leaf\"\nextends = \"aqua\"\n",
        );
        let file = dir.path().join("aqua.toml");
        fs::write(
            &file,
            "schema_version = 1\nname = \"Draft Aqua\"\n\n[semantic]\naccent = \"#0000ff\"\n",
        )
        .unwrap();

        let registry = ThemeRegistry::load_check_target(&file, ValidationMode::Strict).unwrap();
        let target = registry.get("aqua").unwrap();
        assert_ne!(target.source, ThemeSource::BuiltIn);
        assert_eq!(target.status, ThemeStatus::Valid);
        assert!(target
            .diagnostics
            .iter()
            .any(|d| d.is_warning() && d.help.as_deref().unwrap_or("").contains("aqua-custom")));
        assert_eq!(
            registry
                .resolved(&ThemeId::parse("leaf").unwrap())
                .expect("the sibling resolves against the checked draft")
                .semantic
                .accent,
            Color::Rgb(0, 0, 0xff)
        );
    }

    #[test]
    fn a_check_target_may_carry_a_reserved_id_but_is_warned_about_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("fire.toml");
        fs::write(&file, minimal("Draft Fire")).unwrap();

        let registry = ThemeRegistry::load_check_target(&file, ValidationMode::Strict).unwrap();
        let target = registry.get("fire").unwrap();
        assert_ne!(target.source, ThemeSource::BuiltIn);
        assert_eq!(target.status, ThemeStatus::Valid);
        assert!(target
            .diagnostics
            .iter()
            .any(|d| d.is_warning() && d.help.as_deref().unwrap_or("").contains("fire-custom")));
    }
}
