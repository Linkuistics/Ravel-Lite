//! Drift guard: every `ravel-lite <chain...>` invocation referenced in
//! any embedded prompt file (phase prompts, `create-plan.md`, the survey
//! and discover prompts, pi system/memory prompts, etc.) must resolve
//! against the live CLI surface.
//!
//! Existing drift guards cover registration
//! (`every_file_under_defaults_is_registered_in_embedded_files`) and unresolved
//! `{{tokens}}` (`shipped_pi_prompts_have_no_dangling_tokens`). Neither catches a
//! prompt that references a renamed or typo'd verb — clap surfaces the
//! mismatch only at agent invocation time, which is too late.
//!
//! Strategy: walk every entry returned by `init::embedded_entries_with_prefix("")`,
//! parse all `ravel-lite <chain...>` invocations into kebab-case verb chains,
//! shell out to `<bin> <chain> --help`, and fail the test on `unrecognized
//! subcommand` in stderr.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use ravel_lite::init::embedded_entries_with_prefix;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ravel-lite"))
}

fn phases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("defaults")
        .join("phases")
}

const MAX_CHAIN_DEPTH: usize = 3;

fn is_kebab_case(s: &str) -> bool {
    !s.is_empty()
        && s.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.ends_with('-')
}

/// Strip surrounding punctuation (backticks, parens, commas, periods) from a
/// whitespace-delimited token without touching internal characters.
fn strip_token_padding(token: &str) -> &str {
    token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-')
}

/// Extract every `ravel-lite <chain...>` reference from `body`. The chain
/// terminates at the first non-kebab token (e.g. `{{PLAN}}`, `<id>`, `--flag`,
/// or a quoted argument), so the result is the verb path only.
fn extract_verb_chains(body: &str) -> Vec<Vec<String>> {
    let tokens: Vec<&str> = body.split_whitespace().collect();
    let mut chains = Vec::new();
    let mut idx = 0;
    while idx < tokens.len() {
        if strip_token_padding(tokens[idx]) != "ravel-lite" {
            idx += 1;
            continue;
        }
        idx += 1;
        let mut chain = Vec::new();
        while idx < tokens.len() && chain.len() < MAX_CHAIN_DEPTH {
            let stripped = strip_token_padding(tokens[idx]);
            if !is_kebab_case(stripped) {
                break;
            }
            chain.push(stripped.to_string());
            idx += 1;
        }
        if !chain.is_empty() {
            chains.push(chain);
        }
    }
    chains
}

#[test]
fn phase_prompts_reference_only_live_cli_verbs() {
    let mut chains: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut attribution: BTreeMap<Vec<String>, BTreeSet<String>> = BTreeMap::new();

    for (rel, body) in embedded_entries_with_prefix("") {
        if !body.contains("ravel-lite ") {
            continue;
        }
        for chain in extract_verb_chains(body) {
            attribution
                .entry(chain.clone())
                .or_default()
                .insert(rel.to_string());
            chains.insert(chain);
        }
    }

    assert!(
        !chains.is_empty(),
        "no `ravel-lite ...` invocations parsed from embedded prompts; \
         the parser or the prompts have changed shape"
    );

    let mut offences: Vec<String> = Vec::new();
    for chain in &chains {
        let output = Command::new(bin())
            .args(chain)
            .arg("--help")
            .output()
            .expect("failed to spawn ravel-lite");
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("unrecognized subcommand") {
            let sources: Vec<&str> = attribution
                .get(chain)
                .into_iter()
                .flatten()
                .map(|s| s.as_str())
                .collect();
            offences.push(format!(
                "ravel-lite {chain} → {first_line} (referenced by: {sources})",
                chain = chain.join(" "),
                first_line = stderr.lines().next().unwrap_or("").trim(),
                sources = sources.join(", ")
            ));
        }
    }

    assert!(
        offences.is_empty(),
        "embedded prompts reference stale `ravel-lite` verbs:\n  {}\n\n\
         A verb has been renamed or removed from the binary, but a prompt \
         still references the old name. Either restore the verb or update \
         the prompt.",
        offences.join("\n  ")
    );
}

#[test]
fn triage_first_cycle_verbs_are_referenced_in_phase_prompts() {
    // Anchor test for the verbs introduced by
    // `phase-prompt-updates-for-triage-first-cycle-shape`. If a future change
    // removes their prompt references, the broader resolution test would
    // silently drop them from coverage; this test fails loud instead.
    let phases = phases_dir();
    let mut all_text = String::new();
    for entry in fs::read_dir(&phases).expect("readable phases dir").flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        all_text.push_str(&fs::read_to_string(&path).expect("readable phase prompt"));
        all_text.push('\n');
    }

    let required = [
        "state focus-objections list",
        "state focus-objections add-wrong-target",
        "state focus-objections add-skip-item",
        "state focus-objections add-premature",
        "state this-cycle-focus show",
        "state this-cycle-focus set",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|verb| !all_text.contains(verb))
        .collect();
    assert!(
        missing.is_empty(),
        "phase prompts no longer reference triage-first cycle verbs: {missing:?}.\n\
         Restore the prompts that invoke these verbs, or update this anchor \
         when intentionally retiring one."
    );
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn extracts_simple_chain() {
        let body = "Run `ravel-lite state backlog list {{PLAN}}` first.";
        let chains = extract_verb_chains(body);
        assert_eq!(chains, vec![vec!["state", "backlog", "list"]]);
    }

    #[test]
    fn placeholder_terminates_chain() {
        let body = "ravel-lite state set-phase {{PLAN}} git-commit-triage";
        let chains = extract_verb_chains(body);
        assert_eq!(chains, vec![vec!["state", "set-phase"]]);
    }

    #[test]
    fn extracts_three_token_chain() {
        let body = "use ravel-lite state focus-objections add-wrong-target {{PLAN}} --reasoning";
        let chains = extract_verb_chains(body);
        assert_eq!(
            chains,
            vec![vec!["state", "focus-objections", "add-wrong-target"]]
        );
    }

    #[test]
    fn caps_chain_depth_at_three() {
        let body = "ravel-lite a b c d e f";
        let chains = extract_verb_chains(body);
        assert_eq!(chains, vec![vec!["a", "b", "c"]]);
    }

    #[test]
    fn flag_terminates_chain() {
        let body = "ravel-lite state backlog list --has-handoff";
        let chains = extract_verb_chains(body);
        assert_eq!(chains, vec![vec!["state", "backlog", "list"]]);
    }

    #[test]
    fn ignores_non_kebab_token_after_ravel_lite() {
        let body = "ravel-lite Tutorial section";
        let chains = extract_verb_chains(body);
        // `Tutorial` is not kebab-case (capital T), so chain stops empty.
        assert!(chains.is_empty(), "got: {chains:?}");
    }

    #[test]
    fn finds_multiple_invocations() {
        let body = "
            First `ravel-lite state backlog list {{PLAN}}`.
            Then `ravel-lite state memory list {{PLAN}}`.
        ";
        let chains = extract_verb_chains(body);
        assert_eq!(
            chains,
            vec![
                vec!["state", "backlog", "list"],
                vec!["state", "memory", "list"],
            ]
        );
    }

    #[test]
    fn line_continuation_does_not_corrupt_chain() {
        let body = "       ravel-lite state phase-summary render {{PLAN}} --phase triage \\\n           --baseline foo";
        let chains = extract_verb_chains(body);
        assert_eq!(chains, vec![vec!["state", "phase-summary", "render"]]);
    }
}
