//! Integration tests for the v2 path-shape gate wired into the cycle-shaped
//! verbs of the `ravel-lite` binary.

use std::process::Command;

#[test]
fn run_against_v1_path_errors_with_migrate_hint() {
    // `run` now takes plan NAMES, not paths — but a v1-style path arg
    // is the natural mistake users make when transitioning from v1.
    // The path-shaped-arg rejection in `resolve_plan_name` detects the
    // `LLM_STATE/` substring and points the user at `migrate-v1-v2`.
    let exe = env!("CARGO_BIN_EXE_ravel-lite");
    let output = Command::new(exe)
        .args(["run", "LLM_STATE/core"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "expected non-zero exit; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("v1 plan path") && stderr.contains("migrate-v1-v2"),
        "stderr was:\n{stderr}"
    );
}

#[test]
fn set_phase_against_v1_path_errors_with_migrate_hint() {
    let tmp = tempfile::TempDir::new().unwrap();
    let plan = tmp.path().join("project/LLM_STATE/core");
    std::fs::create_dir_all(&plan).unwrap();
    std::fs::write(plan.join("phase.md"), "triage\n").unwrap();

    let exe = env!("CARGO_BIN_EXE_ravel-lite");
    let output = Command::new(exe)
        .args(["state", "set-phase", plan.to_str().unwrap(), "work"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("v1 layout") && stderr.contains("migrate-v1-v2"),
        "stderr was:\n{stderr}"
    );
}
