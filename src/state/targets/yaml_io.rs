//! Atomic read/write of `<plan>/targets.yaml`.
//!
//! `targets.yaml` is runtime state — born when the runner mounts the
//! first worktree, not at plan creation. Mirrors the
//! `findings::yaml_io::read_findings` "missing = empty" pattern rather
//! than the strict "missing is an error" pattern used by `intents`,
//! `backlog`, and `memory`, because callers should be able to ask
//! "what is mounted?" without first checking whether anything has been
//! mounted yet.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

use super::schema::{TargetsFile, TARGETS_SCHEMA_VERSION};
use crate::component_ref::ComponentRef;
use crate::state::filenames::TARGETS_FILENAME;

pub fn targets_path(plan_dir: &Path) -> PathBuf {
    plan_dir.join(TARGETS_FILENAME)
}

/// Read `<plan>/targets.yaml`. Returns an empty (default) document when
/// the file does not yet exist — see module docs.
pub fn read_targets(plan_dir: &Path) -> Result<TargetsFile> {
    let path = targets_path(plan_dir);
    if !path.exists() {
        return Ok(TargetsFile::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let parsed: TargetsFile = serde_yaml::from_str(&text)
        .with_context(|| format!("Failed to parse {} as {TARGETS_FILENAME} schema", path.display()))?;
    if parsed.schema_version != TARGETS_SCHEMA_VERSION {
        bail!(
            "{} declares schema_version {}, expected {}.",
            path.display(),
            parsed.schema_version,
            TARGETS_SCHEMA_VERSION
        );
    }
    Ok(parsed)
}

pub fn write_targets(plan_dir: &Path, targets: &TargetsFile) -> Result<()> {
    let path = targets_path(plan_dir);
    let yaml = serde_yaml::to_string(targets)
        .with_context(|| format!("Failed to serialise {TARGETS_FILENAME}"))?;
    atomic_write(&path, yaml.as_bytes())
}

/// Mounted worktree path for the named target, joined onto `plan_dir`.
/// Errors when no `Target` row in `targets.yaml` matches the
/// `(repo_slug, component_id)` reference — meaning the caller asked for
/// a worktree that hasn't been mounted yet.
///
/// Used by the `commits.yaml` applier (architecture-next §Commits) to
/// resolve each commit's `target: ComponentRef` to the worktree it must
/// `chdir` into. The result preserves the absoluteness of `plan_dir`:
/// pass an absolute plan_dir to get an absolute worktree path.
pub fn resolve_target_worktree(plan_dir: &Path, target: &ComponentRef) -> Result<PathBuf> {
    let targets = read_targets(plan_dir)?;
    let entry = targets
        .targets
        .iter()
        .find(|t| t.repo_slug == target.repo_slug && t.component_id == target.component_id)
        .ok_or_else(|| {
            anyhow!(
                "{target} is not a mounted target in {}; \
                 add it via target-requests.yaml so the next phase boundary mounts it",
                plan_dir.display()
            )
        })?;
    Ok(plan_dir.join(&entry.working_root))
}

/// Absolute filesystem paths of every mounted target's `working_root`
/// that is NOT already a descendant of `cwd`. Intended to feed
/// `--add-dir <path>` arguments to claude-code agent spawns.
///
/// The cwd-descendant filter avoids redundant flags: claude already
/// trusts its launch cwd, so emitting `--add-dir` for a path inside cwd
/// adds no permission and may trigger an unseen trust-grant modal that
/// hangs the spawn (see the comment in
/// `agent::claude_code::invoke_interactive`).
pub fn mounted_worktree_add_dirs(plan_dir: &Path, cwd: &Path) -> Result<Vec<String>> {
    let targets = read_targets(plan_dir)?;
    let mut out = Vec::new();
    for t in &targets.targets {
        let abs = plan_dir.join(&t.working_root);
        if abs.starts_with(cwd) {
            continue;
        }
        let s = abs.to_str().with_context(|| {
            format!("working_root path is not valid UTF-8: {}", abs.display())
        })?;
        out.push(s.to_string());
    }
    Ok(out)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    let file_name = path
        .file_name()
        .with_context(|| format!("{} has no file name", path.display()))?
        .to_string_lossy();
    let tmp = parent.join(format!(".{file_name}.tmp"));
    std::fs::write(&tmp, bytes)
        .with_context(|| format!("Failed to write temp file {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("Failed to rename {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::targets::schema::Target;
    use tempfile::TempDir;

    fn sample_target() -> Target {
        Target {
            repo_slug: "atlas".into(),
            component_id: "atlas-ontology".into(),
            working_root: ".worktrees/atlas".into(),
            branch: "ravel-lite/sample-plan/main".into(),
            path_segments: vec!["crates".into(), "atlas-ontology".into()],
        }
    }

    #[test]
    fn read_returns_empty_default_when_targets_yaml_is_absent() {
        let tmp = TempDir::new().unwrap();
        let parsed = read_targets(tmp.path()).unwrap();
        assert_eq!(parsed.schema_version, TARGETS_SCHEMA_VERSION);
        assert!(parsed.targets.is_empty());
    }

    #[test]
    fn write_then_read_round_trips_target_fields() {
        let tmp = TempDir::new().unwrap();
        let targets = TargetsFile {
            schema_version: TARGETS_SCHEMA_VERSION,
            targets: vec![sample_target()],
        };
        write_targets(tmp.path(), &targets).unwrap();

        let round_tripped = read_targets(tmp.path()).unwrap();
        assert_eq!(round_tripped, targets);
    }

    #[test]
    fn read_errors_on_schema_version_mismatch() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(targets_path(tmp.path()), "schema_version: 99\ntargets: []\n").unwrap();
        let err = read_targets(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"), "error must cite schema_version: {msg}");
        assert!(msg.contains("99"), "error must show found version: {msg}");
    }

    #[test]
    fn write_uses_atomic_rename_via_dot_tmp_path() {
        let tmp = TempDir::new().unwrap();
        let targets = TargetsFile {
            schema_version: TARGETS_SCHEMA_VERSION,
            targets: vec![sample_target()],
        };
        write_targets(tmp.path(), &targets).unwrap();

        let final_path = targets_path(tmp.path());
        assert!(final_path.exists(), "final file must be present after write");
        let tmp_path = tmp.path().join(format!(".{TARGETS_FILENAME}.tmp"));
        assert!(!tmp_path.exists(), "temp file must be renamed away after write");
    }

    #[test]
    fn mounted_worktree_add_dirs_returns_empty_when_targets_yaml_absent() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        let dirs = mounted_worktree_add_dirs(tmp.path(), cwd).unwrap();
        assert!(dirs.is_empty());
    }

    #[test]
    fn mounted_worktree_add_dirs_skips_paths_inside_cwd() {
        // V1 layout: plan_dir is a descendant of cwd (project_dir),
        // worktrees live under plan_dir, so they are already reachable
        // from cwd and emitting --add-dir for them would be redundant
        // and could trigger an unseen claude trust-grant modal.
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        let plan_dir = cwd.join("LLM_STATE/sample-plan");
        std::fs::create_dir_all(&plan_dir).unwrap();
        let targets = TargetsFile {
            schema_version: TARGETS_SCHEMA_VERSION,
            targets: vec![sample_target()],
        };
        write_targets(&plan_dir, &targets).unwrap();

        let dirs = mounted_worktree_add_dirs(&plan_dir, cwd).unwrap();
        assert!(
            dirs.is_empty(),
            "worktrees under cwd must be filtered out; got {dirs:?}"
        );
    }

    #[test]
    fn mounted_worktree_add_dirs_returns_paths_outside_cwd() {
        // V2 layout: plan_dir lives outside the agent's cwd, so worktree
        // paths under plan_dir are not reachable without --add-dir.
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().join("project");
        let plan_dir = tmp.path().join("context/plans/sample-plan");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&plan_dir).unwrap();
        let targets = TargetsFile {
            schema_version: TARGETS_SCHEMA_VERSION,
            targets: vec![sample_target()],
        };
        write_targets(&plan_dir, &targets).unwrap();

        let dirs = mounted_worktree_add_dirs(&plan_dir, &cwd).unwrap();
        let expected = plan_dir.join(".worktrees/atlas").to_string_lossy().to_string();
        assert_eq!(dirs, vec![expected]);
    }

    #[test]
    fn resolve_target_worktree_returns_plan_relative_path_for_mounted_target() {
        // Common-path lookup: a target row with the matching ComponentRef
        // produces `plan_dir/<working_root>`, preserving plan_dir's
        // absoluteness so callers feeding the result to `git -C` get an
        // absolute path when they passed one in.
        let tmp = TempDir::new().unwrap();
        let plan_dir = tmp.path();
        let targets = TargetsFile {
            schema_version: TARGETS_SCHEMA_VERSION,
            targets: vec![sample_target()],
        };
        write_targets(plan_dir, &targets).unwrap();

        let r = ComponentRef::new("atlas", "atlas-ontology");
        let resolved = resolve_target_worktree(plan_dir, &r).unwrap();
        assert_eq!(resolved, plan_dir.join(".worktrees/atlas"));
    }

    #[test]
    fn resolve_target_worktree_errors_when_component_not_mounted() {
        // Caller asked for a worktree the runner hasn't mounted yet. Must
        // surface a clear, actionable error pointing at target-requests
        // rather than silently returning a non-existent path.
        let tmp = TempDir::new().unwrap();
        let plan_dir = tmp.path();
        let targets = TargetsFile {
            schema_version: TARGETS_SCHEMA_VERSION,
            targets: vec![sample_target()],
        };
        write_targets(plan_dir, &targets).unwrap();

        let r = ComponentRef::new("ravel-lite", "phase-loop");
        let err = resolve_target_worktree(plan_dir, &r).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("ravel-lite:phase-loop"), "must cite the missing ref: {msg}");
        assert!(msg.contains("target-requests"), "must point user at the mount path: {msg}");
    }

    #[test]
    fn resolve_target_worktree_errors_when_targets_yaml_absent() {
        // Pre-mount state: no `targets.yaml` on disk. `read_targets`
        // returns an empty default, so the resolver still hits the
        // not-mounted path with the same error contract — the caller
        // does not need to special-case "file missing" vs "row missing".
        let tmp = TempDir::new().unwrap();
        let r = ComponentRef::new("atlas", "atlas-ontology");
        let err = resolve_target_worktree(tmp.path(), &r).unwrap_err();
        assert!(format!("{err:#}").contains("atlas:atlas-ontology"));
    }

    #[test]
    fn resolve_target_worktree_distinguishes_components_sharing_a_worktree() {
        // Two components can share one repo's worktree; the resolver
        // returns the same path for both because `working_root` is
        // identical, but matches on the full ComponentRef so the wrong
        // component_id can't accidentally bind to the right worktree.
        let tmp = TempDir::new().unwrap();
        let plan_dir = tmp.path();
        let mut second = sample_target();
        second.component_id = "atlas-discovery".into();
        let targets = TargetsFile {
            schema_version: TARGETS_SCHEMA_VERSION,
            targets: vec![sample_target(), second],
        };
        write_targets(plan_dir, &targets).unwrap();

        let ontology = resolve_target_worktree(
            plan_dir,
            &ComponentRef::new("atlas", "atlas-ontology"),
        )
        .unwrap();
        let discovery = resolve_target_worktree(
            plan_dir,
            &ComponentRef::new("atlas", "atlas-discovery"),
        )
        .unwrap();
        assert_eq!(ontology, discovery, "shared worktree means equal paths");

        let unmounted = resolve_target_worktree(
            plan_dir,
            &ComponentRef::new("atlas", "not-a-component"),
        );
        assert!(unmounted.is_err());
    }

    #[test]
    fn mounted_worktree_add_dirs_emits_one_path_per_target() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().join("project");
        let plan_dir = tmp.path().join("plan");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&plan_dir).unwrap();

        let mut t2 = sample_target();
        t2.repo_slug = "ravel".into();
        t2.component_id = "ravel-core".into();
        t2.working_root = ".worktrees/ravel".into();
        let targets = TargetsFile {
            schema_version: TARGETS_SCHEMA_VERSION,
            targets: vec![sample_target(), t2],
        };
        write_targets(&plan_dir, &targets).unwrap();

        let dirs = mounted_worktree_add_dirs(&plan_dir, &cwd).unwrap();
        assert_eq!(dirs.len(), 2);
        assert!(dirs[0].ends_with(".worktrees/atlas"));
        assert!(dirs[1].ends_with(".worktrees/ravel"));
    }
}
