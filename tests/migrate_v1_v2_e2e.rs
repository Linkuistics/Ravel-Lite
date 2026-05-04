//! End-to-end test of the migrate-v1-v2 orchestrator using the
//! StubAgent test seam. Exercises the full flow: validate → copy →
//! 3 stub agent invocations → 3 apply steps.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use ravel_lite::agent::test_stub::stub;
use ravel_lite::migrate_v1_v2::run_migrate_v1_v2;

fn run_git(cwd: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git failed to spawn");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn setup(tmp: &Path) -> (PathBuf, PathBuf) {
    // Source repo: real git repo with Atlas index.
    let source = tmp.join("MyProj");
    fs::create_dir_all(&source).unwrap();
    run_git(&source, &["init", "--initial-branch=main"]);
    fs::write(source.join("README"), "x").unwrap();
    run_git(
        &source,
        &["-c", "user.email=t@t", "-c", "user.name=t", "add", "."],
    );
    run_git(
        &source,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-m",
            "init",
        ],
    );
    fs::create_dir_all(source.join(".atlas")).unwrap();
    fs::write(
        source.join(".atlas/components.yaml"),
        "schema_version: 1\n\
         root: source\n\
         generated_at: 2026-04-01T00:00:00Z\n\
         cache_fingerprints:\n  ontology_sha: ''\n  model_id: ''\n  backend_version: ''\n\
         components:\n  - id: core\n    kind: library\n    evidence_grade: strong\n    rationale: test fixture\n",
    )
    .unwrap();

    // V1 plan inside the source repo.
    let plan = source.join("LLM_STATE/core");
    fs::create_dir_all(&plan).unwrap();
    fs::write(plan.join("phase.md"), "Pursue X.\n").unwrap();
    // Real v1 wire shape: `tasks:` / `title:` / `description:` for
    // backlog, `entries:` / `title:` / `body:` for memory. The
    // transform step reshapes these into v2 before apply_intent runs.
    fs::write(
        plan.join("backlog.yaml"),
        "tasks:\n- id: t-001\n  title: T1\n  category: x\n  status: not_started\n  description: ''\n",
    )
    .unwrap();
    fs::write(
        plan.join("memory.yaml"),
        "entries:\n- id: m-001\n  title: M1\n  body: ''\n",
    )
    .unwrap();

    // Config dir.
    let context = tmp.join("ctx");
    fs::create_dir_all(context.join("plans")).unwrap();
    fs::write(
        context.join("repos.yaml"),
        format!(
            "schema_version: 1\nrepos:\n  myproj:\n    url: git@example:myproj.git\n    local_path: {}\n",
            source.display()
        ),
    )
    .unwrap();

    (plan, context)
}

#[tokio::test]
async fn end_to_end_migrate_with_stub_agent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (old_plan, context) = setup(tmp.path());
    let new_plan_dir = context.join("plans/myproj-core");

    let intent_yaml = "intents:\n\
- id: i-001\n  kind: intent\n  claim: Pursue X\n  justifications:\n  - kind: rationale\n    text: From phase.md\n  status: active\n  supersedes: []\n  authored_at: '2026-04-01T00:00:00Z'\n  authored_in: 'migrate-intent'\n\
item_attributions:\n- item_id: t-001\n  serves: i-001\n";
    let targets_yaml = "targets:\n- component_id: core\n";
    let memory_yaml = "attributions:\n- entry_id: m-001\n  attribution: myproj:core\n";

    let agent = stub(new_plan_dir.clone(), intent_yaml, targets_yaml, memory_yaml);

    run_migrate_v1_v2(agent, &old_plan, "myproj-core", &context, true)
        .await
        .unwrap();

    // Files copied + written.
    assert!(new_plan_dir.join("phase.md").is_file());
    assert!(new_plan_dir.join("intents.yaml").is_file());
    assert!(new_plan_dir.join("backlog.yaml").is_file());
    assert!(new_plan_dir.join("memory.yaml").is_file());
    assert!(new_plan_dir.join("targets.yaml").is_file());
    assert!(new_plan_dir.join(".worktrees/myproj").is_dir());

    // Scratch files cleaned up.
    assert!(!new_plan_dir.join("migrate-intent-proposal.yaml").exists());
    assert!(!new_plan_dir.join("migrate-targets-proposal.yaml").exists());
    assert!(!new_plan_dir.join("migrate-memory-proposal.yaml").exists());

    // Backlog item picked up the serves-intent justification.
    let backlog = ravel_lite::state::backlog::yaml_io::read_backlog(&new_plan_dir).unwrap();
    let t1 = backlog.items.iter().find(|e| e.item.id == "t-001").unwrap();
    assert!(matches!(
        t1.item.justifications.last(),
        Some(knowledge_graph::Justification::ServesIntent { intent_id }) if intent_id == "i-001"
    ));

    // Memory entry picked up attribution.
    let memory = ravel_lite::state::memory::yaml_io::read_memory(&new_plan_dir).unwrap();
    let m1 = memory.items.iter().find(|e| e.item.id == "m-001").unwrap();
    assert_eq!(m1.attribution.as_deref(), Some("myproj:core"));
}
