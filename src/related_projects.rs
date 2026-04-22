//! Global per-user related-projects edge list at
//! `<config_root>/related-projects.yaml`.
//!
//! Edges describe sibling (unordered) and parent-of (ordered)
//! relationships between projects by *name*. Names resolve to absolute
//! paths via the per-user `projects.yaml` catalog, so the edge list
//! itself is shareable between users with different workstation layouts.
//!
//! Two kinds share a single `participants: [A, B]` shape:
//! * `sibling`: unordered pair — canonicalised as sorted for dedup.
//! * `parent-of`: ordered pair `[parent, child]` — direction is semantic.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::projects::{self, ProjectsCatalog};

pub const RELATED_PROJECTS_FILE: &str = "related-projects.yaml";

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    Sibling,
    ParentOf,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Sibling => "sibling",
            EdgeKind::ParentOf => "parent-of",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "sibling" => Some(EdgeKind::Sibling),
            "parent-of" => Some(EdgeKind::ParentOf),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub kind: EdgeKind,
    pub participants: Vec<String>,
}

impl Edge {
    pub fn sibling(a: impl Into<String>, b: impl Into<String>) -> Self {
        Edge {
            kind: EdgeKind::Sibling,
            participants: vec![a.into(), b.into()],
        }
    }

    pub fn parent_of(parent: impl Into<String>, child: impl Into<String>) -> Self {
        Edge {
            kind: EdgeKind::ParentOf,
            participants: vec![parent.into(), child.into()],
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.participants.len() != 2 {
            bail!(
                "edge must have exactly 2 participants, got {} ({:?})",
                self.participants.len(),
                self.participants
            );
        }
        if self.participants[0] == self.participants[1] {
            bail!(
                "edge participants must be distinct; '{}' appears twice",
                self.participants[0]
            );
        }
        Ok(())
    }

    /// Canonical key for dedup: sibling is order-insensitive, parent-of
    /// is order-sensitive (direction is part of the identity).
    fn canonical_key(&self) -> (EdgeKind, Vec<String>) {
        match self.kind {
            EdgeKind::Sibling => {
                let mut sorted = self.participants.clone();
                sorted.sort();
                (self.kind, sorted)
            }
            EdgeKind::ParentOf => (self.kind, self.participants.clone()),
        }
    }

    pub fn involves(&self, project_name: &str) -> bool {
        self.participants.iter().any(|p| p == project_name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedProjectsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub edges: Vec<Edge>,
}

impl Default for RelatedProjectsFile {
    fn default() -> Self {
        RelatedProjectsFile {
            schema_version: SCHEMA_VERSION,
            edges: Vec::new(),
        }
    }
}

impl RelatedProjectsFile {
    /// Append `edge` if not already present (by canonical key). Returns
    /// true when the edge was newly added, false on dedup no-op.
    pub fn add_edge(&mut self, edge: Edge) -> Result<bool> {
        edge.validate()?;
        let key = edge.canonical_key();
        if self.edges.iter().any(|e| e.canonical_key() == key) {
            return Ok(false);
        }
        self.edges.push(edge);
        Ok(true)
    }

    /// Remove the edge matching the canonical key of `edge`. Returns
    /// true if an edge was removed.
    pub fn remove_edge(&mut self, edge: &Edge) -> Result<bool> {
        edge.validate()?;
        let key = edge.canonical_key();
        let before = self.edges.len();
        self.edges.retain(|e| e.canonical_key() != key);
        Ok(self.edges.len() != before)
    }

}

pub fn load_or_empty(config_root: &Path) -> Result<RelatedProjectsFile> {
    let path = config_root.join(RELATED_PROJECTS_FILE);
    if !path.exists() {
        return Ok(RelatedProjectsFile::default());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let file: RelatedProjectsFile = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    if file.schema_version != SCHEMA_VERSION {
        bail!(
            "{} has schema_version {} but this ravel-lite expects {}; aborting to avoid data loss",
            path.display(),
            file.schema_version,
            SCHEMA_VERSION
        );
    }
    for edge in &file.edges {
        edge.validate()
            .with_context(|| format!("invalid edge in {}", path.display()))?;
    }
    Ok(file)
}

pub fn save_atomic(config_root: &Path, file: &RelatedProjectsFile) -> Result<()> {
    let path = config_root.join(RELATED_PROJECTS_FILE);
    let yaml = serde_yaml::to_string(file)
        .context("Failed to serialise related-projects to YAML")?;
    let tmp = config_root.join(format!(".{RELATED_PROJECTS_FILE}.tmp"));
    std::fs::write(&tmp, yaml.as_bytes())
        .with_context(|| format!("Failed to write temp file {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("Failed to rename {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

// ---------- CLI handlers ----------

/// Emit the file as YAML. When `plan_dir` is supplied, filter to edges
/// that involve the project derived from that plan (via the per-user
/// catalog); edges on unknown projects are omitted.
pub fn run_list(config_root: &Path, plan_dir: Option<&Path>) -> Result<()> {
    let file = load_or_empty(config_root)?;
    let filtered = match plan_dir {
        None => file,
        Some(plan) => {
            let catalog = projects::load_or_empty(config_root)?;
            let project_name = resolve_plan_project_name(&catalog, plan)?;
            RelatedProjectsFile {
                schema_version: file.schema_version,
                edges: file
                    .edges
                    .into_iter()
                    .filter(|e| e.involves(&project_name))
                    .collect(),
            }
        }
    };
    let yaml = serde_yaml::to_string(&filtered)
        .context("Failed to serialise related-projects to YAML")?;
    print!("{yaml}");
    Ok(())
}

pub fn run_add_edge(
    config_root: &Path,
    kind: EdgeKind,
    a: &str,
    b: &str,
) -> Result<()> {
    let catalog = projects::load_or_empty(config_root)?;
    require_project_known(&catalog, a)?;
    require_project_known(&catalog, b)?;
    let mut file = load_or_empty(config_root)?;
    let edge = Edge {
        kind,
        participants: vec![a.to_string(), b.to_string()],
    };
    let added = file.add_edge(edge)?;
    if !added {
        // Non-fatal: echo to stderr so scripts can notice without hard-failing.
        eprintln!(
            "edge already present (kind={}, {} / {}); no change.",
            kind.as_str(),
            a,
            b
        );
        return Ok(());
    }
    save_atomic(config_root, &file)
}

/// Cascade of `projects::run_rename`: any edge referencing `old` is
/// rewritten to reference `new` instead. No-op when the file is absent
/// (a catalog without any related-projects.yaml is a valid state — the
/// rename on the catalog side must still succeed).
///
/// Callers must already hold the invariant that `new` is not already
/// catalogued; `run_rename` checks this before invoking the cascade.
/// Under that invariant no edge self-loop or duplicate can emerge from
/// the substitution, so the rewrite is mechanical.
pub fn rename_project_in_edges(config_root: &Path, old: &str, new: &str) -> Result<()> {
    let path = config_root.join(RELATED_PROJECTS_FILE);
    if !path.exists() {
        return Ok(());
    }
    let mut file = load_or_empty(config_root)?;
    let mut changed = false;
    for edge in &mut file.edges {
        for participant in &mut edge.participants {
            if participant == old {
                *participant = new.to_string();
                changed = true;
            }
        }
    }
    if !changed {
        return Ok(());
    }
    save_atomic(config_root, &file)
}

pub fn run_remove_edge(
    config_root: &Path,
    kind: EdgeKind,
    a: &str,
    b: &str,
) -> Result<()> {
    let mut file = load_or_empty(config_root)?;
    let edge = Edge {
        kind,
        participants: vec![a.to_string(), b.to_string()],
    };
    let removed = file.remove_edge(&edge)?;
    if !removed {
        bail!(
            "no matching edge to remove (kind={}, {} / {})",
            kind.as_str(),
            a,
            b
        );
    }
    save_atomic(config_root, &file)
}

fn require_project_known(catalog: &ProjectsCatalog, name: &str) -> Result<()> {
    if catalog.find_by_name(name).is_none() {
        bail!(
            "project '{}' is not in the catalog; add it with `ravel-lite state projects add --name {} --path <abs-path>`",
            name,
            name
        );
    }
    Ok(())
}

fn resolve_plan_project_name(
    catalog: &ProjectsCatalog,
    plan_dir: &Path,
) -> Result<String> {
    let project_path = plan_project_path(plan_dir)?;
    if let Some(entry) = catalog.find_by_path(&project_path) {
        return Ok(entry.name.clone());
    }
    bail!(
        "plan's project {} is not in the catalog; run `ravel-lite run` once or add it with `ravel-lite state projects add`",
        project_path.display()
    )
}

/// Derive `<plan>/../..` as the project path. Matches
/// `git::project_root_for_plan` semantics but returns a PathBuf without
/// going through the String detour.
fn plan_project_path(plan_dir: &Path) -> Result<PathBuf> {
    let parent = plan_dir
        .parent()
        .with_context(|| format!("plan dir {} has no parent", plan_dir.display()))?;
    let grandparent = parent
        .parent()
        .with_context(|| format!("plan dir {} has no grandparent (expected <project>/<state-dir>/<plan>)", plan_dir.display()))?;
    Ok(grandparent.to_path_buf())
}

// ---------- Migration from legacy related-plans.md ----------

/// What the migration produced: edges it intended to add, plus which
/// ones were actually new (after dedup) and which were already present.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    pub added: Vec<Edge>,
    pub skipped_existing: Vec<Edge>,
}

#[derive(Debug, Clone, Default)]
pub struct MigrateRelatedProjectsOptions {
    pub dry_run: bool,
    pub delete_original: bool,
}

pub fn run_migrate_related_projects(
    config_root: &Path,
    plan_dir: &Path,
    options: &MigrateRelatedProjectsOptions,
) -> Result<MigrationReport> {
    let source = plan_dir.join("related-plans.md");
    if !source.exists() {
        bail!(
            "no related-plans.md at {}; nothing to migrate",
            source.display()
        );
    }
    let text = std::fs::read_to_string(&source)
        .with_context(|| format!("failed to read {}", source.display()))?;

    let project_path = plan_project_path(plan_dir)?;
    let dev_root = project_path
        .parent()
        .with_context(|| format!("project {} has no parent (DEV_ROOT)", project_path.display()))?;
    let substituted = substitute_path_tokens(
        &text,
        &project_path,
        plan_dir,
        dev_root,
    );

    let parsed = parse_related_plans_sections(&substituted)
        .with_context(|| format!("failed to parse {}", source.display()))?;

    // Resolve the plan's own project into the catalog first; build
    // edges entirely in memory before touching disk so a catalog
    // rejection on any peer leaves the existing file intact.
    let mut catalog = projects::load_or_empty(config_root)?;
    let plan_project_name = ensure_project_in_catalog(&mut catalog, &project_path)?;

    let mut proposed_edges: Vec<Edge> = Vec::new();
    for bullet in &parsed.siblings {
        let peer = ensure_project_in_catalog(&mut catalog, &bullet.path)?;
        if peer == plan_project_name {
            continue;
        }
        proposed_edges.push(Edge::sibling(&plan_project_name, &peer));
    }
    for bullet in &parsed.parents {
        let peer = ensure_project_in_catalog(&mut catalog, &bullet.path)?;
        if peer == plan_project_name {
            continue;
        }
        // "Parents" in related-plans.md = projects I depend on,
        // i.e. they are parents of me: parent-of[peer, me].
        proposed_edges.push(Edge::parent_of(&peer, &plan_project_name));
    }
    for bullet in &parsed.children {
        let peer = ensure_project_in_catalog(&mut catalog, &bullet.path)?;
        if peer == plan_project_name {
            continue;
        }
        // "Children" = projects that depend on me:
        // parent-of[me, peer].
        proposed_edges.push(Edge::parent_of(&plan_project_name, &peer));
    }

    if options.dry_run {
        let file = load_or_empty(config_root)?;
        let mut report = MigrationReport::default();
        for edge in proposed_edges {
            let key = edge.canonical_key();
            if file.edges.iter().any(|e| e.canonical_key() == key) {
                report.skipped_existing.push(edge);
            } else {
                report.added.push(edge);
            }
        }
        return Ok(report);
    }

    // Commit any catalog changes first. Doing it here (after parse
    // success, before edge merge) keeps failures before this point from
    // leaving orphan catalog entries.
    projects::save_atomic(config_root, &catalog)?;

    let mut file = load_or_empty(config_root)?;
    let mut report = MigrationReport::default();
    for edge in proposed_edges {
        let edge_clone = edge.clone();
        if file.add_edge(edge)? {
            report.added.push(edge_clone);
        } else {
            report.skipped_existing.push(edge_clone);
        }
    }

    if !report.added.is_empty() {
        save_atomic(config_root, &file)?;
    }

    if options.delete_original {
        std::fs::remove_file(&source)
            .with_context(|| format!("failed to delete {}", source.display()))?;
    }

    Ok(report)
}

/// Minimal inline substitution for migration. `substitute_tokens`
/// hard-errors on any unresolved token (including `{{RELATED_PLANS}}`,
/// which is meaningless inside related-plans.md itself), so we can't
/// reuse it; the set of tokens a legacy related-plans.md might contain
/// is just path tokens.
fn substitute_path_tokens(
    text: &str,
    project: &Path,
    plan: &Path,
    dev_root: &Path,
) -> String {
    text.replace("{{DEV_ROOT}}", &dev_root.to_string_lossy())
        .replace("{{PROJECT}}", &project.to_string_lossy())
        .replace("{{PLAN}}", &plan.to_string_lossy())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ParsedRelatedPlans {
    siblings: Vec<Bullet>,
    parents: Vec<Bullet>,
    children: Vec<Bullet>,
}

#[derive(Debug, PartialEq, Eq)]
struct Bullet {
    path: PathBuf,
}

/// Parse the legacy `related-plans.md` format:
///
/// ```markdown
/// ## Siblings
/// - /abs/path/A — description
///
/// ## Parents
/// - /abs/path/B — description
///
/// ## Children
/// - /abs/path/C — description
/// ```
///
/// Sections can appear in any order; missing sections are treated as
/// empty. Non-`-` lines within a section are ignored (so the
/// `_No active sibling plans._` placeholder becomes zero bullets).
fn parse_related_plans_sections(text: &str) -> Result<ParsedRelatedPlans> {
    let mut parsed = ParsedRelatedPlans::default();
    let mut current: Option<&mut Vec<Bullet>> = None;

    for line in text.lines() {
        let trimmed = line.trim_end();
        if let Some(heading) = trimmed.strip_prefix("## ") {
            let h = heading.trim();
            current = match h {
                "Siblings" => Some(&mut parsed.siblings),
                "Parents" => Some(&mut parsed.parents),
                "Children" => Some(&mut parsed.children),
                _ => None,
            };
            continue;
        }
        if let Some(bucket) = current.as_deref_mut() {
            if let Some(bullet) = parse_bullet_line(trimmed)? {
                bucket.push(bullet);
            }
        }
    }
    Ok(parsed)
}

fn parse_bullet_line(line: &str) -> Result<Option<Bullet>> {
    let stripped = match line.strip_prefix("- ") {
        Some(rest) => rest.trim(),
        None => return Ok(None),
    };
    if stripped.is_empty() {
        return Ok(None);
    }
    // Path ends at the first em-dash separator, or at end-of-line.
    let path_str = match stripped.find(" — ") {
        Some(idx) => &stripped[..idx],
        None => stripped,
    };
    let path_str = path_str.trim();
    if path_str.is_empty() {
        bail!("bullet has empty path portion: {line:?}");
    }
    let path = PathBuf::from(path_str);
    if !path.is_absolute() {
        bail!(
            "bullet path must be absolute after token substitution; got {:?}. \
             Check that the related-plans.md uses {{{{DEV_ROOT}}}} / {{{{PROJECT}}}} tokens.",
            path_str
        );
    }
    Ok(Some(Bullet { path }))
}

/// Ensure `project_path` is in `catalog`, mutating it in memory. The
/// caller is responsible for persisting when migration commits.
/// `NameCollision` bails with an actionable message because a one-shot
/// migration CLI has no tty to prompt on.
fn ensure_project_in_catalog(
    catalog: &mut ProjectsCatalog,
    project_path: &Path,
) -> Result<String> {
    match projects::auto_add(catalog, project_path)? {
        projects::AutoAddOutcome::AlreadyCatalogued { name } => Ok(name),
        projects::AutoAddOutcome::Added { name } => Ok(name),
        projects::AutoAddOutcome::NameCollision {
            attempted_name,
            existing_path,
        } => bail!(
            "migration hit a name collision: path {} would be added as '{}', but that name is already used for {}. \
             Resolve manually with `ravel-lite state projects add --name <alt-name> --path {}` then re-run.",
            project_path.display(),
            attempted_name,
            existing_path.display(),
            project_path.display()
        ),
    }
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn mk_catalog_with(config_root: &Path, names: &[&str]) -> Vec<PathBuf> {
        let mut catalog = ProjectsCatalog::default();
        let mut paths = Vec::new();
        for name in names {
            let p = config_root.join(name);
            std::fs::create_dir_all(&p).unwrap();
            projects::try_add_named(&mut catalog, name, &p).unwrap();
            paths.push(p);
        }
        projects::save_atomic(config_root, &catalog).unwrap();
        paths
    }

    #[test]
    fn load_or_empty_returns_empty_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let file = load_or_empty(tmp.path()).unwrap();
        assert_eq!(file.schema_version, SCHEMA_VERSION);
        assert!(file.edges.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = TempDir::new().unwrap();
        let mut file = RelatedProjectsFile::default();
        file.add_edge(Edge::sibling("Alpha", "Beta")).unwrap();
        file.add_edge(Edge::parent_of("Alpha", "Gamma")).unwrap();

        save_atomic(tmp.path(), &file).unwrap();
        let loaded = load_or_empty(tmp.path()).unwrap();
        assert_eq!(loaded, file);
    }

    #[test]
    fn load_rejects_unknown_schema_version() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join(RELATED_PROJECTS_FILE),
            "schema_version: 99\nedges: []\n",
        )
        .unwrap();
        let err = load_or_empty(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"));
        assert!(msg.contains("99"));
    }

    #[test]
    fn add_sibling_is_idempotent_regardless_of_order() {
        let mut file = RelatedProjectsFile::default();
        assert!(file.add_edge(Edge::sibling("A", "B")).unwrap());
        assert!(!file.add_edge(Edge::sibling("A", "B")).unwrap());
        assert!(!file.add_edge(Edge::sibling("B", "A")).unwrap());
        assert_eq!(file.edges.len(), 1);
    }

    #[test]
    fn add_parent_of_is_order_sensitive() {
        let mut file = RelatedProjectsFile::default();
        assert!(file.add_edge(Edge::parent_of("Parent", "Child")).unwrap());
        // Reverse direction is a *different* edge.
        assert!(file.add_edge(Edge::parent_of("Child", "Parent")).unwrap());
        assert_eq!(file.edges.len(), 2);
    }

    #[test]
    fn remove_edge_returns_false_when_absent() {
        let mut file = RelatedProjectsFile::default();
        let removed = file.remove_edge(&Edge::sibling("X", "Y")).unwrap();
        assert!(!removed);
    }

    #[test]
    fn remove_sibling_works_regardless_of_participant_order() {
        let mut file = RelatedProjectsFile::default();
        file.add_edge(Edge::sibling("A", "B")).unwrap();
        let removed = file.remove_edge(&Edge::sibling("B", "A")).unwrap();
        assert!(removed);
        assert!(file.edges.is_empty());
    }

    #[test]
    fn edge_validate_rejects_self_loop() {
        let err = Edge::sibling("A", "A").validate().unwrap_err();
        assert!(format!("{err:#}").contains("distinct"));
    }

    #[test]
    fn edge_validate_rejects_wrong_participant_count() {
        let e = Edge {
            kind: EdgeKind::Sibling,
            participants: vec!["solo".into()],
        };
        let err = e.validate().unwrap_err();
        assert!(format!("{err:#}").contains("exactly 2"));
    }

    #[test]
    fn run_add_edge_rejects_unknown_project() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["Known"]);

        let err = run_add_edge(&cfg, EdgeKind::Sibling, "Known", "Stranger").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Stranger"));
        assert!(msg.contains("state projects add"));
        // File must not have been written on rejection.
        assert!(!cfg.join(RELATED_PROJECTS_FILE).exists());
    }

    #[test]
    fn run_add_edge_persists_and_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["Alpha", "Beta"]);

        run_add_edge(&cfg, EdgeKind::Sibling, "Alpha", "Beta").unwrap();
        run_add_edge(&cfg, EdgeKind::Sibling, "Beta", "Alpha").unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
    }

    #[test]
    fn run_remove_edge_errors_when_absent() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();

        let err = run_remove_edge(&cfg, EdgeKind::Sibling, "A", "B").unwrap_err();
        assert!(format!("{err:#}").contains("no matching edge"));
    }

    #[test]
    fn edge_involves_matches_either_participant() {
        let e = Edge::sibling("Alpha", "Beta");
        assert!(e.involves("Alpha"));
        assert!(e.involves("Beta"));
        assert!(!e.involves("Gamma"));
    }

    #[test]
    fn plan_project_path_derives_grandparent() {
        let path = plan_project_path(Path::new("/a/b/c")).unwrap();
        assert_eq!(path, PathBuf::from("/a"));
    }

    #[test]
    fn plan_project_path_errors_on_shallow_input() {
        let err = plan_project_path(Path::new("/")).unwrap_err();
        assert!(format!("{err:#}").contains("parent") || format!("{err:#}").contains("grandparent"));
    }

    // ---------- Migration tests ----------

    /// Scaffold a realistic layout inside `tmp` and return `(config_root, plan_dir)`.
    /// Creates a config dir with a seeded catalog, a project dir
    /// `tmp/projects/<name>` for each of `me` plus `peers`, and a plan
    /// dir at `tmp/projects/<me>/LLM_STATE/core`.
    fn scaffold_plan(
        tmp: &Path,
        me: &str,
        peers: &[&str],
    ) -> (PathBuf, PathBuf) {
        let cfg = tmp.join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let projects_root = tmp.join("projects");

        let mut catalog = ProjectsCatalog::default();
        for name in std::iter::once(me).chain(peers.iter().copied()) {
            let p = projects_root.join(name);
            std::fs::create_dir_all(&p).unwrap();
            projects::try_add_named(&mut catalog, name, &p).unwrap();
        }
        projects::save_atomic(&cfg, &catalog).unwrap();

        let plan_dir = projects_root.join(me).join("LLM_STATE").join("core");
        std::fs::create_dir_all(&plan_dir).unwrap();
        (cfg, plan_dir)
    }

    fn write_related_plans_md(plan_dir: &Path, body: &str) {
        std::fs::write(plan_dir.join("related-plans.md"), body).unwrap();
    }

    #[test]
    fn parse_related_plans_handles_three_sections() {
        let body = "\
# Related Plans

## Siblings
- /dev/Peer1 — peer one

## Parents
- /dev/Up1 — upstream

## Children
- /dev/Down1 — downstream
- /dev/Down2 — another downstream
";
        let parsed = parse_related_plans_sections(body).unwrap();
        assert_eq!(parsed.siblings.len(), 1);
        assert_eq!(parsed.parents.len(), 1);
        assert_eq!(parsed.children.len(), 2);
        assert_eq!(parsed.siblings[0].path, PathBuf::from("/dev/Peer1"));
        assert_eq!(parsed.children[1].path, PathBuf::from("/dev/Down2"));
    }

    #[test]
    fn parse_related_plans_treats_placeholder_prose_as_empty() {
        let body = "\
## Siblings
_No active sibling plans._

## Parents
";
        let parsed = parse_related_plans_sections(body).unwrap();
        assert!(parsed.siblings.is_empty());
        assert!(parsed.parents.is_empty());
        assert!(parsed.children.is_empty());
    }

    #[test]
    fn parse_bullet_rejects_relative_path() {
        let err = parse_bullet_line("- relative/path — desc").unwrap_err();
        assert!(format!("{err:#}").contains("absolute"));
    }

    #[test]
    fn migrate_writes_sibling_and_directional_parent_edges() {
        let tmp = TempDir::new().unwrap();
        let (cfg, plan_dir) = scaffold_plan(tmp.path(), "Me", &["Peer", "Up", "Down"]);
        let projects_root = tmp.path().join("projects");

        let body = format!(
            "# Related Plans\n\n\
             ## Siblings\n- {peer} — peer\n\n\
             ## Parents\n- {up} — upstream\n\n\
             ## Children\n- {down} — downstream\n",
            peer = projects_root.join("Peer").display(),
            up = projects_root.join("Up").display(),
            down = projects_root.join("Down").display(),
        );
        write_related_plans_md(&plan_dir, &body);

        let report = run_migrate_related_projects(
            &cfg,
            &plan_dir,
            &MigrateRelatedProjectsOptions::default(),
        )
        .unwrap();

        assert_eq!(report.added.len(), 3);
        assert!(report.skipped_existing.is_empty());

        let file = load_or_empty(&cfg).unwrap();
        assert_eq!(file.edges.len(), 3);

        // Sibling edge: both orderings should match.
        assert!(file
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Sibling
                && e.canonical_key().1 == vec!["Me".to_string(), "Peer".to_string()]));
        // Parent-of: Parents section → peer is parent of me.
        assert!(file
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::ParentOf && e.participants == vec!["Up", "Me"]));
        // Parent-of: Children section → me is parent of peer.
        assert!(file
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::ParentOf && e.participants == vec!["Me", "Down"]));
    }

    #[test]
    fn migrate_substitutes_dev_root_token() {
        let tmp = TempDir::new().unwrap();
        let (cfg, plan_dir) = scaffold_plan(tmp.path(), "Me", &["Peer"]);
        let projects_root = tmp.path().join("projects");

        // Token used in place of the dev_root prefix.
        let body = "## Siblings\n- {{DEV_ROOT}}/Peer — via token\n";
        write_related_plans_md(&plan_dir, body);
        // Expected dev_root is parent of the project dir (projects_root).
        assert_eq!(projects_root, projects_root); // sanity

        run_migrate_related_projects(
            &cfg,
            &plan_dir,
            &MigrateRelatedProjectsOptions::default(),
        )
        .unwrap();

        let file = load_or_empty(&cfg).unwrap();
        assert_eq!(file.edges.len(), 1);
        assert_eq!(file.edges[0].kind, EdgeKind::Sibling);
    }

    #[test]
    fn migrate_is_idempotent_on_second_run() {
        let tmp = TempDir::new().unwrap();
        let (cfg, plan_dir) = scaffold_plan(tmp.path(), "Me", &["Peer"]);
        let projects_root = tmp.path().join("projects");

        let body = format!(
            "## Siblings\n- {peer} — peer\n",
            peer = projects_root.join("Peer").display(),
        );
        write_related_plans_md(&plan_dir, &body);

        let first = run_migrate_related_projects(
            &cfg,
            &plan_dir,
            &MigrateRelatedProjectsOptions::default(),
        )
        .unwrap();
        assert_eq!(first.added.len(), 1);

        let second = run_migrate_related_projects(
            &cfg,
            &plan_dir,
            &MigrateRelatedProjectsOptions::default(),
        )
        .unwrap();
        assert!(second.added.is_empty());
        assert_eq!(second.skipped_existing.len(), 1);

        let file = load_or_empty(&cfg).unwrap();
        assert_eq!(file.edges.len(), 1);
    }

    #[test]
    fn migrate_auto_adds_unknown_peer_to_catalog() {
        let tmp = TempDir::new().unwrap();
        // Seed catalog with only `Me`; peer path exists on disk but is not catalogued.
        let (cfg, plan_dir) = scaffold_plan(tmp.path(), "Me", &[]);
        let projects_root = tmp.path().join("projects");
        let peer_path = projects_root.join("NewPeer");
        std::fs::create_dir_all(&peer_path).unwrap();

        let body = format!("## Siblings\n- {} — new\n", peer_path.display());
        write_related_plans_md(&plan_dir, &body);

        run_migrate_related_projects(
            &cfg,
            &plan_dir,
            &MigrateRelatedProjectsOptions::default(),
        )
        .unwrap();

        let catalog = projects::load_or_empty(&cfg).unwrap();
        assert!(catalog.find_by_name("NewPeer").is_some());
    }

    #[test]
    fn migrate_bails_on_name_collision() {
        let tmp = TempDir::new().unwrap();
        let (cfg, plan_dir) = scaffold_plan(tmp.path(), "Me", &[]);
        // Pre-seed the catalog with a different path under the same basename
        // the migration will try to use.
        let other = tmp.path().join("elsewhere").join("Peer");
        std::fs::create_dir_all(&other).unwrap();
        let mut catalog = projects::load_or_empty(&cfg).unwrap();
        projects::try_add_named(&mut catalog, "Peer", &other).unwrap();
        projects::save_atomic(&cfg, &catalog).unwrap();

        let projects_root = tmp.path().join("projects");
        let peer_path = projects_root.join("Peer");
        std::fs::create_dir_all(&peer_path).unwrap();

        let body = format!("## Siblings\n- {} — peer\n", peer_path.display());
        write_related_plans_md(&plan_dir, &body);

        let err = run_migrate_related_projects(
            &cfg,
            &plan_dir,
            &MigrateRelatedProjectsOptions::default(),
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("collision"));
        assert!(msg.contains("state projects add"));

        // Neither the target file nor the catalog should have changed.
        assert!(!cfg.join(RELATED_PROJECTS_FILE).exists());
    }

    #[test]
    fn migrate_dry_run_writes_nothing() {
        let tmp = TempDir::new().unwrap();
        let (cfg, plan_dir) = scaffold_plan(tmp.path(), "Me", &["Peer"]);
        let projects_root = tmp.path().join("projects");
        let body = format!(
            "## Siblings\n- {peer} — peer\n",
            peer = projects_root.join("Peer").display(),
        );
        write_related_plans_md(&plan_dir, &body);

        let opts = MigrateRelatedProjectsOptions {
            dry_run: true,
            delete_original: false,
        };
        let report = run_migrate_related_projects(&cfg, &plan_dir, &opts).unwrap();

        assert_eq!(report.added.len(), 1);
        assert!(!cfg.join(RELATED_PROJECTS_FILE).exists());
        assert!(plan_dir.join("related-plans.md").exists());
    }

    #[test]
    fn migrate_delete_original_removes_md_after_success() {
        let tmp = TempDir::new().unwrap();
        let (cfg, plan_dir) = scaffold_plan(tmp.path(), "Me", &["Peer"]);
        let projects_root = tmp.path().join("projects");
        let body = format!(
            "## Siblings\n- {peer} — peer\n",
            peer = projects_root.join("Peer").display(),
        );
        write_related_plans_md(&plan_dir, &body);

        let opts = MigrateRelatedProjectsOptions {
            dry_run: false,
            delete_original: true,
        };
        run_migrate_related_projects(&cfg, &plan_dir, &opts).unwrap();

        assert!(!plan_dir.join("related-plans.md").exists());
        assert!(cfg.join(RELATED_PROJECTS_FILE).exists());
    }

    #[test]
    fn migrate_errors_when_no_related_plans_md() {
        let tmp = TempDir::new().unwrap();
        let (cfg, plan_dir) = scaffold_plan(tmp.path(), "Me", &[]);

        let err = run_migrate_related_projects(
            &cfg,
            &plan_dir,
            &MigrateRelatedProjectsOptions::default(),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("no related-plans.md"));
    }

    #[test]
    fn migrate_merges_into_existing_related_projects_yaml() {
        let tmp = TempDir::new().unwrap();
        let (cfg, plan_dir) = scaffold_plan(tmp.path(), "Me", &["Peer", "Other"]);
        let projects_root = tmp.path().join("projects");

        // Pre-seed with an unrelated edge that the migration must preserve.
        let mut file = RelatedProjectsFile::default();
        file.add_edge(Edge::sibling("Me", "Other")).unwrap();
        save_atomic(&cfg, &file).unwrap();

        let body = format!(
            "## Siblings\n- {peer} — peer\n",
            peer = projects_root.join("Peer").display(),
        );
        write_related_plans_md(&plan_dir, &body);
        run_migrate_related_projects(
            &cfg,
            &plan_dir,
            &MigrateRelatedProjectsOptions::default(),
        )
        .unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 2);
        assert!(loaded
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Sibling && e.involves("Peer")));
        assert!(loaded
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Sibling && e.involves("Other")));
    }
}
