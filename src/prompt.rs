// src/prompt.rs
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::Result;
use regex::Regex;

use crate::bail_with;
use crate::cli::ErrorCode;
use crate::init::require_embedded;
use crate::types::{LlmPhase, PlanContext};

/// Matches leftover `{{NAME}}` placeholders (ASCII letters, digits, `_`).
/// A failed substitution is almost always a typo in a phase prompt, so we
/// hard-error with the full set of names rather than log a warning — the
/// pi `{{MEMORY_DIR}}` bug slipped through precisely because a silent pass
/// reached the LLM unchanged.
fn unresolved_token_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{\{([A-Za-z0-9_]+)\}\}").unwrap())
}

/// Replace template tokens like {{PLAN}}, {{PROJECT}}, etc., then verify
/// no `{{NAME}}` placeholders remain. Returns `Err` listing every
/// unresolved token found, so drift in a phase prompt fails loudly at
/// compose time instead of silently reaching the LLM.
pub fn substitute_tokens(
    content: &str,
    ctx: &PlanContext,
    tokens: &HashMap<String, String>,
) -> Result<String> {
    let mut result = content.to_string();

    // Expand content macros first. These may inline authored content
    // (e.g. `related-plans.md`) that itself references path tokens; if
    // path tokens were substituted first, those inlined placeholders
    // would survive into the final output and trip the guard below.
    result = result.replace("{{RELATED_PLANS}}", &ctx.related_plans);
    for (key, value) in tokens {
        result = result.replace(&format!("{{{{{key}}}}}"), value);
    }

    // Then expand atomic path tokens, so any placeholders surfaced by
    // the macro expansions above get resolved in the same pass.
    result = result.replace("{{DEV_ROOT}}", &ctx.dev_root);
    result = result.replace("{{PROJECT}}", &ctx.project_dir);
    result = result.replace("{{PLAN}}", &ctx.plan_dir);
    result = result.replace("{{ORCHESTRATOR}}", &ctx.config_root);

    let unresolved: BTreeSet<&str> = unresolved_token_regex()
        .captures_iter(&result)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();

    if !unresolved.is_empty() {
        let names: Vec<String> = unresolved
            .iter()
            .map(|n| format!("{{{{{n}}}}}"))
            .collect();
        bail_with!(
            ErrorCode::InvalidInput,
            "Prompt contains unresolved token(s) after substitution: {}. \
             This usually indicates a typo in a phase prompt or a missing \
             agent-provided token.",
            names.join(", ")
        );
    }

    Ok(result)
}

/// Load the phase prompt from the embedded set. No disk read — the
/// shipped default is the only source of truth.
pub fn load_phase_file(phase: LlmPhase) -> Result<String> {
    match phase {
        // TODO(migrate-v1-v2 Phase 5): replace these placeholders with the
        // embedded `phases/migrate-*.md` prompt files. Phase 1 introduced
        // the `LlmPhase` variants; Phase 5 ships the prompts and registers
        // them in `EMBEDDED_FILES`. Until then `load_phase_file` would
        // hard-error trying to read a non-existent embedded entry, so
        // return an empty string and let the migrator orchestrator (also
        // not yet wired) be the only caller.
        LlmPhase::MigrateIntent | LlmPhase::MigrateTargets | LlmPhase::MigrateMemoryBackfill => {
            Ok(String::new())
        }
        _ => {
            let rel = format!("phases/{}.md", phase);
            Ok(require_embedded(&rel)?.to_string())
        }
    }
}

/// Load an optional plan-specific prompt override.
pub fn load_plan_override(plan_dir: &Path, phase: LlmPhase) -> Option<String> {
    let path = plan_dir.join(format!("prompt-{}.md", phase));
    fs::read_to_string(&path).ok()
}

/// Compose the full prompt for a phase. The composition layers, in
/// order:
///   1. The embedded phase prompt (`load_phase_file`).
///   2. The optional plan-level override at
///      `<plan_dir>/prompt-<phase>.md` (`load_plan_override`).
///   3. Any text registered via `ravel.append_prompt(phase, ...)` in
///      a `config.lua` layer, in registration order.
///
/// Each section is separated by a horizontal rule so the LLM sees
/// distinct blocks rather than a run-on prompt. Token substitution
/// runs over the concatenated whole, so path tokens (`{{PLAN}}`,
/// `{{PROJECT}}`, …) inside any layer resolve identically.
pub fn compose_prompt(
    phase: LlmPhase,
    ctx: &PlanContext,
    tokens: &HashMap<String, String>,
    appends: &[String],
) -> Result<String> {
    let base = load_phase_file(phase)?;
    let override_text = load_plan_override(Path::new(&ctx.plan_dir), phase);

    let mut prompt = base;
    if let Some(ov) = override_text {
        prompt.push_str("\n\n---\n\n");
        prompt.push_str(&ov);
    }
    for append in appends {
        prompt.push_str("\n\n---\n\n");
        prompt.push_str(append);
    }

    substitute_tokens(&prompt, ctx, tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> PlanContext {
        PlanContext {
            plan_dir: "/plans/my-plan".to_string(),
            project_dir: "/project".to_string(),
            dev_root: "/dev".to_string(),
            related_plans: "related stuff".to_string(),
            config_root: "/config".to_string(),
        }
    }

    #[test]
    fn substitutes_built_in_tokens() {
        let ctx = test_ctx();
        let result = substitute_tokens(
            "plan={{PLAN}} project={{PROJECT}}",
            &ctx,
            &HashMap::new(),
        ).unwrap();
        assert_eq!(result, "plan=/plans/my-plan project=/project");
    }

    #[test]
    fn substitutes_custom_tokens() {
        let ctx = test_ctx();
        let mut tokens = HashMap::new();
        tokens.insert("TOOL_READ".to_string(), "Read".to_string());
        let result = substitute_tokens("Use {{TOOL_READ}}", &ctx, &tokens).unwrap();
        assert_eq!(result, "Use Read");
    }

    #[test]
    fn fails_on_unresolved_token() {
        let ctx = test_ctx();
        let err = substitute_tokens("needs {{MEMORY_DIR}} here", &ctx, &HashMap::new())
            .expect_err("unresolved token should fail");
        let msg = err.to_string();
        assert!(msg.contains("{{MEMORY_DIR}}"), "message was: {msg}");
    }

    #[test]
    fn lists_all_unresolved_tokens_sorted_and_deduped() {
        let ctx = test_ctx();
        let err = substitute_tokens(
            "{{UNKNOWN_B}} and {{UNKNOWN_A}} and {{UNKNOWN_A}} again",
            &ctx,
            &HashMap::new(),
        )
        .expect_err("unresolved tokens should fail");
        let msg = err.to_string();
        // BTreeSet ordering: A before B, duplicates collapsed.
        let a = msg.find("{{UNKNOWN_A}}").expect("missing UNKNOWN_A");
        let b = msg.find("{{UNKNOWN_B}}").expect("missing UNKNOWN_B");
        assert!(a < b, "names should be sorted: {msg}");
        assert_eq!(msg.matches("{{UNKNOWN_A}}").count(), 1, "dedup failed: {msg}");
    }

    #[test]
    fn substitutes_path_tokens_inside_related_plans() {
        // Regression: `related-plans.md` is documented (create-plan.md)
        // to use `{{DEV_ROOT}}` etc. for path references. Those tokens
        // must still resolve after the file content is inlined via
        // `{{RELATED_PLANS}}`, or every plan with a related-plans.md
        // hits a fatal "unresolved token" at prompt-compose time.
        let ctx = PlanContext {
            plan_dir: "/plans/my-plan".to_string(),
            project_dir: "/project".to_string(),
            dev_root: "/dev".to_string(),
            related_plans: "- {{DEV_ROOT}}/Peer — sibling project".to_string(),
            config_root: "/config".to_string(),
        };
        let result = substitute_tokens(
            "Related plans:\n{{RELATED_PLANS}}",
            &ctx,
            &HashMap::new(),
        )
        .expect("path tokens inside related_plans should resolve");
        assert_eq!(result, "Related plans:\n- /dev/Peer — sibling project");
    }

    #[test]
    fn accepts_single_brace_sequences() {
        // `{foo}` (rust format-style) and `{{foo}}` lowercase with a colon
        // inside shouldn't false-positive. The regex requires
        // [A-Za-z0-9_] names so punctuation breaks the match.
        let ctx = test_ctx();
        let result = substitute_tokens("keep {x} and {{not-a-token}}", &ctx, &HashMap::new())
            .unwrap();
        assert_eq!(result, "keep {x} and {{not-a-token}}");
    }

    #[test]
    fn compose_prompt_uses_embedded_phase_and_applies_override() {
        // The base phase prompt comes from the embedded set; no file
        // is materialised for it. A plan-level `prompt-<phase>.md`
        // (here for triage) is appended after a horizontal rule, and
        // path tokens are substituted across the whole composition.
        let plan_dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            plan_dir.path().join("prompt-triage.md"),
            "Local override for {{PLAN}}",
        )
        .unwrap();

        let ctx = PlanContext {
            plan_dir: plan_dir.path().to_string_lossy().to_string(),
            project_dir: "/project".to_string(),
            dev_root: "/dev".to_string(),
            related_plans: "".to_string(),
            config_root: "/config".to_string(),
        };

        let result = compose_prompt(LlmPhase::Triage, &ctx, &HashMap::new(), &[]).unwrap();
        assert!(
            result.contains("Local override for"),
            "override must be appended"
        );
        assert!(
            result.contains(plan_dir.path().to_string_lossy().as_ref()),
            "{{{{PLAN}}}} token must be substituted in the override"
        );
    }

    #[test]
    fn compose_prompt_appends_lua_registered_text_in_order() {
        // Two `ravel.append_prompt` calls (passed through here as a
        // pre-collected slice) land after the embedded base in the
        // order they were registered. Each is fenced off with a
        // horizontal rule so the LLM can see the boundary.
        let plan_dir = tempfile::TempDir::new().unwrap();
        let ctx = PlanContext {
            plan_dir: plan_dir.path().to_string_lossy().to_string(),
            project_dir: "/project".to_string(),
            dev_root: "/dev".to_string(),
            related_plans: "".to_string(),
            config_root: "/config".to_string(),
        };
        let appends = vec![
            "First append for {{PLAN}}".to_string(),
            "Second append".to_string(),
        ];

        let result =
            compose_prompt(LlmPhase::Triage, &ctx, &HashMap::new(), &appends).unwrap();

        let first_pos = result.find("First append for").expect("first append present");
        let second_pos = result.find("Second append").expect("second append present");
        assert!(
            first_pos < second_pos,
            "appends must keep registration order; first should precede second"
        );
        assert!(
            result.contains(plan_dir.path().to_string_lossy().as_ref()),
            "tokens inside appends must be substituted"
        );
        // The horizontal-rule fence ensures distinct sections.
        assert!(
            result.matches("\n\n---\n\n").count() >= appends.len(),
            "each append should be fenced by a horizontal rule"
        );
    }
}
