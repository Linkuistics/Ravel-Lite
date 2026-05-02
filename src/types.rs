// src/types.rs
use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;

/// LLM phases — the agent subprocess runs these. Variant order matches
/// the cycle position: triage opens, reflect closes. Dream is no longer
/// part of the in-cycle execution; it moves to `ravel-lite curate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LlmPhase {
    Triage,
    Work,
    AnalyseWork,
    Reflect,
}

impl LlmPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Triage => "triage",
            Self::Work => "work",
            Self::AnalyseWork => "analyse-work",
            Self::Reflect => "reflect",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "triage" => Some(Self::Triage),
            "work" => Some(Self::Work),
            "analyse-work" => Some(Self::AnalyseWork),
            "reflect" => Some(Self::Reflect),
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

/// A subagent dispatch entry from subagent-dispatch.yaml.
#[derive(Debug, Deserialize)]
pub struct SubagentDispatch {
    pub target: String,
    pub kind: String,
    pub summary: String,
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
    fn phase_display() {
        assert_eq!(Phase::Llm(LlmPhase::AnalyseWork).to_string(), "analyse-work");
        assert_eq!(Phase::Script(ScriptPhase::GitCommitReflect).to_string(), "git-commit-reflect");
    }
}
