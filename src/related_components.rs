//! Host adapter for the component-ontology graph at
//! `<config_root>/related-components.yaml`.
//!
//! The on-disk types and validation rules live in the
//! `component-ontology` crate (atlas-contracts workspace) — that crate
//! is host-agnostic so it can serve Atlas, Ravel-Lite, or any future
//! consumer. Everything host-specific (the filename, the
//! `<config-root>` join, the per-context `repos.yaml` resolver, the CLI
//! verbs) lives here.
//!
//! Schema is v2: every edge carries `(kind, lifecycle, participants,
//! evidence_grade, evidence_fields, rationale)`. The loader rejects any
//! file whose `schema_version` is not 2 (enforced in
//! `component_ontology::yaml_io::load_or_default`). There is no in-memory
//! v1 → v2 upgrader — the file is a generated artifact, so
//! delete-and-regenerate is the supported upgrade path
//! (`docs/component-ontology.md` §12).

use std::path::Path;

use anyhow::{Context, Result};

use crate::bail_with;
use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;
use component_ontology::{self as ontology, Edge, EvidenceGrade, RelatedComponentsFile};
use crate::repos::{self, ReposRegistry};
use crate::state::filenames::RELATED_COMPONENTS_FILENAME;
use crate::state::targets::read_targets;

// Re-export the v2 ontology surface that the host needs to construct
// edges through this adapter — the binary crate (`main.rs`) and tests
// route through `crate::related_components::*` rather than touching
// `component_ontology` directly.
pub use component_ontology::{EdgeKind, LifecycleScope};

/// Filter set for `run_list`. An empty filter emits every edge; each
/// populated field narrows the match set by AND-composition.
pub struct ListFilter<'a> {
    pub plan: Option<&'a Path>,
    pub kind: Option<EdgeKind>,
    pub lifecycle: Option<LifecycleScope>,
}

/// Full-field request for `run_add_edge`. Participants are borrowed
/// because their lifetime is trivially shorter than the caller's stack
/// frame; the owned fields (`evidence_fields`, `rationale`) move into
/// the constructed `Edge`.
pub struct AddEdgeRequest<'a> {
    pub kind: EdgeKind,
    pub lifecycle: LifecycleScope,
    pub a: &'a str,
    pub b: &'a str,
    pub evidence_grade: EvidenceGrade,
    pub evidence_fields: Vec<String>,
    pub rationale: String,
}

pub fn load_or_empty(config_root: &Path) -> Result<RelatedComponentsFile> {
    ontology::load_or_default(&config_root.join(RELATED_COMPONENTS_FILENAME))
}

pub fn save_atomic(config_root: &Path, file: &RelatedComponentsFile) -> Result<()> {
    ontology::save_atomic(&config_root.join(RELATED_COMPONENTS_FILENAME), file)
}

/// Rewrites every participant reference in `related-components.yaml`
/// from `old` to `new`. No-op when the file is absent (a catalog
/// without any edge file is valid). Symmetric kinds are re-sorted
/// internally by the ontology layer. Currently unused after the
/// projects.yaml → repos.yaml cutover removed the rename verb; kept as
/// a building block for any future rename machinery on `repos.yaml`.
pub fn rename_component_in_edges(config_root: &Path, old: &str, new: &str) -> Result<()> {
    let path = config_root.join(RELATED_COMPONENTS_FILENAME);
    if !path.exists() {
        return Ok(());
    }
    let mut file = load_or_empty(config_root)?;
    if file.rename_component_in_edges(old, new) {
        save_atomic(config_root, &file)?;
    }
    Ok(())
}

/// Canonical read of a plan's `related-plans.md` prose, used by the
/// phase-loop entry points in `main.rs` and `multi_plan.rs` to seed
/// `PlanContext::related_plans` (the `{{RELATED_PLANS}}` macro).
/// Returns an empty string when the file is absent — graceful default.
pub fn read_related_plans_markdown(plan_dir: &Path) -> String {
    std::fs::read_to_string(plan_dir.join("related-plans.md")).unwrap_or_default()
}

// ---------- CLI handlers ----------

pub fn run_list(config_root: &Path, filter: &ListFilter<'_>) -> Result<()> {
    let file = load_or_empty(config_root)?;

    // Plan-derived component filter is resolved once from the plan's
    // `targets.yaml`; kind/lifecycle are direct value comparisons against
    // each edge.
    let plan_components = match filter.plan {
        None => None,
        Some(plan) => Some(resolve_plan_component_names(plan)?),
    };

    let filtered = RelatedComponentsFile {
        schema_version: file.schema_version,
        edges: file
            .edges
            .into_iter()
            .filter(|e| {
                plan_components
                    .as_deref()
                    .is_none_or(|names| names.iter().any(|n| e.involves(n)))
            })
            .filter(|e| filter.kind.is_none_or(|k| e.kind == k))
            .filter(|e| filter.lifecycle.is_none_or(|l| e.lifecycle == l))
            .collect(),
    };

    let yaml = serde_yaml::to_string(&filtered)
        .context("failed to serialise related-components to YAML")
        .with_code(ErrorCode::Internal)?;
    print!("{yaml}");
    Ok(())
}

/// Add an edge with the full ontology v2 field set supplied by the
/// caller. Validation happens inside `Edge::validate` via
/// `RelatedComponentsFile::add_edge` — non-empty rationale,
/// `evidence_fields` non-empty unless `evidence_grade=weak`, symmetric
/// kinds stored in sorted order, distinct participants.
pub fn run_add_edge(config_root: &Path, req: &AddEdgeRequest<'_>) -> Result<()> {
    let registry = repos::load_for_lookup(config_root)?;
    require_component_known(&registry, req.a)?;
    require_component_known(&registry, req.b)?;

    let participants = canonicalise_participants_for_kind(req.kind, req.a, req.b);
    let edge = Edge {
        kind: req.kind,
        lifecycle: req.lifecycle,
        participants,
        evidence_grade: req.evidence_grade,
        evidence_fields: req.evidence_fields.clone(),
        rationale: req.rationale.clone(),
    };

    let mut file = load_or_empty(config_root)?;
    let added = file.add_edge(edge)?;
    if !added {
        eprintln!(
            "edge already present (kind={}, lifecycle={}, {} / {}); no change.",
            req.kind.as_str(),
            req.lifecycle.as_str(),
            req.a,
            req.b
        );
        return Ok(());
    }
    save_atomic(config_root, &file)
}

/// Remove the unique edge matching `(kind, lifecycle, canonicalised
/// participants)`. Errors when the search finds nothing so scripted
/// cleanups don't silently no-op.
pub fn run_remove_edge(
    config_root: &Path,
    kind: EdgeKind,
    lifecycle: LifecycleScope,
    a: &str,
    b: &str,
) -> Result<()> {
    let mut file = load_or_empty(config_root)?;
    let want = canonicalise_participants_for_kind(kind, a, b);
    let before = file.edges.len();
    file.edges.retain(|e| {
        !(e.kind == kind && e.lifecycle == lifecycle && e.participants == want)
    });
    if file.edges.len() == before {
        bail_with!(
            ErrorCode::NotFound,
            "no matching edge to remove (kind={}, lifecycle={}, {} / {})",
            kind.as_str(),
            lifecycle.as_str(),
            a,
            b
        );
    }
    save_atomic(config_root, &file)
}

fn canonicalise_participants_for_kind(kind: EdgeKind, a: &str, b: &str) -> Vec<String> {
    let mut v = vec![a.to_string(), b.to_string()];
    if !kind.is_directed() {
        v.sort();
    }
    v
}

fn require_component_known(registry: &ReposRegistry, slug: &str) -> Result<()> {
    if registry.get(slug).is_none() {
        bail_with!(
            ErrorCode::NotFound,
            "component '{}' is not in the repo registry; register it with \
             `ravel-lite repo add {} --url <git-url> [--local-path <path>]`",
            slug,
            slug
        );
    }
    Ok(())
}

/// Identifier set used to filter edges by `--plan`. Reads
/// `<plan>/targets.yaml` and returns the union of every target row's
/// `repo_slug` and `component_id`, deduped, preserving target order.
///
/// Both halves are included because edge participants are opaque
/// strings (`Edge::participants: Vec<String>`): `discover` currently
/// emits repo-level slugs, but the architecture-next direction is for
/// participants to become component ids. Carrying both keeps the
/// filter correct across that evolution without a follow-up change.
///
/// A missing or empty `targets.yaml` returns an empty vec, which
/// causes the filter to match no edges — matching `work.md`'s
/// "empty output is fine" contract for a freshly-created plan
/// before anything is mounted.
fn resolve_plan_component_names(plan_dir: &Path) -> Result<Vec<String>> {
    let targets = read_targets(plan_dir)?;
    let mut names: Vec<String> = Vec::with_capacity(targets.targets.len() * 2);
    for t in &targets.targets {
        if !names.iter().any(|n| n == &t.repo_slug) {
            names.push(t.repo_slug.clone());
        }
        if !names.iter().any(|n| n == &t.component_id) {
            names.push(t.component_id.clone());
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn mk_catalog_with(config_root: &Path, names: &[&str]) {
        let mut registry = ReposRegistry::default();
        for name in names {
            let p = config_root.join(name);
            std::fs::create_dir_all(&p).unwrap();
            repos::try_add(&mut registry, name, "test-url", Some(&p)).unwrap();
        }
        repos::save_atomic(config_root, &registry).unwrap();
    }

    /// Test helper: build a weak-evidence edge with a fixed lifecycle so
    /// unit tests don't replicate the add-edge flag plumbing. Mirrors
    /// what the CLI would produce for `add-edge <kind> <lifecycle> a b
    /// --evidence-grade weak --rationale test`.
    fn weak_edge(kind: EdgeKind, lifecycle: LifecycleScope, a: &str, b: &str) -> Edge {
        Edge {
            kind,
            lifecycle,
            participants: canonicalise_participants_for_kind(kind, a, b),
            evidence_grade: EvidenceGrade::Weak,
            evidence_fields: Vec::new(),
            rationale: "test".into(),
        }
    }

    fn req_weak<'a>(
        kind: EdgeKind,
        lifecycle: LifecycleScope,
        a: &'a str,
        b: &'a str,
    ) -> AddEdgeRequest<'a> {
        AddEdgeRequest {
            kind,
            lifecycle,
            a,
            b,
            evidence_grade: EvidenceGrade::Weak,
            evidence_fields: Vec::new(),
            rationale: "test".into(),
        }
    }

    #[test]
    fn load_or_empty_returns_empty_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let file = load_or_empty(tmp.path()).unwrap();
        assert_eq!(file.schema_version, ontology::SCHEMA_VERSION);
        assert!(file.edges.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = TempDir::new().unwrap();
        let mut file = RelatedComponentsFile::default();
        file.add_edge(weak_edge(EdgeKind::Generates, LifecycleScope::Codegen, "Alpha", "Beta"))
            .unwrap();
        save_atomic(tmp.path(), &file).unwrap();
        let loaded = load_or_empty(tmp.path()).unwrap();
        assert_eq!(loaded, file);
    }

    #[test]
    fn rename_cascade_rewrites_participants() {
        let tmp = TempDir::new().unwrap();
        let mut file = RelatedComponentsFile::default();
        file.add_edge(weak_edge(EdgeKind::Generates, LifecycleScope::Codegen, "OldName", "Peer"))
            .unwrap();
        save_atomic(tmp.path(), &file).unwrap();

        rename_component_in_edges(tmp.path(), "OldName", "NewName").unwrap();

        let loaded = load_or_empty(tmp.path()).unwrap();
        assert_eq!(loaded.edges[0].participants, vec!["NewName", "Peer"]);
    }

    #[test]
    fn rename_cascade_is_noop_when_file_absent() {
        let tmp = TempDir::new().unwrap();
        rename_component_in_edges(tmp.path(), "Solo", "SoloRenamed").unwrap();
        assert!(!tmp.path().join(RELATED_COMPONENTS_FILENAME).exists());
    }

    #[test]
    fn add_edge_rejects_unknown_component() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["Known"]);

        let err = run_add_edge(
            &cfg,
            &req_weak(EdgeKind::Generates, LifecycleScope::Codegen, "Known", "Stranger"),
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Stranger"));
        assert!(msg.contains("repo add"));
        assert!(!cfg.join(RELATED_COMPONENTS_FILENAME).exists());
    }

    #[test]
    fn add_edge_persists_caller_supplied_v2_fields() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["A", "B"]);

        let req = AddEdgeRequest {
            kind: EdgeKind::Generates,
            lifecycle: LifecycleScope::Codegen,
            a: "A",
            b: "B",
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec!["A.produces_files".into(), "B.consumes_files".into()],
            rationale: "A emits schemas B consumes".into(),
        };
        run_add_edge(&cfg, &req).unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
        let edge = &loaded.edges[0];
        assert_eq!(edge.kind, EdgeKind::Generates);
        assert_eq!(edge.lifecycle, LifecycleScope::Codegen);
        assert_eq!(edge.evidence_grade, EvidenceGrade::Strong);
        assert_eq!(
            edge.evidence_fields,
            vec!["A.produces_files".to_string(), "B.consumes_files".to_string()]
        );
        assert_eq!(edge.rationale, "A emits schemas B consumes");
    }

    #[test]
    fn add_edge_rejects_strong_grade_without_evidence_fields() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["A", "B"]);

        let req = AddEdgeRequest {
            kind: EdgeKind::Generates,
            lifecycle: LifecycleScope::Codegen,
            a: "A",
            b: "B",
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: Vec::new(),
            rationale: "A emits schemas B consumes".into(),
        };
        let err = run_add_edge(&cfg, &req).unwrap_err();
        assert!(format!("{err:#}").contains("evidence_field"));
        assert!(!cfg.join(RELATED_COMPONENTS_FILENAME).exists());
    }

    #[test]
    fn add_edge_rejects_empty_rationale() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["A", "B"]);

        let req = AddEdgeRequest {
            kind: EdgeKind::Generates,
            lifecycle: LifecycleScope::Codegen,
            a: "A",
            b: "B",
            evidence_grade: EvidenceGrade::Weak,
            evidence_fields: Vec::new(),
            rationale: "   ".into(),
        };
        let err = run_add_edge(&cfg, &req).unwrap_err();
        assert!(format!("{err:#}").contains("rationale"));
    }

    #[test]
    fn add_edge_is_idempotent_on_directed_kind() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["A", "B"]);

        let req = req_weak(EdgeKind::Generates, LifecycleScope::Codegen, "A", "B");
        run_add_edge(&cfg, &req).unwrap();
        run_add_edge(&cfg, &req).unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
    }

    #[test]
    fn add_edge_accepts_same_pair_at_distinct_lifecycles() {
        // §3.5: one pair, one kind, two lifecycles → two edges. The CLI
        // must preserve this — `lifecycle` participates in the dedup key.
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["A", "B"]);

        run_add_edge(
            &cfg,
            &req_weak(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B"),
        )
        .unwrap();
        run_add_edge(
            &cfg,
            &req_weak(EdgeKind::DependsOn, LifecycleScope::Runtime, "A", "B"),
        )
        .unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 2);
    }

    #[test]
    fn add_edge_canonicalises_symmetric_participants() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["Alpha", "Beta"]);

        // Reverse order on a symmetric kind must still dedup with the
        // sorted form.
        run_add_edge(
            &cfg,
            &req_weak(EdgeKind::CoImplements, LifecycleScope::Design, "Beta", "Alpha"),
        )
        .unwrap();
        run_add_edge(
            &cfg,
            &req_weak(EdgeKind::CoImplements, LifecycleScope::Design, "Alpha", "Beta"),
        )
        .unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
        assert_eq!(
            loaded.edges[0].participants,
            vec!["Alpha".to_string(), "Beta".to_string()]
        );
    }

    #[test]
    fn remove_edge_errors_when_absent() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        let err = run_remove_edge(
            &cfg,
            EdgeKind::Generates,
            LifecycleScope::Codegen,
            "A",
            "B",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("no matching edge"));
    }

    #[test]
    fn remove_edge_matches_only_specified_lifecycle() {
        // Adding a `depends-on` at two lifecycles then removing one must
        // leave the other in place — the lifecycle is a required part
        // of the match key, not a wildcard.
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["A", "B"]);

        run_add_edge(
            &cfg,
            &req_weak(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B"),
        )
        .unwrap();
        run_add_edge(
            &cfg,
            &req_weak(EdgeKind::DependsOn, LifecycleScope::Runtime, "A", "B"),
        )
        .unwrap();

        run_remove_edge(
            &cfg,
            EdgeKind::DependsOn,
            LifecycleScope::Build,
            "A",
            "B",
        )
        .unwrap();

        let loaded = load_or_empty(&cfg).unwrap();
        assert_eq!(loaded.edges.len(), 1);
        assert_eq!(loaded.edges[0].lifecycle, LifecycleScope::Runtime);
    }

    #[test]
    fn remove_edge_works_on_symmetric_kind_regardless_of_order() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["Alpha", "Beta"]);

        run_add_edge(
            &cfg,
            &req_weak(EdgeKind::CoImplements, LifecycleScope::Design, "Alpha", "Beta"),
        )
        .unwrap();
        run_remove_edge(
            &cfg,
            EdgeKind::CoImplements,
            LifecycleScope::Design,
            "Beta",
            "Alpha",
        )
        .unwrap();

        assert!(load_or_empty(&cfg).unwrap().edges.is_empty());
    }

    #[test]
    fn resolve_plan_component_names_returns_empty_when_targets_yaml_absent() {
        // Fresh plan with nothing mounted: the filter must produce an
        // empty set so `--plan` matches no edges (matching work.md's
        // "empty output is fine" contract).
        let tmp = TempDir::new().unwrap();
        let names = resolve_plan_component_names(tmp.path()).unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn resolve_plan_component_names_collects_repo_slug_and_component_id() {
        // A target row contributes both halves of its `(repo_slug,
        // component_id)` ref — repo slugs match today's discover-emitted
        // participants, component ids match the architecture-next
        // direction. Including both keeps `--plan` filtering correct
        // across that evolution.
        use crate::state::targets::{write_targets, Target, TargetsFile, TARGETS_SCHEMA_VERSION};
        let tmp = TempDir::new().unwrap();
        let plan_dir = tmp.path();
        write_targets(
            plan_dir,
            &TargetsFile {
                schema_version: TARGETS_SCHEMA_VERSION,
                targets: vec![Target {
                    repo_slug: "ravel-lite".into(),
                    component_id: "phase-loop".into(),
                    working_root: ".worktrees/ravel-lite".into(),
                    branch: "ravel-lite/p/main".into(),
                    path_segments: vec!["src".into(), "phase_loop".into()],
                }],
            },
        )
        .unwrap();

        let names = resolve_plan_component_names(plan_dir).unwrap();
        assert_eq!(names, vec!["ravel-lite".to_string(), "phase-loop".to_string()]);
    }

    #[test]
    fn resolve_plan_component_names_dedups_when_repo_slug_equals_component_id() {
        // Root components conventionally use `component_id == repo_slug`
        // (per the targets.yaml examples in architecture-next). Both
        // halves resolve to the same string, so the result must hold one
        // entry, not two.
        use crate::state::targets::{write_targets, Target, TargetsFile, TARGETS_SCHEMA_VERSION};
        let tmp = TempDir::new().unwrap();
        let plan_dir = tmp.path();
        write_targets(
            plan_dir,
            &TargetsFile {
                schema_version: TARGETS_SCHEMA_VERSION,
                targets: vec![Target {
                    repo_slug: "ravel-lite".into(),
                    component_id: "ravel-lite".into(),
                    working_root: ".worktrees/ravel-lite".into(),
                    branch: "ravel-lite/p/main".into(),
                    path_segments: vec![String::new()],
                }],
            },
        )
        .unwrap();

        let names = resolve_plan_component_names(plan_dir).unwrap();
        assert_eq!(names, vec!["ravel-lite".to_string()]);
    }

    #[test]
    fn run_list_with_plan_filter_matches_edges_involving_any_target() {
        // Two targets across two repos; a single edge between them must
        // survive the `--plan` filter because both participants are in
        // the plan's target set.
        use crate::state::targets::{write_targets, Target, TargetsFile, TARGETS_SCHEMA_VERSION};
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        mk_catalog_with(&cfg, &["ravel-lite", "atlas"]);

        run_add_edge(
            &cfg,
            &req_weak(EdgeKind::DependsOn, LifecycleScope::Build, "ravel-lite", "atlas"),
        )
        .unwrap();

        let plan_dir = tmp.path().join("plan");
        std::fs::create_dir_all(&plan_dir).unwrap();
        write_targets(
            &plan_dir,
            &TargetsFile {
                schema_version: TARGETS_SCHEMA_VERSION,
                targets: vec![
                    Target {
                        repo_slug: "ravel-lite".into(),
                        component_id: "ravel-lite".into(),
                        working_root: ".worktrees/ravel-lite".into(),
                        branch: "ravel-lite/p/main".into(),
                        path_segments: vec![String::new()],
                    },
                    Target {
                        repo_slug: "atlas".into(),
                        component_id: "atlas".into(),
                        working_root: ".worktrees/atlas".into(),
                        branch: "ravel-lite/p/main".into(),
                        path_segments: vec![String::new()],
                    },
                ],
            },
        )
        .unwrap();

        let filter = ListFilter {
            plan: Some(&plan_dir),
            kind: None,
            lifecycle: None,
        };
        // Smoke test: this used to bail with "plan's project ... is not
        // registered as a repo's local_path" because the path
        // grandparent (`tmp`) was unregistered. The targets.yaml-based
        // resolver makes the filter independent of where the plan dir
        // sits on disk.
        run_list(&cfg, &filter).unwrap();
    }

    #[test]
    fn run_list_with_plan_filter_in_v2_layout_does_not_require_grandparent_repo() {
        // Regression: a v2 plan at `<context>/plans/<name>/` has a
        // grandparent (`<context>`) that is NOT a registered repo's
        // local_path, and that's expected — the resolver must read
        // `targets.yaml` instead. Empty targets.yaml is treated as
        // "no edges match", not as an error.
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        // A repo whose local_path is *not* the plan's grandparent.
        mk_catalog_with(&cfg, &["ravel-lite"]);

        let plan_dir = tmp.path().join("plans/some-plan");
        std::fs::create_dir_all(&plan_dir).unwrap();

        let filter = ListFilter {
            plan: Some(&plan_dir),
            kind: None,
            lifecycle: None,
        };
        run_list(&cfg, &filter).unwrap();
    }

    /// Migration M2 compat guard: a pre-M2 `related-components.yaml`
    /// (produced when the ontology types lived in `src/ontology/`) must
    /// parse and round-trip bit-identically through the
    /// `component-ontology` crate. Drift in the on-disk shape — added
    /// fields, field reordering, scalar style changes — would fail this
    /// byte-for-byte equality.
    #[test]
    fn pre_m2_related_components_yaml_round_trips_bit_identical() {
        let pre_m2_yaml = r#"schema_version: 2
edges:
- kind: depends-on
  lifecycle: build
  participants:
  - Atlas
  - Ravel-Lite
  evidence_grade: strong
  evidence_fields:
  - Atlas.surface.produces_files
  - Ravel-Lite.surface.consumes_files
  rationale: Ravel-Lite consumes the component-ontology crate produced by Atlas.
- kind: co-implements
  lifecycle: design
  participants:
  - Atlas
  - Ravel-Lite
  evidence_grade: medium
  evidence_fields:
  - Atlas.surface.purpose
  rationale: Shared component-relationship vocabulary.
"#;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(RELATED_COMPONENTS_FILENAME);
        std::fs::write(&path, pre_m2_yaml).unwrap();

        let loaded = load_or_empty(tmp.path()).unwrap();
        save_atomic(tmp.path(), &loaded).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();

        assert_eq!(
            after, pre_m2_yaml,
            "pre-M2 YAML must round-trip byte-for-byte through the component-ontology crate"
        );
    }
}
