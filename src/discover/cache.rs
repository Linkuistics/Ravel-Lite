//! Per-project surface cache at `<config-dir>/discover-cache/<name>.yaml`.
//!
//! Atomic write via tmp-file-plus-rename. Schema-version guarded on
//! load. Cache directory is created lazily on first save.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::schema::{SurfaceFile, SURFACE_SCHEMA_VERSION};

pub const CACHE_DIR: &str = "discover-cache";

pub fn cache_dir(config_root: &Path) -> PathBuf {
    config_root.join(CACHE_DIR)
}

pub fn cache_path(config_root: &Path, project_name: &str) -> PathBuf {
    cache_dir(config_root).join(format!("{project_name}.yaml"))
}

/// Load a cached surface record, or `Ok(None)` if the file does not exist.
/// A present-but-unparseable file is a hard error (it would silently
/// vanish otherwise).
pub fn load(config_root: &Path, project_name: &str) -> Result<Option<SurfaceFile>> {
    let path = cache_path(config_root, project_name);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let file: SurfaceFile = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if file.schema_version != SURFACE_SCHEMA_VERSION {
        bail!(
            "{} has schema_version {} but this ravel-lite expects {}; \
             delete the cache file to force re-analysis",
            path.display(),
            file.schema_version,
            SURFACE_SCHEMA_VERSION
        );
    }
    Ok(Some(file))
}

pub fn save_atomic(config_root: &Path, file: &SurfaceFile) -> Result<()> {
    let dir = cache_dir(config_root);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create cache dir {}", dir.display()))?;
    let path = cache_path(config_root, &file.project);
    let tmp = dir.join(format!(".{}.tmp", file.project));
    let yaml = serde_yaml::to_string(file).context("failed to serialise SurfaceFile")?;
    std::fs::write(&tmp, yaml.as_bytes())
        .with_context(|| format!("failed to write temp file {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("failed to rename {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Rename the cache file for `old` to `new`. Silently succeeds when no
/// cache file exists under `old` (a rename of a project that has never
/// been analysed is still valid).
pub fn rename(config_root: &Path, old: &str, new: &str) -> Result<()> {
    let from = cache_path(config_root, old);
    if !from.exists() {
        return Ok(());
    }
    let to = cache_path(config_root, new);
    std::fs::rename(&from, &to)
        .with_context(|| format!("failed to rename {} to {}", from.display(), to.display()))
}

#[cfg(test)]
mod tests {
    use super::super::schema::SurfaceRecord;
    use super::*;
    use tempfile::TempDir;

    fn sample(name: &str, sha: &str) -> SurfaceFile {
        SurfaceFile {
            schema_version: SURFACE_SCHEMA_VERSION,
            project: name.to_string(),
            tree_sha: sha.to_string(),
            dirty_hash: String::new(),
            analysed_at: "2026-04-22T00:00:00Z".to_string(),
            surface: SurfaceRecord::default(),
        }
    }

    #[test]
    fn load_returns_none_when_file_absent() {
        let tmp = TempDir::new().unwrap();
        assert!(load(tmp.path(), "Nobody").unwrap().is_none());
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = TempDir::new().unwrap();
        let file = sample("Alpha", "abc");
        save_atomic(tmp.path(), &file).unwrap();
        let loaded = load(tmp.path(), "Alpha").unwrap().unwrap();
        assert_eq!(loaded, file);
    }

    #[test]
    fn save_creates_cache_dir_lazily() {
        let tmp = TempDir::new().unwrap();
        assert!(!cache_dir(tmp.path()).exists());
        save_atomic(tmp.path(), &sample("Alpha", "abc")).unwrap();
        assert!(cache_dir(tmp.path()).is_dir());
    }

    #[test]
    fn load_rejects_mismatched_schema_version() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(cache_dir(tmp.path())).unwrap();
        std::fs::write(
            cache_path(tmp.path(), "Alpha"),
            "schema_version: 99\nproject: Alpha\ntree_sha: x\nanalysed_at: t\nsurface: {}\n",
        )
        .unwrap();
        let err = load(tmp.path(), "Alpha").unwrap_err();
        assert!(format!("{err:#}").contains("schema_version"));
    }

    #[test]
    fn rename_moves_cache_file() {
        let tmp = TempDir::new().unwrap();
        save_atomic(tmp.path(), &sample("Old", "x")).unwrap();
        rename(tmp.path(), "Old", "New").unwrap();
        assert!(!cache_path(tmp.path(), "Old").exists());
        let loaded = load(tmp.path(), "New").unwrap().unwrap();
        assert_eq!(loaded.project, "Old");
    }

    #[test]
    fn rename_silently_succeeds_when_source_absent() {
        let tmp = TempDir::new().unwrap();
        rename(tmp.path(), "Ghost", "Phantom").unwrap();
    }
}
