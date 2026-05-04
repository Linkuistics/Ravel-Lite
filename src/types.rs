// src/types.rs
use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;

/// LLM phases — the agent subprocess runs these. The first four
/// variants are the cycle phases in order: triage opens, reflect closes.
/// Dream is no longer part of the in-cycle execution; it moves to
/// `ravel-lite curate`. The trailing `Migrate*` variants are off-cycle —
/// invoked one-shot by the `migrate-v1-v2` verb, never reached through
/// `phase_loop`. See `docs/superpowers/specs/2026-05-04-migrate-v1-v2-design.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LlmPhase {
    Triage,
    Work,
    AnalyseWork,
    Reflect,
    MigrateIntent,
    MigrateTargets,
    MigrateMemoryBackfill,
}

impl LlmPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Triage => "triage",
            Self::Work => "work",
            Self::AnalyseWork => "analyse-work",
            Self::Reflect => "reflect",
            Self::MigrateIntent => "migrate-intent",
            Self::MigrateTargets => "migrate-targets",
            Self::MigrateMemoryBackfill => "migrate-memory-backfill",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "triage" => Some(Self::Triage),
            "work" => Some(Self::Work),
            "analyse-work" => Some(Self::AnalyseWork),
            "reflect" => Some(Self::Reflect),
            "migrate-intent" => Some(Self::MigrateIntent),
            "migrate-targets" => Some(Self::MigrateTargets),
            "migrate-memory-backfill" => Some(Self::MigrateMemoryBackfill),
            _ => None,
        }
    }
}

impl fmt::Display for LlmPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Script phases — handled inline by the orchestrator (git commits).
/// The `GitCommit` prefix is load-bearing: it groups these as
/// commit-audit phases distinct from LLM phases and is visible in
/// phase.md / prompt text.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScriptPhase {
    GitCommitTriage,
    GitCommitWork,
    GitCommitAnalyseWork,
    GitCommitReflect,
}

impl ScriptPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GitCommitTriage => "git-commit-triage",
            Self::GitCommitWork => "git-commit-work",
            Self::GitCommitAnalyseWork => "git-commit-analyse-work",
            Self::GitCommitReflect => "git-commit-reflect",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "git-commit-triage" => Some(Self::GitCommitTriage),
            "git-commit-work" => Some(Self::GitCommitWork),
            "git-commit-analyse-work" => Some(Self::GitCommitAnalyseWork),
            "git-commit-reflect" => Some(Self::GitCommitReflect),
            _ => None,
        }
    }
}

impl fmt::Display for ScriptPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A phase is either an LLM phase or a script phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Llm(LlmPhase),
    Script(ScriptPhase),
}

impl Phase {
    pub fn parse(s: &str) -> Option<Self> {
        LlmPhase::parse(s)
            .map(Phase::Llm)
            .or_else(|| ScriptPhase::parse(s).map(Phase::Script))
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Phase::Llm(p) => write!(f, "{p}"),
            Phase::Script(p) => write!(f, "{p}"),
        }
    }
}

/// Context for a plan execution.
#[derive(Debug, Clone)]
pub struct PlanContext {
    pub plan_dir: String,
    pub project_dir: String,
    pub dev_root: String,
    pub related_plans: String,
    pub config_root: String,
}

/// Top-level shared config (config.yaml).
#[derive(Debug, Default, Clone, Deserialize)]
pub struct SharedConfig {
    pub agent: String,
    pub headroom: usize,
}

/// Per-agent config (agents/<name>/config.yaml).
#[derive(Debug, Default, Clone, Deserialize)]
pub struct AgentConfig {
    #[serde(default)]
    pub models: HashMap<String, String>,
    #[serde(default)]
    pub thinking: HashMap<String, String>,
    #[serde(default)]
    pub params: HashMap<String, HashMap<String, serde_yaml::Value>>,
    pub provider: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_llm_phases() {
        assert_eq!(Phase::parse("triage"), Some(Phase::Llm(LlmPhase::Triage)));
        assert_eq!(Phase::parse("work"), Some(Phase::Llm(LlmPhase::Work)));
        assert_eq!(Phase::parse("analyse-work"), Some(Phase::Llm(LlmPhase::AnalyseWork)));
        assert_eq!(Phase::parse("reflect"), Some(Phase::Llm(LlmPhase::Reflect)));
    }

    #[test]
    fn parse_dream_is_no_longer_in_cycle() {
        assert_eq!(Phase::parse("dream"), None);
        assert_eq!(Phase::parse("git-commit-dream"), None);
    }

    #[test]
    fn parse_script_phases() {
        assert_eq!(Phase::parse("git-commit-triage"), Some(Phase::Script(ScriptPhase::GitCommitTriage)));
        assert_eq!(Phase::parse("git-commit-work"), Some(Phase::Script(ScriptPhase::GitCommitWork)));
        assert_eq!(Phase::parse("git-commit-analyse-work"), Some(Phase::Script(ScriptPhase::GitCommitAnalyseWork)));
        assert_eq!(Phase::parse("git-commit-reflect"), Some(Phase::Script(ScriptPhase::GitCommitReflect)));
    }

    #[test]
    fn parse_invalid_phase() {
        assert_eq!(Phase::parse("invalid"), None);
        assert_eq!(Phase::parse(""), None);
    }

    #[test]
    fn parse_migrate_llm_phases_round_trip() {
        // The migrate-* phases are NOT cycle phases (see phase_loop.rs),
        // but they must round-trip as LlmPhase string forms because the
        // migrator invokes them via `agent.invoke_headless` using the
        // standard prompt-loading machinery.
        for s in ["migrate-intent", "migrate-targets", "migrate-memory-backfill"] {
            let parsed = LlmPhase::parse(s).unwrap_or_else(|| panic!("parse failed for {s:?}"));
            assert_eq!(parsed.as_str(), s);
            assert_eq!(parsed.to_string(), s);
        }
    }

    #[test]
    fn phase_display() {
        assert_eq!(Phase::Llm(LlmPhase::AnalyseWork).to_string(), "analyse-work");
        assert_eq!(Phase::Script(ScriptPhase::GitCommitReflect).to_string(), "git-commit-reflect");
    }
}
