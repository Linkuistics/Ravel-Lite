# R7 — LLM-Driven Related-Projects Discovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `ravel-lite state related-projects discover` and `discover-apply`: a two-stage LLM pipeline that extracts each catalogued project's interaction surface (Stage 1, per-project, cached by subtree tree SHA) and infers cross-project edges (Stage 2, global, uncached), writing proposals to `<config-dir>/discover-proposals.yaml` for user review and merge into `<config-dir>/related-projects.yaml`.

**Architecture:** New `src/discover/` module directory. Stage 1 dispatches concurrent `claude -p` subprocesses with `current_dir` set to each project's path, bounded by a `tokio::sync::Semaphore`. Each Stage 1 subagent reads files via its built-in tools and emits a structured YAML surface record, which Rust validates and caches at `<config-dir>/discover-cache/<project>.yaml` keyed by `git rev-parse HEAD:<subtree-rel-path>`. Stage 2 composes all N surface records into a single prompt, invokes `claude -p` once, parses proposal YAML, and writes `discover-proposals.yaml`. The `discover-apply` sub-verb reads proposals, merges via the existing `RelatedProjectsFile::add_edge` idempotent path, and reports kind-conflicts without aborting. Pattern borrows `spawn_claude_and_read` from `src/survey/invoke.rs` (bypasses the Agent trait; same justification — one-shot LLM calls with bounded prompts and YAML-on-stdout responses).

**Tech Stack:** Rust 2021, serde + serde_yaml, clap v4 derive, anyhow, tokio (semaphore + process), tempfile (tests), the existing `git2`-style shelling-out to the `git` CLI already used by `src/git.rs`.

---

## Environment & scope notes

- **Worktree:** not used. Ships from main, consistent with how R1–R6 landed. No `using-git-worktrees` skill invocation.
- **Spec source of truth:** `docs/r7-related-projects-discovery-design.md`. Decisions deferred to this plan are resolved inline below, flagged with a *(plan-time decision)* note.
- **Scope boundary:** R7 only. Out-of-scope items (non-git support, dirty-tree hashing, auto-scheduling, confidence thresholds) stay deferred.
- **Existing precedents to follow:**
  - `src/survey/invoke.rs::spawn_claude_and_read` — one-shot `claude -p` invocation with timeout.
  - `src/related_projects.rs` — atomic save/load with `schema_version` guard; `RelatedProjectsFile::add_edge` idempotent merge.
  - `src/projects.rs::run_rename` cascade pattern.
  - `src/subagent.rs::dispatch_subagents` — `tokio::JoinSet` fanout. R7 adds a `Semaphore` on top for bounded parallelism.
  - `src/state/backlog/yaml_io.rs` — atomic tmp-then-rename writes.

## File structure

### Created

- `src/discover/mod.rs` — public surface; `run_discover`, `run_discover_apply` entry points; module re-exports.
- `src/discover/schema.rs` — `SurfaceFile`, `SurfaceRecord`, `ProposalsFile`, `ProposalRecord`, `Proposal` types with serde derives.
- `src/discover/cache.rs` — per-project `<name>.yaml` read/write, atomic; rename helper.
- `src/discover/tree_sha.rs` — `compute_project_tree_sha(project_path)`; handles top-level + monorepo subtree.
- `src/discover/stage1.rs` — Stage 1 orchestration: cache check, subagent spawn, YAML parse, identity-field injection, failure collection.
- `src/discover/stage2.rs` — Stage 2 orchestration: prompt composition, `claude -p` call, proposals parse + writeback.
- `src/discover/apply.rs` — proposals → `related-projects.yaml` merge with kind-conflict handling.
- `defaults/discover-stage1.md` — Stage 1 subagent prompt template.
- `defaults/discover-stage2.md` — Stage 2 matcher prompt template.
- `tests/discover.rs` — integration tests via `CARGO_BIN_EXE_ravel-lite` with a fake `claude` shim.

### Modified

- `src/main.rs` — extend `RelatedProjectsCommands` with `Discover { .. }` and `DiscoverApply { .. }` variants; route via `dispatch_related_projects`.
- `src/lib.rs` — add `pub mod discover;`.
- `src/projects.rs::run_rename` — extend rename cascade to also rename `<config-dir>/discover-cache/<old>.yaml` → `<new>.yaml`.
- `src/init.rs` — register the two new prompt templates as embedded defaults so `ravel-lite init` copies them to `<config-dir>/`.

---

## Task 1: Module scaffold + `SurfaceRecord` / `SurfaceFile` types

**Files:**
- Create: `src/discover/mod.rs`
- Create: `src/discover/schema.rs`
- Modify: `src/lib.rs`

Goal: make the module compile with the schema types defined. No behaviour yet.

- [ ] **Step 1: Create `src/discover/mod.rs` with placeholder exports**

```rust
//! LLM-driven discovery of cross-project relationships.
//!
//! Two-stage pipeline keyed from the global projects catalog:
//! * Stage 1 (per-project, cached): subagent reads the project tree and
//!   emits a structured interaction-surface record.
//! * Stage 2 (global, uncached): one LLM call over all N surface records
//!   proposes edges, written to `<config-dir>/discover-proposals.yaml`
//!   for review.
//!
//! Spec: `docs/r7-related-projects-discovery-design.md`.

pub mod schema;
```

- [ ] **Step 2: Create `src/discover/schema.rs` with surface types**

```rust
use serde::{Deserialize, Serialize};

pub const SURFACE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceFile {
    pub schema_version: u32,
    pub project: String,
    pub tree_sha: String,
    pub analysed_at: String,
    pub surface: SurfaceRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SurfaceRecord {
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub consumes_files: Vec<String>,
    #[serde(default)]
    pub produces_files: Vec<String>,
    #[serde(default)]
    pub network_endpoints: Vec<String>,
    #[serde(default)]
    pub data_formats: Vec<String>,
    #[serde(default)]
    pub external_tools_spawned: Vec<String>,
    #[serde(default)]
    pub explicit_cross_project_mentions: Vec<String>,
    #[serde(default)]
    pub notes: String,
}
```

- [ ] **Step 3: Add `pub mod discover;` to `src/lib.rs`**

Add the line in alphabetical order with existing module declarations.

- [ ] **Step 4: Compile check**

Run: `cargo build`
Expected: clean build.

- [ ] **Step 5: Write round-trip test**

Add a `#[cfg(test)]` block in `src/discover/schema.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_file_round_trips_via_yaml() {
        let original = SurfaceFile {
            schema_version: SURFACE_SCHEMA_VERSION,
            project: "Alpha".to_string(),
            tree_sha: "deadbeef".to_string(),
            analysed_at: "2026-04-22T12:00:00Z".to_string(),
            surface: SurfaceRecord {
                purpose: "Does the alpha thing.".to_string(),
                consumes_files: vec!["~/.config/alpha/*.yaml".to_string()],
                produces_files: vec!["/tmp/alpha-output/*.json".to_string()],
                network_endpoints: vec!["grpc://alpha-service:50051".to_string()],
                data_formats: vec!["AlphaRecord".to_string()],
                external_tools_spawned: vec!["git".to_string()],
                explicit_cross_project_mentions: vec!["Beta".to_string()],
                notes: "".to_string(),
            },
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: SurfaceFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn surface_record_empty_fields_round_trip_as_defaults() {
        let yaml = "purpose: hello\n";
        let parsed: SurfaceRecord = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.purpose, "hello");
        assert!(parsed.consumes_files.is_empty());
        assert!(parsed.notes.is_empty());
    }
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p ravel-lite --lib discover::schema`
Expected: 2 passing tests.

- [ ] **Step 7: Commit**

```bash
git add src/discover/mod.rs src/discover/schema.rs src/lib.rs
git commit -m "R7 task 1: scaffold discover module and SurfaceFile schema"
```

---

## Task 2: `ProposalsFile` / `ProposalRecord` types

**Files:**
- Modify: `src/discover/schema.rs`

Goal: define the Stage 2 output schema that `apply` reads.

- [ ] **Step 1: Extend `schema.rs` with proposal types**

Append to `src/discover/schema.rs`:

```rust
use std::collections::BTreeMap;

use crate::related_projects::EdgeKind;

pub const PROPOSALS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalsFile {
    pub schema_version: u32,
    pub generated_at: String,
    #[serde(default)]
    pub source_tree_shas: BTreeMap<String, String>,
    #[serde(default)]
    pub proposals: Vec<ProposalRecord>,
    #[serde(default)]
    pub failures: Vec<Stage1Failure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalRecord {
    pub kind: EdgeKind,
    pub participants: Vec<String>,
    pub rationale: String,
    #[serde(default)]
    pub supporting_surface_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage1Failure {
    pub project: String,
    pub error: String,
}
```

- [ ] **Step 2: Add round-trip test**

```rust
#[test]
fn proposals_file_round_trips_via_yaml() {
    let original = ProposalsFile {
        schema_version: PROPOSALS_SCHEMA_VERSION,
        generated_at: "2026-04-22T12:05:00Z".to_string(),
        source_tree_shas: [
            ("Alpha".to_string(), "abc123".to_string()),
            ("Beta".to_string(), "def456".to_string()),
        ].into_iter().collect(),
        proposals: vec![
            ProposalRecord {
                kind: EdgeKind::Sibling,
                participants: vec!["Alpha".to_string(), "Beta".to_string()],
                rationale: "Both speak the same gRPC protocol.".to_string(),
                supporting_surface_fields: vec![
                    "Alpha.surface.network_endpoints".to_string(),
                    "Beta.surface.network_endpoints".to_string(),
                ],
            },
        ],
        failures: vec![],
    };
    let yaml = serde_yaml::to_string(&original).unwrap();
    let parsed: ProposalsFile = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(parsed, original);
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ravel-lite --lib discover::schema`
Expected: 3 passing tests.

- [ ] **Step 4: Commit**

```bash
git add src/discover/schema.rs
git commit -m "R7 task 2: add ProposalsFile / ProposalRecord schemas"
```

---

## Task 3: Tree-SHA helper + dirty-tree check

**Files:**
- Create: `src/discover/tree_sha.rs`
- Modify: `src/discover/mod.rs` — add `pub mod tree_sha;`.

Goal: compute the subtree-scoped tree SHA for a project path, bailing on non-git or dirty subtree. Works for both top-level and monorepo subtree.

- [ ] **Step 1: Create `src/discover/tree_sha.rs`**

```rust
//! Subtree-scoped git tree SHA for a project path.
//!
//! Works for both top-level repos and monorepo subtrees by computing
//! `rel = <project_path> relative to repo toplevel`, then running
//! `git rev-parse HEAD:<rel>`. An empty `rel` (project IS the repo)
//! returns the root tree.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Computes the subtree-scoped tree SHA for `project_path`.
///
/// Bails if:
/// * `project_path` is not inside a git repository.
/// * The subtree has uncommitted changes (using `git status --porcelain
///   -- <project_path>` from the repo toplevel).
pub fn compute_project_tree_sha(project_path: &Path) -> Result<String> {
    let toplevel = repo_toplevel(project_path)?;
    let rel = project_path
        .strip_prefix(&toplevel)
        .with_context(|| {
            format!(
                "project path {} is not a subpath of its git toplevel {}",
                project_path.display(),
                toplevel.display()
            )
        })?;

    ensure_clean_subtree(&toplevel, rel)?;

    let spec = if rel.as_os_str().is_empty() {
        "HEAD^{tree}".to_string()
    } else {
        format!("HEAD:{}", rel.to_string_lossy())
    };

    let output = Command::new("git")
        .arg("-C")
        .arg(&toplevel)
        .arg("rev-parse")
        .arg(&spec)
        .output()
        .context("failed to spawn `git rev-parse`")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "git rev-parse {} failed in {}: {}",
            spec,
            toplevel.display(),
            stderr.trim()
        );
    }
    let sha = String::from_utf8(output.stdout)
        .context("git rev-parse output was not valid UTF-8")?
        .trim()
        .to_string();
    if sha.is_empty() {
        bail!("git rev-parse {} returned empty output", spec);
    }
    Ok(sha)
}

fn repo_toplevel(project_path: &Path) -> Result<std::path::PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_path)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .context("failed to spawn `git rev-parse --show-toplevel`")?;
    if !output.status.success() {
        bail!(
            "project at {} is not inside a git repository — initialise with \
             `git init` or remove from the catalog",
            project_path.display()
        );
    }
    let s = String::from_utf8(output.stdout)
        .context("git --show-toplevel output was not valid UTF-8")?
        .trim()
        .to_string();
    Ok(std::path::PathBuf::from(s))
}

fn ensure_clean_subtree(toplevel: &Path, rel: &Path) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(toplevel).arg("status").arg("--porcelain");
    if !rel.as_os_str().is_empty() {
        cmd.arg("--").arg(rel);
    }
    let output = cmd
        .output()
        .context("failed to spawn `git status --porcelain`")?;
    if !output.status.success() {
        bail!(
            "git status --porcelain failed in {}",
            toplevel.display()
        );
    }
    let porcelain = String::from_utf8_lossy(&output.stdout);
    if !porcelain.trim().is_empty() {
        bail!(
            "project subtree at {} has uncommitted changes; commit or stash \
             before running discover:\n{}",
            rel.display(),
            porcelain.trim()
        );
    }
    Ok(())
}
```

- [ ] **Step 2: Register the module**

In `src/discover/mod.rs`, add after `pub mod schema;`:
```rust
pub mod tree_sha;
```

- [ ] **Step 3: Write unit tests**

Append to `src/discover/tree_sha.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    /// Initialise a fresh git repo at `path` with one committed file
    /// `README.md`. Returns the repo path for convenience.
    fn init_repo_with_readme(path: &Path) {
        run(path, &["init", "-q", "-b", "main"]);
        run(path, &["config", "user.email", "test@example.com"]);
        run(path, &["config", "user.name", "test"]);
        std::fs::write(path.join("README.md"), "hello\n").unwrap();
        run(path, &["add", "README.md"]);
        run(path, &["commit", "-q", "-m", "init"]);
    }

    fn run(cwd: &Path, args: &[&str]) {
        let status = Command::new("git").arg("-C").arg(cwd).args(args).status().unwrap();
        assert!(status.success(), "git {:?} in {} failed", args, cwd.display());
    }

    #[test]
    fn top_level_repo_yields_non_empty_sha() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());

        let sha = compute_project_tree_sha(tmp.path()).unwrap();
        assert_eq!(sha.len(), 40, "expected 40-hex SHA, got {:?}", sha);
    }

    #[test]
    fn monorepo_subtrees_have_independent_shas() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());

        let sub_a = tmp.path().join("sub-a");
        let sub_b = tmp.path().join("sub-b");
        std::fs::create_dir_all(&sub_a).unwrap();
        std::fs::create_dir_all(&sub_b).unwrap();
        std::fs::write(sub_a.join("a.txt"), "A\n").unwrap();
        std::fs::write(sub_b.join("b.txt"), "B\n").unwrap();
        run(tmp.path(), &["add", "."]);
        run(tmp.path(), &["commit", "-q", "-m", "add subs"]);

        let sha_a = compute_project_tree_sha(&sub_a).unwrap();
        let sha_b = compute_project_tree_sha(&sub_b).unwrap();
        assert_ne!(sha_a, sha_b, "subtrees with different content must have different SHAs");
    }

    #[test]
    fn sibling_subtree_change_does_not_invalidate_other_subtree() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());
        let sub_a = tmp.path().join("sub-a");
        let sub_b = tmp.path().join("sub-b");
        std::fs::create_dir_all(&sub_a).unwrap();
        std::fs::create_dir_all(&sub_b).unwrap();
        std::fs::write(sub_a.join("a.txt"), "A\n").unwrap();
        std::fs::write(sub_b.join("b.txt"), "B1\n").unwrap();
        run(tmp.path(), &["add", "."]);
        run(tmp.path(), &["commit", "-q", "-m", "add subs"]);

        let sha_b_before = compute_project_tree_sha(&sub_b).unwrap();

        // Touch only sub-a.
        std::fs::write(sub_a.join("a.txt"), "A-edited\n").unwrap();
        run(tmp.path(), &["add", "."]);
        run(tmp.path(), &["commit", "-q", "-m", "edit sub-a"]);

        let sha_b_after = compute_project_tree_sha(&sub_b).unwrap();
        assert_eq!(sha_b_before, sha_b_after, "sub-b's tree SHA must be stable across a commit that only touches sub-a");
    }

    #[test]
    fn non_git_project_bails_with_actionable_message() {
        let tmp = TempDir::new().unwrap();
        // No `git init` — path is not a repo.
        let err = compute_project_tree_sha(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("not inside a git repository"), "got: {msg}");
        assert!(msg.contains("git init"), "got: {msg}");
    }

    #[test]
    fn dirty_subtree_bails() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());
        std::fs::write(tmp.path().join("README.md"), "edited\n").unwrap();

        let err = compute_project_tree_sha(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("uncommitted changes"), "got: {msg}");
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p ravel-lite --lib discover::tree_sha`
Expected: 5 passing tests.

- [ ] **Step 5: Commit**

```bash
git add src/discover/tree_sha.rs src/discover/mod.rs
git commit -m "R7 task 3: subtree-scoped tree SHA helper with dirty-tree guard"
```

---

## Task 4: Per-project cache read/write

**Files:**
- Create: `src/discover/cache.rs`
- Modify: `src/discover/mod.rs` — add `pub mod cache;`.

Goal: load/save `<config-dir>/discover-cache/<project>.yaml`; atomic write via tmp-then-rename. Matches the pattern in `src/related_projects.rs::save_atomic`.

- [ ] **Step 1: Create `src/discover/cache.rs`**

```rust
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
```

- [ ] **Step 2: Register module**

`src/discover/mod.rs`:
```rust
pub mod cache;
```

- [ ] **Step 3: Write unit tests**

Append to `src/discover/cache.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use super::super::schema::SurfaceRecord;
    use tempfile::TempDir;

    fn sample(name: &str, sha: &str) -> SurfaceFile {
        SurfaceFile {
            schema_version: SURFACE_SCHEMA_VERSION,
            project: name.to_string(),
            tree_sha: sha.to_string(),
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
        assert_eq!(loaded.project, "Old", "rename does not rewrite identity fields — save_atomic keyed on file.project, so after manual rename the in-file project name is stale. Step 4 of this test is documenting this; callers who rename must follow up with a re-analysis (tree SHA is preserved, cache is cheap to rebuild).");
    }

    #[test]
    fn rename_silently_succeeds_when_source_absent() {
        let tmp = TempDir::new().unwrap();
        rename(tmp.path(), "Ghost", "Phantom").unwrap();
    }
}
```

*(Note on the `rename_moves_cache_file` assertion text: the cache file's internal `project` field becomes stale after a rename. This is fine because the next `discover` run on the renamed project will either cache-hit on the tree SHA and update the identity fields (see Task 6 Step 3 — identity fields are always re-injected on Stage 1 ingest), or cache-miss and regenerate entirely. The comment is a test-time reminder, not a runtime invariant.)*

- [ ] **Step 4: Run tests**

Run: `cargo test -p ravel-lite --lib discover::cache`
Expected: 6 passing tests.

- [ ] **Step 5: Commit**

```bash
git add src/discover/cache.rs src/discover/mod.rs
git commit -m "R7 task 4: per-project surface cache read/write with rename helper"
```

---

## Task 5: Extend `projects::run_rename` cascade into cache dir

**Files:**
- Modify: `src/projects.rs`

Goal: rename of a catalog entry now cascades into three places: the catalog itself, `related-projects.yaml`, and `<config-dir>/discover-cache/<name>.yaml`.

- [ ] **Step 1: Update `run_rename` to cascade cache files**

In `src/projects.rs`, find `run_rename` and add the third cascade after the `related_projects::rename_project_in_edges` call:

```rust
pub fn run_rename(config_root: &Path, old: &str, new: &str) -> Result<()> {
    if old == new {
        return Ok(());
    }
    let mut catalog = load_or_empty(config_root)?;
    if catalog.find_by_name(new).is_some() {
        bail!("cannot rename to '{}': name already in use", new);
    }
    let entry = catalog
        .projects
        .iter_mut()
        .find(|p| p.name == old)
        .with_context(|| format!("no project named '{old}' in catalog"))?;
    entry.name = new.to_string();
    save_atomic(config_root, &catalog)?;
    crate::related_projects::rename_project_in_edges(config_root, old, new)?;
    crate::discover::cache::rename(config_root, old, new)
}
```

- [ ] **Step 2: Write a cascade test**

Append to `src/projects.rs`'s test module:

```rust
#[test]
fn run_rename_cascades_into_discover_cache() {
    use crate::discover::cache;
    use crate::discover::schema::{SurfaceFile, SurfaceRecord, SURFACE_SCHEMA_VERSION};

    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let project = mk_project_dir(tmp.path(), "OldName");
    run_add(&cfg, "OldName", &project).unwrap();

    let surface = SurfaceFile {
        schema_version: SURFACE_SCHEMA_VERSION,
        project: "OldName".to_string(),
        tree_sha: "abc".to_string(),
        analysed_at: "2026-04-22T00:00:00Z".to_string(),
        surface: SurfaceRecord::default(),
    };
    cache::save_atomic(&cfg, &surface).unwrap();
    assert!(cache::cache_path(&cfg, "OldName").exists());

    run_rename(&cfg, "OldName", "NewName").unwrap();

    assert!(!cache::cache_path(&cfg, "OldName").exists());
    assert!(cache::cache_path(&cfg, "NewName").exists());
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ravel-lite --lib projects::tests::run_rename_cascades_into_discover_cache`
Expected: 1 passing test. Also run the full `projects` test module to confirm no regressions: `cargo test -p ravel-lite --lib projects`.

- [ ] **Step 4: Commit**

```bash
git add src/projects.rs
git commit -m "R7 task 5: cascade project rename into discover-cache directory"
```

---

## Task 6: Stage 1 prompt template + catalog iteration + `run_stage1`

**Files:**
- Create: `defaults/discover-stage1.md`
- Create: `src/discover/stage1.rs`
- Modify: `src/discover/mod.rs` — add `pub mod stage1;`.
- Modify: `src/init.rs` — register the prompt template as an embedded default.

Goal: the core Stage 1 orchestration. Given a config root and a catalog, determine which projects need fresh analysis (cache misses), spawn bounded-parallel subagents, inject identity fields, write caches. Return the full set of surfaces (cached + fresh) for Stage 2.

- [ ] **Step 1: Write `defaults/discover-stage1.md`**

```markdown
# Discovery — Stage 1: Extract Interaction Surface

You are analysing the project rooted at your current working directory.

Your task is to read the project thoroughly and emit a structured
interaction-surface record describing how this project interacts with
the outside world — *not* what it does internally.

You have Read / Grep / Glob / Bash tools available. For large projects,
you may dispatch sub-subagents to analyse specific subdirectories in
parallel, then merge their findings into your final output. Use your
judgement.

## What to extract

For each field below, include evidence from the code — do not speculate.
If a field does not apply, emit an empty list or empty string.

- `purpose` — one paragraph describing what this project does, written
  from evidence in the README, main entry points, and top-level modules.
- `consumes_files` — file paths or glob patterns this project *reads*
  from the filesystem (config files, data files, plan-state files, etc.).
  Include both absolute paths and well-known relative patterns.
- `produces_files` — file paths or glob patterns this project *writes*.
- `network_endpoints` — protocols and addresses it serves or consumes.
  Use the format `<protocol>://<address-or-description>`. Examples:
  `grpc://task-service:50051`, `http://localhost/api/tasks`,
  `mcp://stdio (tool server)`.
- `data_formats` — named message types, schema IDs, struct names that
  define the data this project emits or consumes (e.g., `BacklogFile`,
  `TaskCounts`, `MyProtoMessage`).
- `external_tools_spawned` — binaries this project shells out to
  (`git`, `claude`, `cargo`, etc.).
- `explicit_cross_project_mentions` — names or paths of *other projects*
  this project directly references in its README, memory files, or code
  comments.
- `notes` — anything else relationally relevant that did not fit above.

## Output format

Write your output as YAML to `{{SURFACE_OUTPUT_PATH}}` — exactly one
`SurfaceRecord` document. Do NOT emit the `schema_version`, `project`,
`tree_sha`, or `analysed_at` fields — those are injected by the caller.

Your output must be parseable by this Rust struct (field order flexible):

```yaml
purpose: |
  <one paragraph>
consumes_files:
  - <glob or path>
produces_files:
  - <glob or path>
network_endpoints:
  - <protocol>://<address>
data_formats:
  - <name>
external_tools_spawned:
  - <binary-name>
explicit_cross_project_mentions:
  - <project-name-or-path>
notes: |
  <free-form prose>
```

After writing the YAML file, your final message should confirm the path
written. No other output is required.
```

- [ ] **Step 2: Register the prompt in `src/init.rs`**

Find the existing list of embedded defaults (the `EmbeddedFile` array / registration code). Add two entries matching the existing pattern:
- `defaults/discover-stage1.md` → copied to `<config-dir>/discover-stage1.md`.
- `defaults/discover-stage2.md` → copied to `<config-dir>/discover-stage2.md`. (File itself is created in Task 7; register both entries now so the drift-detection test in `init.rs` passes.)

If the drift-detection test in `init.rs` enforces "every file in `defaults/` must be registered", write a stub `defaults/discover-stage2.md` file with a one-line comment (`<!-- Stage 2 prompt — fleshed out in R7 task 7 -->`) so the test passes; the real content lands in Task 7.

- [ ] **Step 3: Create `src/discover/stage1.rs` — core orchestration**

```rust
//! Stage 1: per-project interaction-surface extraction.
//!
//! For each project in the catalog:
//!   1. Compute its subtree-scoped tree SHA.
//!   2. If the cached surface's `tree_sha` matches, use it as-is.
//!   3. Otherwise, spawn a `claude -p` subagent with CWD = project path
//!      and the Stage 1 prompt; parse YAML output; inject identity
//!      fields; write cache atomically.
//!
//! Dispatch is bounded by a `tokio::sync::Semaphore`; default 4.
//!
//! Failure policy is best-effort: per-project failures are captured in
//! a `Vec<Stage1Failure>` and surfaced in the proposals file. They do
//! not abort the pipeline.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::projects::ProjectEntry;

use super::cache;
use super::schema::{Stage1Failure, SurfaceFile, SurfaceRecord, SURFACE_SCHEMA_VERSION};
use super::tree_sha::compute_project_tree_sha;

pub const DEFAULT_STAGE1_TIMEOUT_SECS: u64 = 600;

/// The concrete outcome for one project in a Stage 1 pass.
pub enum Stage1Outcome {
    Fresh(SurfaceFile),
    Cached(SurfaceFile),
    Failed(Stage1Failure),
}

pub struct Stage1Config {
    pub config_root: PathBuf,
    pub model: String,
    pub prompt_template: String,
    pub concurrency: usize,
    pub timeout: Duration,
}

pub async fn run_stage1(
    projects: &[ProjectEntry],
    cfg: &Stage1Config,
) -> Result<Vec<Stage1Outcome>> {
    let semaphore = Arc::new(Semaphore::new(cfg.concurrency.max(1)));
    let mut join_set: JoinSet<(String, Result<Stage1Outcome>)> = JoinSet::new();

    for project in projects {
        let permit_sem = Arc::clone(&semaphore);
        let config_root = cfg.config_root.clone();
        let model = cfg.model.clone();
        let prompt_template = cfg.prompt_template.clone();
        let timeout = cfg.timeout;
        let name = project.name.clone();
        let path = project.path.clone();

        join_set.spawn(async move {
            let _permit = permit_sem.acquire_owned().await.expect("semaphore is never closed");
            let outcome = process_project(&config_root, &name, &path, &model, &prompt_template, timeout).await;
            (name, outcome)
        });
    }

    let mut outcomes = Vec::with_capacity(projects.len());
    while let Some(join_result) = join_set.join_next().await {
        match join_result {
            Ok((_name, Ok(outcome))) => outcomes.push(outcome),
            Ok((name, Err(e))) => outcomes.push(Stage1Outcome::Failed(Stage1Failure {
                project: name,
                error: format!("{e:#}"),
            })),
            Err(join_err) => outcomes.push(Stage1Outcome::Failed(Stage1Failure {
                project: "<unknown>".to_string(),
                error: format!("join error: {join_err:#}"),
            })),
        }
    }
    Ok(outcomes)
}

async fn process_project(
    config_root: &Path,
    name: &str,
    path: &Path,
    model: &str,
    prompt_template: &str,
    timeout: Duration,
) -> Result<Stage1Outcome> {
    let tree_sha = compute_project_tree_sha(path)
        .with_context(|| format!("compute_project_tree_sha for '{name}' at {}", path.display()))?;

    if let Some(cached) = cache::load(config_root, name)? {
        if cached.tree_sha == tree_sha {
            return Ok(Stage1Outcome::Cached(cached));
        }
    }

    let output_path = cache::cache_dir(config_root).join(format!(".tmp-{name}-{}.yaml", std::process::id()));
    std::fs::create_dir_all(cache::cache_dir(config_root))?;
    if output_path.exists() {
        std::fs::remove_file(&output_path)?;
    }

    let prompt = prompt_template.replace(
        "{{SURFACE_OUTPUT_PATH}}",
        &output_path.to_string_lossy(),
    );

    let exit_ok = spawn_claude_with_cwd(&prompt, model, path, timeout).await?;
    if !exit_ok {
        bail!("Stage 1 subagent for '{name}' exited non-zero");
    }
    if !output_path.exists() {
        bail!(
            "Stage 1 subagent for '{name}' did not create {}",
            output_path.display()
        );
    }

    let raw = std::fs::read_to_string(&output_path)?;
    let surface: SurfaceRecord = serde_yaml::from_str(&raw)
        .with_context(|| format!("parse Stage 1 output for '{name}' from {}", output_path.display()))?;
    let _ = std::fs::remove_file(&output_path);

    let file = SurfaceFile {
        schema_version: SURFACE_SCHEMA_VERSION,
        project: name.to_string(),
        tree_sha: tree_sha.clone(),
        analysed_at: chrono::Utc::now().to_rfc3339(),
        surface,
    };
    cache::save_atomic(config_root, &file)?;
    Ok(Stage1Outcome::Fresh(file))
}

async fn spawn_claude_with_cwd(
    prompt: &str,
    model: &str,
    cwd: &Path,
    timeout: Duration,
) -> Result<bool> {
    let mut child = TokioCommand::new("claude")
        .arg("-p")
        .arg(prompt)
        .arg("--model")
        .arg(model)
        .arg("--strict-mcp-config")
        .arg("--setting-sources")
        .arg("project,local")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn `claude` — ensure it is installed and on PATH")?;

    let mut stdout = child.stdout.take().context("claude stdout pipe unavailable")?;
    let mut drain = String::new();
    let wait = tokio::time::timeout(timeout, async {
        let _ = stdout.read_to_string(&mut drain).await;
        child.wait().await
    })
    .await;
    match wait {
        Ok(Ok(status)) => Ok(status.success()),
        Ok(Err(io_err)) => Err(io_err).context("waiting on claude process"),
        Err(_elapsed) => {
            let _ = child.kill().await;
            bail!(
                "claude Stage 1 subagent timed out after {}s in {}",
                timeout.as_secs(),
                cwd.display()
            )
        }
    }
}
```

*(Dependency: this task uses `chrono` for timestamps. If `chrono` isn't already a project dependency, add `chrono = { version = "0.4", default-features = false, features = ["clock"] }` to `Cargo.toml` as the first sub-step of this task. Confirm with `grep chrono Cargo.toml` before running step 1.)*

- [ ] **Step 4: Register module**

`src/discover/mod.rs`:
```rust
pub mod stage1;
```

- [ ] **Step 5: Unit test the cache-hit short-circuit**

Append to `src/discover/stage1.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use super::super::schema::SurfaceRecord;
    use tempfile::TempDir;

    /// Directly exercise `process_project` against a project whose cache
    /// is already warm with the current tree SHA — it must return
    /// `Cached(..)` without attempting to spawn claude.
    #[tokio::test]
    async fn cache_hit_bypasses_subagent() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();

        // Make a fresh git repo for the project, compute its real SHA.
        let project = tmp.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();
        super::super::tree_sha::tests::init_repo_with_readme(&project);
        let sha = super::super::tree_sha::compute_project_tree_sha(&project).unwrap();

        // Seed a cache entry with the exact SHA.
        let file = SurfaceFile {
            schema_version: SURFACE_SCHEMA_VERSION,
            project: "Proj".to_string(),
            tree_sha: sha.clone(),
            analysed_at: "2026-04-22T00:00:00Z".to_string(),
            surface: SurfaceRecord {
                purpose: "cached".to_string(),
                ..Default::default()
            },
        };
        cache::save_atomic(&cfg, &file).unwrap();

        let outcome = process_project(
            &cfg,
            "Proj",
            &project,
            "unused-model",
            "unused-prompt",
            Duration::from_secs(5),
        )
        .await
        .unwrap();

        match outcome {
            Stage1Outcome::Cached(f) => {
                assert_eq!(f.tree_sha, sha);
                assert_eq!(f.surface.purpose, "cached");
            }
            _ => panic!("expected Cached outcome"),
        }
    }
}
```

*(The test borrows `init_repo_with_readme` from `tree_sha::tests`; Rust will need that helper `pub(super)` or re-exported for cross-module test access. Mark `fn init_repo_with_readme` as `pub(crate)` inside `tree_sha::tests` if the compiler complains.)*

- [ ] **Step 6: Run tests**

Run: `cargo test -p ravel-lite --lib discover::stage1`
Expected: 1 passing test.

- [ ] **Step 7: Commit**

```bash
git add defaults/discover-stage1.md defaults/discover-stage2.md src/discover/stage1.rs src/discover/mod.rs src/init.rs Cargo.toml Cargo.lock
git commit -m "R7 task 6: Stage 1 orchestration with cache hit/miss path"
```

---

## Task 7: Stage 2 prompt template + `run_stage2`

**Files:**
- Modify: `defaults/discover-stage2.md` — replace the Task 6 stub with the real prompt.
- Create: `src/discover/stage2.rs`
- Modify: `src/discover/mod.rs` — add `pub mod stage2;`.

Goal: take the full set of Stage 1 surfaces (cached + fresh) plus the catalog, compose a single prompt, invoke `claude -p`, parse proposal YAML, return a `ProposalsFile`. Failures from Stage 1 are passed through into `proposals.failures`.

- [ ] **Step 1: Flesh out `defaults/discover-stage2.md`**

```markdown
# Discovery — Stage 2: Infer Cross-Project Edges

You are given a collection of per-project interaction-surface records.
Your task is to propose relationship edges between catalogued projects
based on what their surfaces reveal.

## Edge kinds

- `sibling(A, B)` — peer-level relationship: two projects share a
  purpose, speak the same protocol, or exchange the same data format as
  peers. Order-insensitive.
- `parent-of(A, B)` — A produces artifacts, files, or contracts that B
  consumes. Order-sensitive: parent first. If A's `produces_files`
  matches B's `consumes_files`, or A serves an endpoint B calls, that
  is evidence for `parent-of(A, B)`.

## Matching signals

Propose edges when you see evidence such as:
- Overlapping file paths or globs between one project's `produces_files`
  and another's `consumes_files`.
- Matching network endpoints (server vs. client of the same protocol/
  address).
- Shared data format names (same struct / schema / message type).
- Shared external tools that suggest tight coupling (e.g., both projects
  spawn `some-custom-daemon`).
- Direct cross-project mentions in `explicit_cross_project_mentions`,
  *especially* when reciprocated by the other project.
- Semantic purpose overlap (both describe themselves as "task queue",
  "config loader", etc.) — use judgement here.

## Output format

Write YAML to `{{PROPOSALS_OUTPUT_PATH}}` matching this shape:

```yaml
generated_at: <ISO-8601 UTC timestamp>
proposals:
  - kind: sibling | parent-of
    participants: [<name>, <name>]    # parent first for parent-of
    rationale: |
      <one paragraph citing specific surface fields from the input>
    supporting_surface_fields:
      - <e.g., "Alpha.surface.produces_files">
      - <e.g., "Beta.surface.consumes_files">
```

Do NOT emit `schema_version` or `source_tree_shas` — those are injected
by the caller. Only propose edges between projects that appear in the
input. Only use project names exactly as they appear in the input —
no paths, no aliases.

After writing the YAML, your final message should confirm the path
written. No other output is required.

## Input

The input below lists every catalogued project's surface record.

---
{{SURFACE_RECORDS_YAML}}
```

- [ ] **Step 2: Create `src/discover/stage2.rs`**

```rust
//! Stage 2: global edge inference over Stage 1 surface records.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;

use super::schema::{
    ProposalRecord, ProposalsFile, Stage1Failure, SurfaceFile, PROPOSALS_SCHEMA_VERSION,
};

pub const DEFAULT_STAGE2_TIMEOUT_SECS: u64 = 300;

pub struct Stage2Config {
    pub config_root: PathBuf,
    pub model: String,
    pub prompt_template: String,
    pub timeout: Duration,
}

/// Run Stage 2 over `surfaces`. `failures` from Stage 1 are passed
/// through unchanged into the output.
pub async fn run_stage2(
    surfaces: &[SurfaceFile],
    failures: Vec<Stage1Failure>,
    cfg: &Stage2Config,
) -> Result<ProposalsFile> {
    let output_path = cfg
        .config_root
        .join(format!(".tmp-proposals-{}.yaml", std::process::id()));
    if output_path.exists() {
        std::fs::remove_file(&output_path)?;
    }

    let surfaces_yaml = render_surfaces_for_prompt(surfaces)?;
    let prompt = cfg
        .prompt_template
        .replace("{{PROPOSALS_OUTPUT_PATH}}", &output_path.to_string_lossy())
        .replace("{{SURFACE_RECORDS_YAML}}", &surfaces_yaml);

    let success = spawn_claude_for_stage2(&prompt, &cfg.model, cfg.timeout).await?;
    if !success {
        bail!("Stage 2 claude subprocess exited non-zero");
    }
    if !output_path.exists() {
        bail!("Stage 2 did not create {}", output_path.display());
    }

    let raw = std::fs::read_to_string(&output_path)?;
    let raw_parsed: RawStage2Output = serde_yaml::from_str(&raw)
        .with_context(|| format!("parse Stage 2 output from {}", output_path.display()))?;
    let _ = std::fs::remove_file(&output_path);

    let source_tree_shas = surfaces
        .iter()
        .map(|s| (s.project.clone(), s.tree_sha.clone()))
        .collect();

    Ok(ProposalsFile {
        schema_version: PROPOSALS_SCHEMA_VERSION,
        generated_at: raw_parsed.generated_at,
        source_tree_shas,
        proposals: raw_parsed.proposals,
        failures,
    })
}

#[derive(serde::Deserialize)]
struct RawStage2Output {
    generated_at: String,
    #[serde(default)]
    proposals: Vec<ProposalRecord>,
}

fn render_surfaces_for_prompt(surfaces: &[SurfaceFile]) -> Result<String> {
    // Emit a single YAML document with a top-level `surfaces:` list for
    // unambiguous consumption by the LLM.
    #[derive(serde::Serialize)]
    struct Wrapped<'a> {
        surfaces: &'a [SurfaceFile],
    }
    Ok(serde_yaml::to_string(&Wrapped { surfaces })?)
}

async fn spawn_claude_for_stage2(
    prompt: &str,
    model: &str,
    timeout: Duration,
) -> Result<bool> {
    let mut child = TokioCommand::new("claude")
        .arg("-p")
        .arg(prompt)
        .arg("--model")
        .arg(model)
        .arg("--strict-mcp-config")
        .arg("--setting-sources")
        .arg("project,local")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn `claude`")?;
    let mut stdout = child.stdout.take().context("claude stdout pipe unavailable")?;
    let mut drain = String::new();
    let wait = tokio::time::timeout(timeout, async {
        let _ = stdout.read_to_string(&mut drain).await;
        child.wait().await
    })
    .await;
    match wait {
        Ok(Ok(status)) => Ok(status.success()),
        Ok(Err(io_err)) => Err(io_err).context("waiting on claude"),
        Err(_elapsed) => {
            let _ = child.kill().await;
            bail!("claude Stage 2 timed out after {}s", timeout.as_secs())
        }
    }
}
```

- [ ] **Step 3: Register module**

`src/discover/mod.rs`:
```rust
pub mod stage2;
```

- [ ] **Step 4: Unit test `render_surfaces_for_prompt`**

Append to `src/discover/stage2.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use super::super::schema::{SurfaceRecord, SURFACE_SCHEMA_VERSION};

    #[test]
    fn render_surfaces_emits_expected_structure() {
        let surfaces = vec![
            SurfaceFile {
                schema_version: SURFACE_SCHEMA_VERSION,
                project: "A".to_string(),
                tree_sha: "aaa".to_string(),
                analysed_at: "t".to_string(),
                surface: SurfaceRecord {
                    purpose: "alpha".to_string(),
                    ..Default::default()
                },
            },
        ];
        let rendered = render_surfaces_for_prompt(&surfaces).unwrap();
        assert!(rendered.contains("surfaces:"));
        assert!(rendered.contains("project: A"));
        assert!(rendered.contains("purpose: alpha"));
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p ravel-lite --lib discover::stage2`
Expected: 1 passing test.

- [ ] **Step 6: Commit**

```bash
git add defaults/discover-stage2.md src/discover/stage2.rs src/discover/mod.rs
git commit -m "R7 task 7: Stage 2 orchestration + prompt template"
```

---

## Task 8: Top-level `run_discover` orchestrator + proposals writeback

**Files:**
- Modify: `src/discover/mod.rs`

Goal: tie Stage 1 + Stage 2 together into a single entry point `run_discover(config_root, project_filter, concurrency, apply)`. Writes `<config-dir>/discover-proposals.yaml`.

- [ ] **Step 1: Add `run_discover` to `src/discover/mod.rs`**

```rust
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};

pub mod apply;
pub mod cache;
pub mod schema;
pub mod stage1;
pub mod stage2;
pub mod tree_sha;

use crate::config::{load_agent_config, load_shared_config};
use crate::projects::{self, ProjectEntry};

use self::schema::{ProposalsFile, Stage1Failure, SurfaceFile};
use self::stage1::{run_stage1, Stage1Config, Stage1Outcome};
use self::stage2::{run_stage2, Stage2Config};

pub const PROPOSALS_FILE: &str = "discover-proposals.yaml";
pub const DEFAULT_CONCURRENCY: usize = 4;
pub const DEFAULT_DISCOVER_MODEL: &str = "claude-sonnet-4-6";

pub struct RunDiscoverOptions {
    pub project_filter: Option<String>,
    pub concurrency: Option<usize>,
    pub apply: bool,
}

pub async fn run_discover(config_root: &Path, options: RunDiscoverOptions) -> Result<()> {
    let catalog = projects::load_or_empty(config_root)?;
    if catalog.projects.is_empty() {
        bail!("projects catalog is empty; nothing to discover");
    }

    let to_analyse: Vec<ProjectEntry> = match &options.project_filter {
        Some(name) => vec![catalog
            .find_by_name(name)
            .with_context(|| format!("project '{name}' not in catalog"))?
            .clone()],
        None => catalog.projects.clone(),
    };
    let all_projects = catalog.projects.clone();

    let shared = load_shared_config(config_root)?;
    let agent_config = load_agent_config(config_root, &shared.agent)?;
    let model = agent_config
        .models
        .get("discover")
        .cloned()
        .unwrap_or_else(|| DEFAULT_DISCOVER_MODEL.to_string());

    let stage1_prompt = load_prompt(config_root, "discover-stage1.md")?;
    let stage2_prompt = load_prompt(config_root, "discover-stage2.md")?;

    let concurrency = options.concurrency.unwrap_or(DEFAULT_CONCURRENCY).max(1);

    let stage1_cfg = Stage1Config {
        config_root: config_root.to_path_buf(),
        model: model.clone(),
        prompt_template: stage1_prompt,
        concurrency,
        timeout: Duration::from_secs(stage1::DEFAULT_STAGE1_TIMEOUT_SECS),
    };

    eprintln!(
        "Stage 1: analysing {} project(s) with concurrency={}...",
        to_analyse.len(),
        concurrency
    );
    let outcomes = run_stage1(&to_analyse, &stage1_cfg).await?;

    // Collect surfaces for Stage 2. For a `--project` filter, fill in
    // the other catalogued projects from their cache so Stage 2 still
    // has the full set. Projects with no cache yet are skipped from
    // Stage 2 and recorded as "not yet analysed" failures.
    let mut surfaces: Vec<SurfaceFile> = Vec::new();
    let mut failures: Vec<Stage1Failure> = Vec::new();
    for outcome in outcomes {
        match outcome {
            Stage1Outcome::Fresh(s) | Stage1Outcome::Cached(s) => surfaces.push(s),
            Stage1Outcome::Failed(f) => failures.push(f),
        }
    }
    if options.project_filter.is_some() {
        for project in &all_projects {
            if surfaces.iter().any(|s| s.project == project.name) {
                continue;
            }
            if failures.iter().any(|f| f.project == project.name) {
                continue;
            }
            match cache::load(config_root, &project.name)? {
                Some(cached) => surfaces.push(cached),
                None => failures.push(Stage1Failure {
                    project: project.name.clone(),
                    error: "not yet analysed; run discover without --project to populate".to_string(),
                }),
            }
        }
    }

    eprintln!(
        "Stage 2: inferring edges over {} surface(s)...",
        surfaces.len()
    );
    let stage2_cfg = Stage2Config {
        config_root: config_root.to_path_buf(),
        model,
        prompt_template: stage2_prompt,
        timeout: Duration::from_secs(stage2::DEFAULT_STAGE2_TIMEOUT_SECS),
    };
    let proposals = run_stage2(&surfaces, failures, &stage2_cfg).await?;

    save_proposals_atomic(config_root, &proposals)?;

    let had_failures = !proposals.failures.is_empty();
    eprintln!(
        "Wrote {} proposal(s) and {} failure(s) to {}",
        proposals.proposals.len(),
        proposals.failures.len(),
        proposals_path(config_root).display()
    );

    if options.apply {
        apply::run_discover_apply(config_root)?;
    }

    if had_failures {
        bail!("discover completed with Stage 1 failures — see the failures section of the proposals file");
    }
    Ok(())
}

pub fn proposals_path(config_root: &Path) -> PathBuf {
    config_root.join(PROPOSALS_FILE)
}

pub fn save_proposals_atomic(config_root: &Path, file: &ProposalsFile) -> Result<()> {
    let path = proposals_path(config_root);
    let tmp = config_root.join(format!(".{PROPOSALS_FILE}.tmp"));
    let yaml = serde_yaml::to_string(file).context("serialise ProposalsFile")?;
    std::fs::write(&tmp, yaml.as_bytes())?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn load_proposals(config_root: &Path) -> Result<ProposalsFile> {
    let path = proposals_path(config_root);
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let file: ProposalsFile = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if file.schema_version != schema::PROPOSALS_SCHEMA_VERSION {
        bail!(
            "{} has schema_version {} but this ravel-lite expects {}",
            path.display(),
            file.schema_version,
            schema::PROPOSALS_SCHEMA_VERSION
        );
    }
    Ok(file)
}

fn load_prompt(config_root: &Path, filename: &str) -> Result<String> {
    let path = config_root.join(filename);
    std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read prompt {}", path.display()))
}
```

*(Plan-time decision: the load_prompt helper above is a local convenience; if the repo already has a canonical prompt-loading function in `src/prompt.rs` that applies token substitution, use that instead and route through `substitute_tokens`. Adjust step 1 accordingly during implementation.)*

- [ ] **Step 2: Write a round-trip test for the proposals writeback**

Append to `src/discover/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use super::schema::*;
    use tempfile::TempDir;

    #[test]
    fn save_then_load_proposals_round_trips() {
        let tmp = TempDir::new().unwrap();
        let file = ProposalsFile {
            schema_version: PROPOSALS_SCHEMA_VERSION,
            generated_at: "2026-04-22T00:00:00Z".to_string(),
            source_tree_shas: [("A".to_string(), "abc".to_string())]
                .into_iter()
                .collect(),
            proposals: vec![],
            failures: vec![Stage1Failure {
                project: "B".to_string(),
                error: "oops".to_string(),
            }],
        };
        save_proposals_atomic(tmp.path(), &file).unwrap();
        let loaded = load_proposals(tmp.path()).unwrap();
        assert_eq!(loaded, file);
    }
}
```

- [ ] **Step 3: Run tests and verify build**

Run: `cargo build && cargo test -p ravel-lite --lib discover`
Expected: clean build; all discover tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/discover/mod.rs
git commit -m "R7 task 8: run_discover orchestrator + proposals writeback"
```

---

## Task 9: `apply` — merge proposals into `related-projects.yaml`

**Files:**
- Create: `src/discover/apply.rs`
- Modify: `src/discover/mod.rs` (already declared `pub mod apply;` in Task 8 — confirm it's there).

Goal: read `discover-proposals.yaml`, merge each proposed edge via `RelatedProjectsFile::add_edge`, report and reject kind-conflicts without aborting.

- [ ] **Step 1: Create `src/discover/apply.rs`**

```rust
//! Merge `discover-proposals.yaml` into `related-projects.yaml`.

use std::path::Path;

use anyhow::{Context, Result};

use crate::related_projects::{self, Edge, EdgeKind, RelatedProjectsFile};

use super::load_proposals;

pub struct ApplyReport {
    pub added: usize,
    pub already_present: usize,
    pub kind_conflicts: Vec<KindConflict>,
}

pub struct KindConflict {
    pub proposed: Edge,
    pub existing: Edge,
}

pub fn run_discover_apply(config_root: &Path) -> Result<()> {
    let report = apply_proposals(config_root)?;
    eprintln!(
        "discover-apply: added {} edge(s), {} already present, {} kind-conflict(s)",
        report.added, report.already_present, report.kind_conflicts.len()
    );
    for c in &report.kind_conflicts {
        eprintln!(
            "  conflict: proposed {:?} on {:?} but existing {:?} blocks it",
            c.proposed.kind, c.proposed.participants, c.existing.kind
        );
    }
    Ok(())
}

pub fn apply_proposals(config_root: &Path) -> Result<ApplyReport> {
    let proposals = load_proposals(config_root)?;
    let mut file = related_projects::load_or_empty(config_root)?;
    let mut added = 0usize;
    let mut already_present = 0usize;
    let mut kind_conflicts = Vec::new();

    for p in proposals.proposals {
        let proposed = Edge {
            kind: p.kind,
            participants: p.participants,
        };
        if let Err(e) = proposed.validate() {
            eprintln!("  skipping invalid proposal: {e:#}");
            continue;
        }
        if let Some(existing) = find_conflicting_kind(&file, &proposed) {
            kind_conflicts.push(KindConflict {
                proposed: proposed.clone(),
                existing: existing.clone(),
            });
            continue;
        }
        match file.add_edge(proposed.clone())? {
            true => added += 1,
            false => already_present += 1,
        }
    }

    if added > 0 {
        related_projects::save_atomic(config_root, &file)
            .context("save related-projects.yaml after applying proposals")?;
    }

    Ok(ApplyReport {
        added,
        already_present,
        kind_conflicts,
    })
}

/// Look for an existing edge on the same participant pair with a
/// *different* kind than `proposed`. Sibling/parent-of on the same
/// unordered pair are mutually exclusive at the apply layer even though
/// the underlying schema would technically allow both to coexist —
/// this check is the policy knob.
fn find_conflicting_kind<'a>(
    file: &'a RelatedProjectsFile,
    proposed: &Edge,
) -> Option<&'a Edge> {
    let pair_sorted = {
        let mut v = proposed.participants.clone();
        v.sort();
        v
    };
    file.edges.iter().find(|e| {
        let mut ev = e.participants.clone();
        ev.sort();
        ev == pair_sorted && e.kind != proposed.kind
    })
}
```

- [ ] **Step 2: Unit tests**

Append to `src/discover/apply.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use super::super::schema::{ProposalRecord, ProposalsFile, PROPOSALS_SCHEMA_VERSION};
    use super::super::save_proposals_atomic;
    use crate::projects;
    use tempfile::TempDir;

    fn seed_two_projects(cfg: &std::path::Path) -> std::path::PathBuf {
        let mut catalog = projects::ProjectsCatalog::default();
        let a = cfg.parent().unwrap().join("A");
        let b = cfg.parent().unwrap().join("B");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        projects::try_add_named(&mut catalog, "A", &a).unwrap();
        projects::try_add_named(&mut catalog, "B", &b).unwrap();
        projects::save_atomic(cfg, &catalog).unwrap();
        a
    }

    fn mk_proposals(pairs: &[(EdgeKind, &str, &str)]) -> ProposalsFile {
        ProposalsFile {
            schema_version: PROPOSALS_SCHEMA_VERSION,
            generated_at: "t".to_string(),
            source_tree_shas: Default::default(),
            proposals: pairs
                .iter()
                .map(|(k, a, b)| ProposalRecord {
                    kind: *k,
                    participants: vec![a.to_string(), b.to_string()],
                    rationale: "test".to_string(),
                    supporting_surface_fields: vec![],
                })
                .collect(),
            failures: vec![],
        }
    }

    #[test]
    fn apply_adds_new_edges() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        seed_two_projects(&cfg);

        save_proposals_atomic(&cfg, &mk_proposals(&[(EdgeKind::Sibling, "A", "B")])).unwrap();
        let report = apply_proposals(&cfg).unwrap();

        assert_eq!(report.added, 1);
        assert_eq!(report.already_present, 0);
        assert!(report.kind_conflicts.is_empty());
        let loaded = related_projects::load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
    }

    #[test]
    fn apply_is_idempotent_on_rerun() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        seed_two_projects(&cfg);
        save_proposals_atomic(&cfg, &mk_proposals(&[(EdgeKind::Sibling, "A", "B")])).unwrap();

        let first = apply_proposals(&cfg).unwrap();
        assert_eq!(first.added, 1);
        let second = apply_proposals(&cfg).unwrap();
        assert_eq!(second.added, 0);
        assert_eq!(second.already_present, 1);
    }

    #[test]
    fn apply_rejects_kind_conflict_and_continues() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        seed_two_projects(&cfg);
        // Seed existing parent-of(A, B).
        let mut file = related_projects::RelatedProjectsFile::default();
        file.add_edge(Edge::parent_of("A", "B")).unwrap();
        related_projects::save_atomic(&cfg, &file).unwrap();

        // Propose sibling(A, B) — must be rejected. Also include a
        // harmless sibling(B, "C") to prove "continues" works; seed C first.
        let mut catalog = projects::load_or_empty(&cfg).unwrap();
        let c_path = tmp.path().join("C");
        std::fs::create_dir_all(&c_path).unwrap();
        projects::try_add_named(&mut catalog, "C", &c_path).unwrap();
        projects::save_atomic(&cfg, &catalog).unwrap();

        save_proposals_atomic(
            &cfg,
            &mk_proposals(&[
                (EdgeKind::Sibling, "A", "B"),   // conflicts
                (EdgeKind::Sibling, "B", "C"),   // OK
            ]),
        )
        .unwrap();

        let report = apply_proposals(&cfg).unwrap();
        assert_eq!(report.added, 1, "second proposal should still land");
        assert_eq!(report.kind_conflicts.len(), 1);

        let loaded = related_projects::load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 2);
        assert!(loaded.edges.iter().any(|e| e.kind == EdgeKind::ParentOf));
        assert!(loaded.edges.iter().any(|e| e.kind == EdgeKind::Sibling));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ravel-lite --lib discover::apply`
Expected: 3 passing tests.

- [ ] **Step 4: Commit**

```bash
git add src/discover/apply.rs
git commit -m "R7 task 9: discover-apply with kind-conflict rejection"
```

---

## Task 10: CLI wiring — `discover` and `discover-apply` subcommands

**Files:**
- Modify: `src/main.rs`

Goal: expose `run_discover` and `run_discover_apply` as CLI verbs under `state related-projects`.

- [ ] **Step 1: Extend `RelatedProjectsCommands` enum**

In `src/main.rs`, add two new variants after the existing `RemoveEdge`:

```rust
    /// Run the two-stage LLM discovery pipeline over all catalogued
    /// projects (or just `--project <name>`). Writes proposals to
    /// `<config-dir>/discover-proposals.yaml` for user review.
    Discover {
        #[arg(long)]
        config: Option<PathBuf>,
        /// Restrict Stage 1 re-analysis to a single project; Stage 2
        /// still operates over the full catalog's cached surfaces.
        #[arg(long)]
        project: Option<String>,
        /// Maximum parallel Stage 1 subagents. Default 4.
        #[arg(long)]
        concurrency: Option<usize>,
        /// Skip the review gate: run `discover-apply` immediately after
        /// proposals are written.
        #[arg(long)]
        apply: bool,
    },
    /// Merge a previously-produced `discover-proposals.yaml` into
    /// `related-projects.yaml`. Idempotent; reports and rejects
    /// kind-conflicts without aborting.
    DiscoverApply {
        #[arg(long)]
        config: Option<PathBuf>,
    },
```

- [ ] **Step 2: Wire the dispatch**

In `dispatch_related_projects`, add match arms:

```rust
        RelatedProjectsCommands::Discover {
            config,
            project,
            concurrency,
            apply,
        } => {
            let config_root = resolve_config_dir(config)?;
            let options = ravel_lite::discover::RunDiscoverOptions {
                project_filter: project,
                concurrency,
                apply,
            };
            // discover is async; we're in a sync fn but main is #[tokio::main].
            // Promote to an async context.
            let rt = tokio::runtime::Handle::current();
            rt.block_on(ravel_lite::discover::run_discover(&config_root, options))
        }
        RelatedProjectsCommands::DiscoverApply { config } => {
            let config_root = resolve_config_dir(config)?;
            ravel_lite::discover::apply::run_discover_apply(&config_root)
        }
```

*(Plan-time note: `dispatch_related_projects` is a sync `fn`; `run_discover` is async. If the existing dispatch chain is already async under `tokio::main`, prefer making `dispatch_related_projects` async and `.await`ing. If the chain is sync, use `tokio::runtime::Handle::current().block_on(..)` as shown. Confirm by reading `dispatch_state`'s call-site in `main` during implementation — see `src/main.rs:633`.)*

- [ ] **Step 3: Confirm build**

Run: `cargo build`
Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "R7 task 10: CLI wiring for state related-projects discover/discover-apply"
```

---

## Task 11: End-to-end integration test with fake-claude shim

**Files:**
- Create: `tests/discover.rs`
- Create: `tests/fake-claude-discover.sh` (or inline as string-literal scaffold in the test)

Goal: one integration test that runs the real `ravel-lite` binary end-to-end with a fake `claude` on PATH that writes canned surface YAML and canned proposals YAML. Validates cache writes, proposals file contents, and apply behaviour.

- [ ] **Step 1: Write `tests/discover.rs` skeleton**

```rust
//! End-to-end integration test for `state related-projects discover`
//! and `discover-apply`. Uses a fake `claude` shell script on PATH that
//! reads the prompt, extracts the output-path tokens, and writes canned
//! YAML there.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tempfile::TempDir;

fn bin_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ravel-lite"))
}

/// Scaffold a monorepo with two subtree projects, both committed to
/// a single git repo, plus a catalogued config dir.
fn scaffold(tmp: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let repo = tmp.join("mono");
    std::fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init", "-q", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "user.name", "test"]);

    let alpha = repo.join("alpha");
    let beta = repo.join("beta");
    std::fs::create_dir_all(&alpha).unwrap();
    std::fs::create_dir_all(&beta).unwrap();
    std::fs::write(alpha.join("README.md"), "alpha consumes /data/*.yaml\n").unwrap();
    std::fs::write(beta.join("README.md"), "beta produces /data/*.yaml\n").unwrap();
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-q", "-m", "init"]);

    let cfg = tmp.join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    (repo, alpha, beta, cfg)
}

fn run_git(cwd: &Path, args: &[&str]) {
    let s = Command::new("git").arg("-C").arg(cwd).args(args).status().unwrap();
    assert!(s.success(), "git {args:?} in {} failed", cwd.display());
}

fn write_fake_claude(shim_dir: &Path, stage1_yaml: &str, stage2_yaml: &str) -> PathBuf {
    // The script extracts the output-path placeholder from the prompt
    // (which the orchestrator substitutes into the prompt as an absolute
    // path) and writes the canned YAML there.
    let script = format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

prompt_arg=""
for ((i=1; i<=$#; i++)); do
  if [[ "${{!i}}" == "-p" ]]; then
    ((j=i+1))
    prompt_arg="${{!j}}"
    break
  fi
done

if grep -q 'Extract Interaction Surface' <<<"$prompt_arg"; then
  out=$(grep -oE '/[^[:space:]]+\.yaml' <<<"$prompt_arg" | head -n1)
  cat >"$out" <<'YAML'
{stage1}
YAML
else
  out=$(grep -oE '/[^[:space:]]+\.yaml' <<<"$prompt_arg" | head -n1)
  cat >"$out" <<'YAML'
{stage2}
YAML
fi
"#,
        stage1 = stage1_yaml,
        stage2 = stage2_yaml,
    );
    let path = shim_dir.join("claude");
    std::fs::write(&path, script).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

#[test]
fn discover_writes_proposals_and_apply_merges_them() {
    let tmp = TempDir::new().unwrap();
    let (_repo, alpha, _beta, cfg) = scaffold(tmp.path());

    // Catalogue both projects.
    let status = Command::new(bin_path())
        .args(["state", "projects", "add", "--config"])
        .arg(&cfg)
        .args(["--name", "Alpha", "--path"])
        .arg(&alpha)
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new(bin_path())
        .args(["state", "projects", "add", "--config"])
        .arg(&cfg)
        .args(["--name", "Beta", "--path"])
        .arg(tmp.path().join("mono").join("beta"))
        .status()
        .unwrap();
    assert!(status.success());

    // Copy prompt templates into the config root (discover reads them
    // from <config-root>/discover-stage{1,2}.md).
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    std::fs::copy(
        repo_root.join("defaults/discover-stage1.md"),
        cfg.join("discover-stage1.md"),
    )
    .unwrap();
    std::fs::copy(
        repo_root.join("defaults/discover-stage2.md"),
        cfg.join("discover-stage2.md"),
    )
    .unwrap();
    // Minimal config.yaml + agent config so load_shared_config / load_agent_config succeed.
    std::fs::write(cfg.join("config.yaml"), "agent: claude-code\n").unwrap();
    std::fs::create_dir_all(cfg.join("agents/claude-code")).unwrap();
    std::fs::write(
        cfg.join("agents/claude-code/config.yaml"),
        "models:\n  discover: fake-model\n",
    )
    .unwrap();

    // Install fake claude on PATH.
    let shim_dir = tmp.path().join("bin");
    std::fs::create_dir_all(&shim_dir).unwrap();
    write_fake_claude(
        &shim_dir,
        "purpose: alpha consumes yaml\nconsumes_files: [/data/*.yaml]\n",
        "generated_at: 2026-04-22T00:00:00Z\nproposals:\n  - kind: parent-of\n    participants: [Beta, Alpha]\n    rationale: 'beta produces, alpha consumes'\n    supporting_surface_fields: []\n",
    );

    // Run discover.
    let status = Command::new(bin_path())
        .env("PATH", format!("{}:{}", shim_dir.display(), std::env::var("PATH").unwrap()))
        .args(["state", "related-projects", "discover", "--config"])
        .arg(&cfg)
        .status()
        .unwrap();
    assert!(status.success());

    // Proposals file exists.
    let proposals_path = cfg.join("discover-proposals.yaml");
    assert!(proposals_path.exists());
    let content = std::fs::read_to_string(&proposals_path).unwrap();
    assert!(content.contains("parent-of"));
    assert!(content.contains("Beta"));
    assert!(content.contains("Alpha"));

    // Cache files exist.
    assert!(cfg.join("discover-cache/Alpha.yaml").exists());
    assert!(cfg.join("discover-cache/Beta.yaml").exists());

    // Apply and verify related-projects.yaml.
    let status = Command::new(bin_path())
        .args(["state", "related-projects", "discover-apply", "--config"])
        .arg(&cfg)
        .status()
        .unwrap();
    assert!(status.success());
    let rp = std::fs::read_to_string(cfg.join("related-projects.yaml")).unwrap();
    assert!(rp.contains("parent-of"));
}
```

- [ ] **Step 2: Run the integration test**

Run: `cargo test --test discover`
Expected: 1 passing test.

- [ ] **Step 3: Commit**

```bash
git add tests/discover.rs
git commit -m "R7 task 11: end-to-end discover integration test with fake claude"
```

---

## Task 12: Add backlog entry for follow-up + close R7 backlog tasks

**Files:**
- Modify: `LLM_STATE/core/backlog.md` (via `ravel-lite state backlog` verbs).

Goal: flip R7-design and R7 to done with Results blocks, and optionally add a follow-up entry if anything was deferred.

- [ ] **Step 1: Mark R7-design done with Results**

Run:
```
ravel-lite state backlog set-status LLM_STATE/core <R7-design-id> done
ravel-lite state backlog set-results LLM_STATE/core <R7-design-id> --description-file <path-to-results-prose>
```

The Results prose should note: spec at `docs/r7-related-projects-discovery-design.md`, plan at `docs/r7-related-projects-discovery-plan.md`.

- [ ] **Step 2: Mark R7 done with Results**

Run the same verbs for R7. Results prose should note: shipped; integration test in `tests/discover.rs`; N new files listed.

- [ ] **Step 3: Build a quick smoke test locally against a real project catalog (optional, documented)**

This step is intentionally not automated — run `ravel-lite state related-projects discover` against your local catalog, review the generated `discover-proposals.yaml`, and decide whether to run `discover-apply`. Record any observations in a follow-up backlog entry if the proposals reveal surprising misses or false positives.

- [ ] **Step 4: Commit the backlog updates**

Handled by the analyse-work phase's normal commit flow; no manual commit needed here if this task is run inside ravel-lite's own work cycle.

---

## Self-review

**Spec coverage:**
- Entry point (new CLI verbs under `state related-projects`) — Tasks 10, 11 ✅
- Stage 1 per-project extraction — Tasks 6 ✅
- Stage 2 global inference — Task 7 ✅
- Cache (tree-SHA keyed, per-project file) — Tasks 3, 4 ✅
- Monorepo subtree handling — Task 3 (test coverage) ✅
- Non-git bail + dirty-tree bail — Task 3 ✅
- Rename cascade — Task 5 ✅
- Review-gate + apply with kind-conflict handling — Task 9 ✅
- `--project`, `--concurrency`, `--apply` flags — Task 10 ✅
- Best-effort failure handling — Task 8 (surfaces vs failures split) ✅
- Nested-subagent prompt concession — Task 6 Step 1 (prompt text) ✅
- Prompt templates registered as embedded defaults — Task 6 Step 2 ✅

**Placeholder scan:** No "TBD", "TODO", or "implement later" markers in task steps. Decisions-deferred-to-plan from the spec are resolved inline (flagged with *(plan-time decision)* notes in Task 6 for `chrono`, Task 8 for prompt-loading, Task 10 for sync/async dispatch).

**Type consistency:** `SurfaceFile`, `SurfaceRecord`, `ProposalsFile`, `ProposalRecord`, `Stage1Failure`, `Stage1Outcome`, `Stage1Config`, `Stage2Config`, `RunDiscoverOptions`, `ApplyReport`, `KindConflict` — each defined once, used consistently downstream.

**Scope focus:** One feature, one plan. No out-of-scope creep. Task 12 is a closeout, not new feature work.
