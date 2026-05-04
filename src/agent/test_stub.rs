//! Test-only Agent impl that, when invoked headless for a Migrate*
//! phase, writes a pre-baked proposal YAML to the scratch location the
//! apply step expects.
//!
//! Used by `migrate_v1_v2` integration tests in `tests/`. Production
//! migrate runs use the real ClaudeCodeAgent / PiAgent. The module is
//! built unconditionally so the integration-test crate (which sees the
//! library without `cfg(test)`) can construct it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::agent::Agent;
use crate::migrate_v1_v2::proposals::{
    INTENT_PROPOSAL_FILENAME, MEMORY_PROPOSAL_FILENAME, TARGETS_PROPOSAL_FILENAME,
};
use crate::types::{LlmPhase, PlanContext};
use crate::ui::UISender;

/// Fixture mapping: which YAML to drop for which phase.
pub struct StubAgent {
    pub intent_proposal_yaml: String,
    pub targets_proposal_yaml: String,
    pub memory_proposal_yaml: String,
    pub plan_dir: PathBuf,
}

#[async_trait]
impl Agent for StubAgent {
    async fn invoke_interactive(&self, _prompt: &str, _ctx: &PlanContext) -> Result<()> {
        unreachable!("StubAgent never runs interactive phases")
    }

    async fn invoke_headless(
        &self,
        _prompt: &str,
        _ctx: &PlanContext,
        phase: LlmPhase,
        _agent_id: &str,
        _tx: UISender,
    ) -> Result<()> {
        let (filename, body) = match phase {
            LlmPhase::MigrateIntent => (INTENT_PROPOSAL_FILENAME, &self.intent_proposal_yaml),
            LlmPhase::MigrateTargets => (TARGETS_PROPOSAL_FILENAME, &self.targets_proposal_yaml),
            LlmPhase::MigrateMemoryBackfill => {
                (MEMORY_PROPOSAL_FILENAME, &self.memory_proposal_yaml)
            }
            other => unreachable!("StubAgent only handles Migrate* phases, got {other:?}"),
        };
        std::fs::write(self.plan_dir.join(filename), body)?;
        Ok(())
    }

    fn tokens(&self) -> HashMap<String, String> {
        HashMap::new()
    }
}

/// Convenience constructor.
pub fn stub(plan_dir: PathBuf, intent: &str, targets: &str, memory: &str) -> Arc<dyn Agent> {
    Arc::new(StubAgent {
        intent_proposal_yaml: intent.into(),
        targets_proposal_yaml: targets.into(),
        memory_proposal_yaml: memory.into(),
        plan_dir,
    })
}
