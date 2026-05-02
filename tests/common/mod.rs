#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use ravel_lite::agent::Agent;
use ravel_lite::plan_kg::{BacklogStatus, MemoryStatus};
use ravel_lite::state::backlog::schema::{BacklogEntry, BacklogFile, BACKLOG_SCHEMA_VERSION};
use ravel_lite::state::backlog::write_backlog;
use ravel_lite::state::memory::schema::{MemoryEntry, MemoryFile, MEMORY_SCHEMA_VERSION};
use ravel_lite::state::memory::write_memory;
use ravel_lite::types::{LlmPhase, PlanContext};
use ravel_lite::ui::UISender;
use knowledge_graph::{Item, Justification, KindMarker};

/// Seed `memory.yaml` so that `dream`'s word counter sees exactly
/// `target_words` words of content (one entry, empty claim, rationale
/// of that many tokens). Lets tests focus on threshold behaviour without
/// wiring up the whole memory schema by hand.
pub fn write_memory_yaml_with_word_count(plan: &Path, target_words: usize) {
    let body = if target_words == 0 {
        String::new()
    } else {
        vec!["word"; target_words].join(" ")
    };
    let memory = MemoryFile {
        schema_version: MEMORY_SCHEMA_VERSION,
        items: vec![MemoryEntry {
            item: Item {
                id: "test-entry".into(),
                kind: KindMarker::new(),
                claim: String::new(),
                justifications: vec![Justification::Rationale { text: body }],
                status: MemoryStatus::Active,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "test".into(),
                authored_in: "test".into(),
            },
            attribution: None,
        }],
    };
    write_memory(plan, &memory).unwrap();
}

/// Seed `backlog.yaml` with a single item whose claim embeds `marker`.
/// The marker surfaces in the serialised YAML so tests can assert on
/// rendered survey output.
pub fn write_backlog_yaml_with_marker(plan: &Path, marker: &str) {
    let backlog = BacklogFile {
        schema_version: BACKLOG_SCHEMA_VERSION,
        items: vec![BacklogEntry {
            item: Item {
                id: "marker-task".into(),
                kind: KindMarker::new(),
                claim: marker.into(),
                justifications: vec![Justification::Rationale {
                    text: "Marker body.\n".into(),
                }],
                status: BacklogStatus::Active,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "test".into(),
                authored_in: "test".into(),
            },
            category: "maintenance".into(),
            blocked_reason: None,
            dependencies: vec![],
            results: None,
            handoff: None,
        }],
    };
    write_backlog(plan, &backlog).unwrap();
}

pub fn init_test_repo(root: &Path) {
    let run = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    run(&["commit", "-q", "--allow-empty", "-m", "init"]);
}

/// Minimal `Agent` impl that records which phases it was invoked for and
/// optionally rewrites `phase.md` to advance the cycle. Sufficient for
/// tests that exercise the phase-loop wiring without caring about
/// per-phase file writes.
pub struct MockAgent {
    pub calls: Arc<Mutex<Vec<LlmPhase>>>,
    pub next_phase_after: HashMap<LlmPhase, &'static str>,
    pub plan_dir: std::path::PathBuf,
}

#[async_trait]
impl Agent for MockAgent {
    async fn invoke_interactive(&self, _prompt: &str, _ctx: &PlanContext) -> anyhow::Result<()> {
        Ok(())
    }

    async fn invoke_headless(
        &self,
        _prompt: &str,
        _ctx: &PlanContext,
        phase: LlmPhase,
        _agent_id: &str,
        _tx: UISender,
    ) -> anyhow::Result<()> {
        self.calls.lock().unwrap().push(phase);
        if let Some(next) = self.next_phase_after.get(&phase) {
            fs::write(self.plan_dir.join("phase.md"), next)?;
        }
        Ok(())
    }

    async fn dispatch_subagent(
        &self,
        _prompt: &str,
        _target_plan: &str,
        _agent_id: &str,
        _tx: UISender,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn tokens(&self) -> HashMap<String, String> {
        // The shipped work prompt references `{{TOOL_READ}}`. In the
        // triage-first cycle the loop transitions through Work after
        // GitCommitTriage, so even tests that don't exercise the work
        // phase compose its prompt — supply a stand-in to keep
        // substitution successful.
        HashMap::from([("TOOL_READ".to_string(), "Read".to_string())])
    }
}
