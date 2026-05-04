//! Per-context repository registry: `<context>/repos.yaml`.
//!
//! Lists every repo the user wants to be able to target with a plan.
//! Each entry maps a stable slug (the `repo_slug` half of every
//! `ComponentRef`) to a clone URL plus an optional local checkout path.
//!
//! ## Path representation
//!
//! `local_path` is the user's regular working checkout. It is stored as
//! an absolute path on disk and held as an absolute, cleaned `PathBuf`
//! in memory. `local_path` is optional; when absent, callers that need
//! a working tree fall back to a context-cache clone.
//!
//! ## Slug stability
//!
//! Each top-level key in `repos.yaml` is the `repo_slug` baked into
//! every `ComponentRef`, target, and memory attribution downstream.
//! Renames cascade — there is intentionally no `rename` verb in v1.
//!
//! All read/write goes through `load_or_empty` / `save_atomic` so the
//! single `schema_version` field is always applied correctly and every
//! write is tmp-file-plus-rename.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use indexmap::IndexMap;
use path_clean::PathClean;
use serde::{Deserialize, Serialize};

use crate::bail_with;
use crate::cli::error_context::{CodedError, ResultExt};
use crate::cli::{ErrorCode, OutputFormat};

pub const REGISTRY_FILE: &str = "repos.yaml";

/// Only schema version in circulation; bump when the on-disk shape
/// changes incompatibly.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReposRegistry {
    pub schema_version: u32,
    pub repos: IndexMap<String, RepoEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoEntry {
    pub url: String,
    pub local_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawReposRegistry {
    schema_version: u32,
    #[serde(default)]
    repos: IndexMap<String, RawRepoEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawRepoEntry {
    url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    local_path: Option<PathBuf>,
}

impl Default for ReposRegistry {
    fn default() -> Self {
        ReposRegistry {
            schema_version: SCHEMA_VERSION,
            repos: IndexMap::new(),
        }
    }
}

impl ReposRegistry {
    pub fn get(&self, slug: &str) -> Option<&RepoEntry> {
        self.repos.get(slug)
    }
}

pub fn load_or_empty(context_root: &Path) -> Result<ReposRegistry> {
    let path = context_root.join(REGISTRY_FILE);
    if !path.exists() {
        return Ok(ReposRegistry::default());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))
        .with_code(ErrorCode::IoError)?;
    let raw: RawReposRegistry = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))
        .with_code(ErrorCode::InvalidInput)?;
    if raw.schema_version != SCHEMA_VERSION {
        bail_with!(
            ErrorCode::Conflict,
            "{} has schema_version {} but this ravel-lite expects {}; aborting to avoid data loss",
            path.display(),
            raw.schema_version,
            SCHEMA_VERSION
        );
    }
    let repos = raw
        .repos
        .into_iter()
        .map(|(slug, raw_entry)| {
            (
                slug,
                RepoEntry {
                    url: raw_entry.url,
                    local_path: raw_entry.local_path.map(|p| p.clean()),
                },
            )
        })
        .collect();
    Ok(ReposRegistry {
        schema_version: raw.schema_version,
        repos,
    })
}

pub fn save_atomic(context_root: &Path, registry: &ReposRegistry) -> Result<()> {
    let path = context_root.join(REGISTRY_FILE);
    let yaml = serialise_registry(registry)?;
    let tmp = context_root.join(format!(".{REGISTRY_FILE}.tmp"));
    std::fs::write(&tmp, yaml.as_bytes())
        .with_context(|| format!("Failed to write temp file {}", tmp.display()))
        .with_code(ErrorCode::IoError)?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("Failed to rename {} to {}", tmp.display(), path.display()))
        .with_code(ErrorCode::IoError)?;
    Ok(())
}

fn serialise_registry(registry: &ReposRegistry) -> Result<String> {
    let raw = to_raw_registry(registry);
    serde_yaml::to_string(&raw).context("Failed to serialise repos registry to YAML")
}

/// Insert a new repo entry. Errors if the slug is already registered.
/// Does not persist; the caller saves on `Ok`.
pub fn try_add(
    registry: &mut ReposRegistry,
    slug: &str,
    url: &str,
    local_path: Option<&Path>,
) -> Result<()> {
    if registry.repos.contains_key(slug) {
        bail_with!(
            ErrorCode::Conflict,
            "repo slug '{}' is already registered; pick a different name",
            slug
        );
    }
    let cleaned = local_path
        .map(|p| {
            std::path::absolute(p)
                .with_context(|| {
                    format!("Failed to resolve local_path {} to an absolute path", p.display())
                })
                .map(|abs| abs.clean())
        })
        .transpose()?;
    registry.repos.insert(
        slug.to_string(),
        RepoEntry {
            url: url.to_string(),
            local_path: cleaned,
        },
    );
    Ok(())
}

// ---------- CLI handlers ----------

pub fn run_list(context_root: &Path, format: OutputFormat) -> Result<()> {
    let registry = load_or_empty(context_root)?;
    let raw = to_raw_registry(&registry);
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(&raw)
            .context("Failed to serialise repos registry to YAML")?,
        OutputFormat::Json => serde_json::to_string_pretty(&raw)
            .context("Failed to serialise repos registry to JSON")?
            + "\n",
        OutputFormat::Markdown => bail_with!(
            ErrorCode::InvalidInput,
            "format `markdown` is not supported on `repo list`; supported: yaml, json"
        ),
    };
    print!("{serialised}");
    Ok(())
}

fn to_raw_registry(registry: &ReposRegistry) -> RawReposRegistry {
    RawReposRegistry {
        schema_version: registry.schema_version,
        repos: registry
            .repos
            .iter()
            .map(|(slug, entry)| {
                (
                    slug.clone(),
                    RawRepoEntry {
                        url: entry.url.clone(),
                        local_path: entry.local_path.clone(),
                    },
                )
            })
            .collect(),
    }
}

pub fn run_add(
    context_root: &Path,
    slug: &str,
    url: &str,
    local_path: Option<&Path>,
) -> Result<()> {
    let mut registry = load_or_empty(context_root)?;
    try_add(&mut registry, slug, url, local_path)?;
    save_atomic(context_root, &registry)
}

pub fn run_remove(context_root: &Path, slug: &str) -> Result<()> {
    let mut registry = load_or_empty(context_root)?;
    if registry.repos.shift_remove(slug).is_none() {
        bail_with!(
            ErrorCode::NotFound,
            "no repo named '{}' in registry at {}",
            slug,
            context_root.join(REGISTRY_FILE).display()
        );
    }
    save_atomic(context_root, &registry)
}

/// Filename of the legacy per-user catalog. Centralised so the
/// migration-error helper and any future cleanup path stay in sync.
pub const LEGACY_PROJECTS_FILENAME: &str = "projects.yaml";

/// Single source of truth for the wording every consumer surfaces when a
/// pre-cutover `projects.yaml` is detected without a populated
/// `repos.yaml`. Centralising the message guards against drift between
/// call sites.
pub fn migrate_projects_yaml_error() -> anyhow::Error {
    anyhow::Error::new(CodedError {
        code: ErrorCode::InvalidInput,
        message: format!(
            "{} is no longer supported. The per-context repo registry moved to {} \
             in the architecture-next migration. Migrate by running\n\
             \n  \
             ravel-lite repo add <slug> --url <git-url> [--local-path <path>]\n\
             \n\
             for each entry in your existing {}, then delete the old file. \
             See docs/architecture-next.md §\"The repo registry\".",
            LEGACY_PROJECTS_FILENAME, REGISTRY_FILE, LEGACY_PROJECTS_FILENAME,
        ),
    })
}

/// Loader used by every consumer that previously read `projects.yaml`.
/// Returns the registry on success; if the registry would be empty AND
/// the legacy `projects.yaml` is present in the same context dir, fails
/// with the canonical migration error so the user gets actionable advice
/// instead of a misleading "empty catalog" downstream message.
pub fn load_for_lookup(context_root: &Path) -> Result<ReposRegistry> {
    let registry = load_or_empty(context_root)?;
    if registry.repos.is_empty()
        && context_root.join(LEGACY_PROJECTS_FILENAME).exists()
    {
        return Err(migrate_projects_yaml_error());
    }
    Ok(registry)
}

/// Look up a repo entry by absolute `local_path`. Returns `(slug, entry)`
/// for the first match. `None` when no entry has that local_path. Used
/// by consumers that only know a working-tree path (e.g.
/// `state related-components list --plan`) and need to recover the slug.
pub fn find_by_local_path<'a>(
    registry: &'a ReposRegistry,
    local_path: &Path,
) -> Option<(&'a str, &'a RepoEntry)> {
    let cleaned = local_path.to_path_buf().clean();
    registry.repos.iter().find_map(|(slug, entry)| {
        entry
            .local_path
            .as_deref()
            .filter(|p| *p == cleaned)
            .map(|_| (slug.as_str(), entry))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_or_empty_returns_empty_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let registry = load_or_empty(tmp.path()).unwrap();
        assert_eq!(registry.schema_version, SCHEMA_VERSION);
        assert!(registry.repos.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = TempDir::new().unwrap();
        let mut registry = ReposRegistry::default();
        try_add(
            &mut registry,
            "atlas",
            "git@github.com:antony/atlas.git",
            Some(&tmp.path().join("atlas")),
        )
        .unwrap();
        try_add(
            &mut registry,
            "ravel-lite",
            "git@github.com:Linkuistics/Ravel-Lite.git",
            None,
        )
        .unwrap();

        save_atomic(tmp.path(), &registry).unwrap();
        let loaded = load_or_empty(tmp.path()).unwrap();
        assert_eq!(loaded, registry);
    }

    #[test]
    fn insertion_order_is_preserved() {
        let tmp = TempDir::new().unwrap();
        let mut registry = ReposRegistry::default();
        try_add(&mut registry, "zeta", "u1", None).unwrap();
        try_add(&mut registry, "alpha", "u2", None).unwrap();
        try_add(&mut registry, "mu", "u3", None).unwrap();

        save_atomic(tmp.path(), &registry).unwrap();
        let loaded = load_or_empty(tmp.path()).unwrap();
        let slugs: Vec<&str> = loaded.repos.keys().map(String::as_str).collect();
        assert_eq!(slugs, vec!["zeta", "alpha", "mu"]);
    }

    #[test]
    fn load_rejects_unknown_schema_version() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join(REGISTRY_FILE),
            "schema_version: 99\nrepos: {}\n",
        )
        .unwrap();
        let err = load_or_empty(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"));
        assert!(msg.contains("99"));
    }

    #[test]
    fn try_add_rejects_duplicate_slug() {
        let mut registry = ReposRegistry::default();
        try_add(&mut registry, "atlas", "u1", None).unwrap();
        let err = try_add(&mut registry, "atlas", "u2", None).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("already registered"));
    }

    #[test]
    fn local_path_is_stored_absolute() {
        let tmp = TempDir::new().unwrap();
        let local = tmp.path().join("checkout");
        std::fs::create_dir_all(&local).unwrap();
        let mut registry = ReposRegistry::default();
        try_add(&mut registry, "atlas", "u", Some(&local)).unwrap();
        let stored = registry.repos.get("atlas").unwrap().local_path.as_ref().unwrap();
        assert!(stored.is_absolute(), "local_path should be absolute, was {stored:?}");
    }

    #[test]
    fn run_remove_errors_when_slug_missing() {
        let tmp = TempDir::new().unwrap();
        let err = run_remove(tmp.path(), "nope").unwrap_err();
        assert!(format!("{err:#}").contains("no repo named 'nope'"));
    }

    #[test]
    fn run_add_then_remove_round_trips() {
        let tmp = TempDir::new().unwrap();
        run_add(tmp.path(), "atlas", "git@example/atlas.git", None).unwrap();
        let after_add = load_or_empty(tmp.path()).unwrap();
        assert!(after_add.repos.contains_key("atlas"));

        run_remove(tmp.path(), "atlas").unwrap();
        let after_remove = load_or_empty(tmp.path()).unwrap();
        assert!(!after_remove.repos.contains_key("atlas"));
    }

    #[test]
    fn local_path_omitted_serialises_without_field() {
        let mut registry = ReposRegistry::default();
        try_add(&mut registry, "atlas", "u", None).unwrap();
        let yaml = serialise_registry(&registry).unwrap();
        assert!(
            !yaml.contains("local_path"),
            "absent local_path should not appear in YAML; got:\n{yaml}"
        );
    }

    #[test]
    fn migrate_projects_yaml_error_is_actionable() {
        let err = migrate_projects_yaml_error();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("projects.yaml is no longer supported"),
            "message must explain projects.yaml is retired; got:\n{msg}"
        );
        assert!(
            msg.contains("repos.yaml"),
            "message must name the replacement file; got:\n{msg}"
        );
        assert!(
            msg.contains("ravel-lite repo add"),
            "message must show the migration command shape; got:\n{msg}"
        );
        assert!(
            msg.contains("docs/architecture-next.md"),
            "message must point at the design doc; got:\n{msg}"
        );
    }

    #[test]
    fn load_for_lookup_returns_registry_when_repos_yaml_populated() {
        let tmp = TempDir::new().unwrap();
        run_add(tmp.path(), "atlas", "u", None).unwrap();
        let loaded = load_for_lookup(tmp.path()).unwrap();
        assert!(loaded.repos.contains_key("atlas"));
    }

    #[test]
    fn load_for_lookup_returns_empty_when_neither_file_present() {
        let tmp = TempDir::new().unwrap();
        let loaded = load_for_lookup(tmp.path()).unwrap();
        assert!(loaded.repos.is_empty());
    }

    #[test]
    fn load_for_lookup_emits_migration_error_when_legacy_projects_yaml_present() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("projects.yaml"),
            "schema_version: 1\nprojects: []\n",
        )
        .unwrap();
        let err = load_for_lookup(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("projects.yaml is no longer supported"));
        assert!(msg.contains("ravel-lite repo add"));
    }

    #[test]
    fn load_for_lookup_does_not_emit_migration_error_when_repos_yaml_has_entries() {
        // User has begun migrating: legacy file still on disk but at
        // least one repo entry is registered. Treat as migrated.
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("projects.yaml"),
            "schema_version: 1\nprojects: []\n",
        )
        .unwrap();
        run_add(tmp.path(), "atlas", "u", None).unwrap();
        let loaded = load_for_lookup(tmp.path()).unwrap();
        assert!(loaded.repos.contains_key("atlas"));
    }

    #[test]
    fn find_by_local_path_matches_registered_path() {
        let tmp = TempDir::new().unwrap();
        let local = tmp.path().join("checkout");
        std::fs::create_dir_all(&local).unwrap();
        let mut registry = ReposRegistry::default();
        try_add(&mut registry, "atlas", "u", Some(&local)).unwrap();

        let hit = find_by_local_path(&registry, &local);
        assert!(hit.is_some(), "exact-match lookup should hit");
        let (slug, _entry) = hit.unwrap();
        assert_eq!(slug, "atlas");
    }

    #[test]
    fn find_by_local_path_returns_none_for_unmatched_path() {
        let tmp = TempDir::new().unwrap();
        let local = tmp.path().join("checkout");
        std::fs::create_dir_all(&local).unwrap();
        let mut registry = ReposRegistry::default();
        try_add(&mut registry, "atlas", "u", Some(&local)).unwrap();

        assert!(find_by_local_path(&registry, &tmp.path().join("other")).is_none());
    }

    #[test]
    fn find_by_local_path_skips_entries_without_local_path() {
        let mut registry = ReposRegistry::default();
        try_add(&mut registry, "url-only", "u", None).unwrap();
        // Bare lookup against any path: must not panic, must not match.
        assert!(find_by_local_path(&registry, Path::new("/anything")).is_none());
    }
}
