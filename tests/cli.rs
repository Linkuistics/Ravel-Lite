use std::fs;
use std::io::Cursor;
use std::process::Command;

use tempfile::TempDir;

#[test]
fn state_set_phase_rejects_invalid_phase_via_binary() {
    // V2-shaped plan dir so the v2_gate accepts it; the test then
    // exercises the inner phase-name validation.
    let tmp = TempDir::new().unwrap();
    let context = tmp.path().join("ctx");
    fs::create_dir_all(context.join("plans/p")).unwrap();
    fs::write(context.join("repos.yaml"), "schema_version: 1\nrepos: {}\n").unwrap();
    let plan = context.join("plans/p");
    fs::write(plan.join("phase.md"), "work").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "set-phase"])
        .arg(&plan)
        .arg("analyze-work") // American spelling — invalid
        .output()
        .expect("binary must spawn");
    assert!(!out.status.success(), "invalid phase must exit non-zero");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Invalid phase"),
        "stderr missing diagnostic: {stderr}"
    );
    // On-disk phase.md unchanged.
    assert_eq!(
        fs::read_to_string(plan.join("phase.md")).unwrap().trim(),
        "work"
    );
}

/// 5c CLI validation: with two or more plan_dirs, `--survey-state` is
/// required. The pre-flight check must fire BEFORE the binary tries to
/// load configs or spawn agents — proving the validation lives in the
/// CLI dispatch layer where multi-plan vs single-plan branches.
#[test]
fn run_multi_plan_requires_survey_state_flag() {
    // V2-shaped plan dirs so the v2_gate accepts them; the test then
    // exercises the multi-plan --survey-state validation.
    let tmp = TempDir::new().unwrap();
    let config_root = tmp.path().join("cfg");
    ravel_lite::init::run_init(&config_root, false).unwrap();
    let plan_a = config_root.join("plans/plan-a");
    let plan_b = config_root.join("plans/plan-b");
    fs::create_dir_all(&plan_a).unwrap();
    fs::create_dir_all(&plan_b).unwrap();
    fs::write(plan_a.join("phase.md"), "work").unwrap();
    fs::write(plan_b.join("phase.md"), "work").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["run", "--config"])
        .arg(&config_root)
        .arg(&plan_a)
        .arg(&plan_b)
        .output()
        .expect("binary must spawn");
    assert!(
        !out.status.success(),
        "multi-plan run without --survey-state must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--survey-state"),
        "stderr should name the missing flag: {stderr}"
    );
    assert!(
        stderr.contains("required"),
        "stderr should explain why: {stderr}"
    );
}

/// 5c CLI validation: with exactly one plan_dir, `--survey-state` has
/// no meaning and is rejected. Catches accidental misuse where a user
/// adds the flag to a single-plan invocation expecting it to be
/// ignored — silently ignoring would mask their mistake.
#[test]
fn run_single_plan_rejects_survey_state_flag() {
    // V2-shaped plan dir so the v2_gate accepts it; the test then
    // exercises single-plan rejection of --survey-state.
    let tmp = TempDir::new().unwrap();
    let config_root = tmp.path().join("cfg");
    ravel_lite::init::run_init(&config_root, false).unwrap();
    let plan = config_root.join("plans/solo");
    fs::create_dir_all(&plan).unwrap();
    fs::write(plan.join("phase.md"), "work").unwrap();

    let state_path = tmp.path().join("survey.yaml");

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["run", "--config"])
        .arg(&config_root)
        .arg("--survey-state")
        .arg(&state_path)
        .arg(&plan)
        .output()
        .expect("binary must spawn");
    assert!(
        !out.status.success(),
        "single-plan run with --survey-state must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--survey-state"),
        "stderr should name the offending flag: {stderr}"
    );
    assert!(
        stderr.contains("multiple plan"),
        "stderr should explain when the flag is meaningful: {stderr}"
    );
    // The state file must not have been written — the validation
    // fires before any survey work happens.
    assert!(
        !state_path.exists(),
        "validation must short-circuit before touching the state file"
    );
}

/// 5c integration: the multi-plan run loop relies on round-tripping
/// the survey YAML through `--survey-state`. This test bypasses the
/// claude spawn (which we can't mock cheaply) and verifies the pieces
/// of the loop that DO live in Rust:
///   - `build_plan_dir_map` correctly indexes discovered plans by the
///     same `project/plan` key the survey rows carry.
///   - `select_plan_from_response` resolves a recommendation key back
///     to the right plan directory.
///   - The persisted YAML round-trips (parse → emit → parse) via the
///     same path the next cycle's incremental survey will follow.
#[test]
fn multi_plan_round_trip_preserves_selection_mapping() {
    let tmp = TempDir::new().unwrap();
    let project = tmp.path().join("Proj");
    fs::create_dir_all(project.join(".git")).unwrap();
    let plan_a = project.join("LLM_STATE").join("plan-a");
    let plan_b = project.join("LLM_STATE").join("plan-b");
    fs::create_dir_all(&plan_a).unwrap();
    fs::create_dir_all(&plan_b).unwrap();
    fs::write(plan_a.join("phase.md"), "work").unwrap();
    fs::write(plan_b.join("phase.md"), "triage").unwrap();

    let map =
        ravel_lite::multi_plan::build_plan_dir_map(&[plan_a.clone(), plan_b.clone()]).unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map.get("Proj/plan-a"), Some(&plan_a));
    assert_eq!(map.get("Proj/plan-b"), Some(&plan_b));

    // Simulate what compute_survey_response returns + persists.
    let response_yaml = "schema_version: 1\n\
        plans:\n  \
        - project: Proj\n    plan: plan-a\n    phase: work\n    unblocked: 0\n    blocked: 0\n    done: 0\n    received: 0\n  \
        - project: Proj\n    plan: plan-b\n    phase: triage\n    unblocked: 0\n    blocked: 0\n    done: 0\n    received: 0\n\
        recommended_invocation_order:\n  \
        - plan: Proj/plan-b\n    order: 1\n    rationale: Triage first to unblock A\n  \
        - plan: Proj/plan-a\n    order: 2\n    rationale: Then resume work\n";

    // First cycle: parse, emit (what run_multi_plan writes to --survey-state),
    // then parse again (what the next cycle's --prior load would do).
    let response = ravel_lite::survey::parse_survey_response(response_yaml).unwrap();
    let emitted = ravel_lite::survey::emit_survey_yaml(&response).unwrap();
    let reparsed = ravel_lite::survey::parse_survey_response(&emitted).unwrap();
    assert_eq!(
        response, reparsed,
        "round-trip through --survey-state must preserve the response"
    );

    // User picks the top-ranked plan (#1 = Proj/plan-b).
    let mut output = Vec::new();
    let mut input = Cursor::new("1\n");
    let picked =
        ravel_lite::multi_plan::select_plan_from_response(&reparsed, &map, &mut output, &mut input)
            .unwrap();
    assert_eq!(
        picked,
        Some(plan_b.clone()),
        "ordinal 1 (top-ranked Proj/plan-b) must resolve back to plan_b's PathBuf"
    );

    // User picks the second-ranked plan (#2 = Proj/plan-a).
    let mut output2 = Vec::new();
    let mut input2 = Cursor::new("2\n");
    let picked2 = ravel_lite::multi_plan::select_plan_from_response(
        &reparsed,
        &map,
        &mut output2,
        &mut input2,
    )
    .unwrap();
    assert_eq!(picked2, Some(plan_a));
}

/// `ravel-lite init --config <path>` is path-optional in the v2 CLI
/// (no positional `dir` arg). Spawning the binary end-to-end proves
/// the clap definition resolves the path through
/// `resolve_config_dir_for_init`, which permits a non-existent target.
/// The resulting context must hold every v2 layout artefact the
/// downstream phases lean on (`repos.yaml`, `findings.yaml`, the four
/// subdirs, `.git/`, and the `config.lua` stub).
#[test]
fn init_via_binary_scaffolds_v2_layout_with_config_flag() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("ctx");
    assert!(!target.exists(), "precondition: target dir must not exist");

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["init", "--config"])
        .arg(&target)
        .output()
        .expect("binary must spawn");
    assert!(
        out.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(target.is_dir(), "context dir must exist after init");
    assert!(target.join("config.lua").exists());
    assert!(target.join("repos.yaml").exists());
    assert!(target.join("findings.yaml").exists());
    for sub in ["agents", "phases", "fixed-memory", "plans"] {
        assert!(target.join(sub).is_dir(), "missing subdir: {sub}");
    }
    assert!(target.join(".git").is_dir(), "context must own its git history");
}

/// Top-level `--help` must surface the source repo and the docs site
/// so a user reading help can find both without remembering the URLs.
#[test]
fn top_level_help_shows_repo_and_website_urls() {
    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .arg("--help")
        .output()
        .expect("binary must spawn");
    assert!(out.status.success(), "--help must exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("https://github.com/Linkuistics/Ravel-Lite"),
        "--help must contain the repo URL: {stdout}"
    );
    assert!(
        stdout.contains("https://www.linkuistics.com/projects/ravel-lite/"),
        "--help must contain the website URL: {stdout}"
    );
}

/// Invalid `--format` value is a typed `InvalidInput` error: exit 2
/// (usage error) per `ExitCategory::UsageError`, not the catch-all
/// exit 1. Asserts the `bail_with!` plumbing reaches the renderer.
#[test]
fn invalid_format_exits_with_usage_error_code() {
    let tmp = TempDir::new().unwrap();
    let plan = tmp.path();
    fs::write(plan.join("phase.md"), "work").unwrap();
    fs::write(plan.join("backlog.yaml"), "schema_version: 1\nitems: []\n").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "backlog", "list"])
        .arg(plan)
        .args(["--format", "xml"])
        .output()
        .expect("binary must spawn");
    assert_eq!(
        out.status.code(),
        Some(2),
        "invalid --format must exit 2 (UsageError); got {:?}, stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Unknown `fixed-memory show <slug>` is `NotFound`: exit 3 per
/// `ExitCategory::NotFound`. Asserts the `bail_with!` at the
/// `dispatch_fixed_memory` site sets the right code.
#[test]
fn unknown_fixed_memory_slug_exits_with_not_found_code() {
    let tmp = TempDir::new().unwrap();
    let config_root = tmp.path().join("cfg");
    ravel_lite::init::run_init(&config_root, false).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["fixed-memory", "show"])
        .arg("--config")
        .arg(&config_root)
        .arg("no-such-slug")
        .output()
        .expect("binary must spawn");
    assert_eq!(
        out.status.code(),
        Some(3),
        "unknown fixed-memory slug must exit 3 (NotFound); got {:?}, stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Every leaf verb's `--help` must include at least two concrete
/// invocation examples — the highest-leverage agent-friendliness
/// affordance per `defaults/fixed-memory/cli-tool-design.md` §2. This
/// test spot-checks one representative leaf per verb family; the
/// `after_help` const block in `main.rs` is the source of truth.
#[test]
fn leaf_verb_help_carries_invocation_examples() {
    let cases = [
        &["init", "--help"][..],
        &["run", "--help"][..],
        &["capabilities", "--help"][..],
        &["repo", "list", "--help"][..],
        &["repo", "add", "--help"][..],
        &["fixed-memory", "show", "--help"][..],
        &["state", "backlog", "list", "--help"][..],
        &["state", "backlog", "set-status", "--help"][..],
        &["state", "memory", "add", "--help"][..],
        &["state", "intents", "set-status", "--help"][..],
        &["state", "this-cycle-focus", "set", "--help"][..],
        &["state", "session-log", "append", "--help"][..],
        &["findings", "add", "--help"][..],
        &["plan", "list-items", "--help"][..],
    ];
    for argv in cases {
        let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
            .args(argv)
            .output()
            .expect("binary must spawn");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("Examples:"),
            "{:?} --help missing `Examples:` block:\n{stdout}",
            argv
        );
        assert!(
            stdout.contains("ravel-lite "),
            "{:?} --help examples must show `ravel-lite ...` invocations:\n{stdout}",
            argv
        );
    }
}

/// Inner-verb error sites must surface typed exit codes — e.g. asking for a
/// memory entry by an unknown id is `NotFound` (exit 3), not the catch-all
/// `Internal` (exit 1). Asserts the `bail_with!`/`CodedError` plumbing
/// reaches the renderer for the per-kind `state <kind>` modules tagged in
/// the inner-verb sweep.
#[test]
fn state_memory_show_unknown_id_exits_with_not_found_code() {
    let tmp = TempDir::new().unwrap();
    let plan = tmp.path();
    fs::write(plan.join("phase.md"), "work").unwrap();
    fs::write(plan.join("memory.yaml"), "schema_version: 1\nitems: []\n").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "memory", "show"])
        .arg(plan)
        .arg("no-such-id")
        .output()
        .expect("binary must spawn");
    assert_eq!(
        out.status.code(),
        Some(3),
        "unknown memory id must exit 3 (NotFound); got {:?}, stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Backlog: setting `--status blocked` without `--reason` is `InvalidInput`
/// (exit 2). Asserts the inner-verb tagging in `state/backlog/verbs.rs`.
#[test]
fn state_backlog_set_status_blocked_without_reason_exits_with_usage_error() {
    let tmp = TempDir::new().unwrap();
    let plan = tmp.path();
    fs::write(plan.join("phase.md"), "work").unwrap();
    // Seed one item so the verb gets past the read.
    let body = "schema_version: 1\nitems:\n- schema_version: 1\n  id: alpha\n  kind: backlog-item\n  claim: Alpha\n  justifications:\n  - kind: rationale\n    text: |\n      x\n  status: active\n  supersedes: []\n  authored_at: '2026-01-01T00:00:00Z'\n  authored_in: test\n  category: infra\n  dependencies: []\n";
    fs::write(plan.join("backlog.yaml"), body).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "backlog", "set-status"])
        .arg(plan)
        .args(["alpha", "blocked"])
        .output()
        .expect("binary must spawn");
    assert_eq!(
        out.status.code(),
        Some(2),
        "blocked-without-reason must exit 2 (UsageError); got {:?}, stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Schema-version mismatch on a YAML file is `Conflict` (exit 5 per
/// `ExitCategory::Conflict`). Asserts the typed code on the schema_version
/// guard in every `state <kind>` yaml_io module.
#[test]
fn state_backlog_schema_version_mismatch_exits_with_conflict_code() {
    let tmp = TempDir::new().unwrap();
    let plan = tmp.path();
    fs::write(plan.join("phase.md"), "work").unwrap();
    fs::write(plan.join("backlog.yaml"), "schema_version: 99\nitems: []\n").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "backlog", "list"])
        .arg(plan)
        .output()
        .expect("binary must spawn");
    assert_eq!(
        out.status.code(),
        Some(5),
        "schema_version mismatch must exit 5 (Conflict); got {:?}, stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Repo registry: `repo remove <unknown>` is `NotFound` (exit 3). Asserts
/// the typed tagging in `repos.rs`.
#[test]
fn repo_remove_unknown_slug_exits_with_not_found_code() {
    let tmp = TempDir::new().unwrap();
    let config_root = tmp.path().join("cfg");
    ravel_lite::init::run_init(&config_root, false).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["repo", "remove", "--config"])
        .arg(&config_root)
        .arg("no-such-repo")
        .output()
        .expect("binary must spawn");
    assert_eq!(
        out.status.code(),
        Some(3),
        "unknown repo slug must exit 3 (NotFound); got {:?}, stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// `visible_alias` annotations from `defaults/fixed-memory/cli-tool-design.md`
/// §5: every canonical verb that has a common synonym (`ls` for `list`,
/// `rm`/`delete` for `remove`, `get`/`cat` for `show`, `create` for `add`)
/// is reachable via the alias. This test exercises one alias per category
/// against the binary; success is the alias dispatching to the same code
/// path as the canonical verb.
#[test]
fn visible_aliases_dispatch_to_canonical_verbs() {
    let tmp = TempDir::new().unwrap();
    let plan = tmp.path();
    fs::write(plan.join("phase.md"), "work").unwrap();
    fs::write(plan.join("backlog.yaml"), "schema_version: 1\nitems: []\n").unwrap();
    fs::write(plan.join("memory.yaml"), "schema_version: 1\nitems: []\n").unwrap();

    let run = |argv: &[&str]| -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
            .args(argv)
            .output()
            .expect("binary must spawn")
    };

    // `ls` alias for `list` — dispatches to the same handler.
    let out = run(&[
        "state",
        "backlog",
        "ls",
        plan.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "`backlog ls` must dispatch like `list`; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    // `create` alias for `add` — append a backlog item via the alias.
    let out = run(&[
        "state",
        "backlog",
        "create",
        plan.to_str().unwrap(),
        "--title",
        "Alias Test Task",
        "--category",
        "infra",
        "--description",
        "test body",
    ]);
    assert!(
        out.status.success(),
        "`backlog create` must dispatch like `add`; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    // Recover the generated id from `list` so the next assertions are
    // independent of the slugifier.
    let listing = run(&["state", "backlog", "list", plan.to_str().unwrap()]);
    assert!(listing.status.success());
    let listing_yaml = String::from_utf8_lossy(&listing.stdout);
    let id_line = listing_yaml
        .lines()
        .find(|l| l.trim_start().starts_with("- id:"))
        .expect("listing must contain an id line");
    let id = id_line.split_once("id:").unwrap().1.trim();

    // `get` and `cat` aliases for `show` — both must produce the same
    // output as the canonical `show` invocation.
    let canonical = run(&["state", "backlog", "show", plan.to_str().unwrap(), id]);
    let via_get = run(&["state", "backlog", "get", plan.to_str().unwrap(), id]);
    let via_cat = run(&["state", "backlog", "cat", plan.to_str().unwrap(), id]);
    assert!(canonical.status.success() && via_get.status.success() && via_cat.status.success());
    assert_eq!(canonical.stdout, via_get.stdout, "`get` must match `show`");
    assert_eq!(canonical.stdout, via_cat.stdout, "`cat` must match `show`");

    // `rm` alias for `delete` (backlog uses `Delete` as the canonical).
    let out = run(&["state", "backlog", "rm", plan.to_str().unwrap(), id]);
    assert!(
        out.status.success(),
        "`backlog rm` must dispatch like `delete`; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    // Repo registry: `delete` alias for `remove` (Repo uses `Remove` as
    // the canonical, with `rm` and `delete` as aliases).
    let config_root = tmp.path().join("cfg");
    ravel_lite::init::run_init(&config_root, false).unwrap();
    let cfg_str = config_root.to_str().unwrap();
    let out = run(&[
        "repo", "add", "--config", cfg_str, "alias-repo", "--url", "https://example.com/r.git",
    ]);
    assert!(out.status.success(), "repo add must succeed");
    let out = run(&["repo", "delete", "--config", cfg_str, "alias-repo"]);
    assert!(
        out.status.success(),
        "`repo delete` must dispatch like `remove`; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    // Help output must surface the aliases so agents discover them.
    let help = run(&["state", "backlog", "--help"]);
    assert!(help.status.success());
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(help_text.contains("ls"), "backlog --help must list `ls` alias: {help_text}");
    assert!(help_text.contains("create"), "backlog --help must list `create` alias: {help_text}");
}

/// JSON-mode error envelope must carry the typed `code` field
/// (`INVALID_INPUT`) — agents branch on the wire-form code without
/// parsing the prose message.
#[test]
fn invalid_format_in_json_mode_emits_typed_code_envelope() {
    let tmp = TempDir::new().unwrap();
    let plan = tmp.path();
    fs::write(plan.join("phase.md"), "work").unwrap();
    fs::write(plan.join("backlog.yaml"), "schema_version: 1\nitems: []\n").unwrap();

    // `--format json` selects JSON mode; `--status bogus` then fails
    // with the tagged `InvalidInput` error. Together they exercise the
    // JSON envelope rendering path with a typed code.
    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "backlog", "list"])
        .arg(plan)
        .args(["--format", "json", "--status", "bogus-status"])
        .output()
        .expect("binary must spawn");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("\"code\""),
        "stderr must be the JSON envelope: {stderr}"
    );
    assert!(
        stderr.contains("INVALID_INPUT"),
        "envelope must carry the typed code: {stderr}"
    );
}

/// `state related-components discover` against an empty repo registry
/// must surface the typed `NotFound` code (exit 3) — proving the
/// discover module's tagged-error pipeline is wired through to the
/// renderer end-to-end. The exit-category mapping is the contract
/// agents branch on.
#[test]
fn discover_with_empty_registry_exits_with_not_found_code() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    fs::create_dir_all(&cfg).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "related-components", "discover", "--config"])
        .arg(&cfg)
        .output()
        .expect("binary must spawn");
    assert_eq!(
        out.status.code(),
        Some(3),
        "empty registry must exit 3 (NotFound); got {:?}, stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
}

// `state migrate` (.md → .yaml converter) was removed in the v1→v2
// cutover; every plan in every LLM_STATE dir is already YAML-shaped.
// The companion test for the typed-error survival through that verb is
// retired with the verb itself.
