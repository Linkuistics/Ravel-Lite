//! Datalog smoke rules over the knowledge-graph substrate, evaluated by `ascent`.
//!
//! Two illustrative rules from `docs/architecture-next.md` §Datalog-style
//! inferencing are implemented here as a smoke test of the engine wiring:
//!
//! - `orphaned(Item)` — a backlog item with no active justifying intent.
//! - `suspect(Entry)` — a memory entry whose code-anchor SHA has changed.
//!
//! The smoke rules are not the production rule set; consumers (ravel-lite's
//! plan KG, the catalog KG) will register their own ascent programs against
//! the same substrate. This module exists to prove the engine binding works.

use ascent::ascent;

ascent! {
    pub struct SmokeProgram;

    relation backlog_item(String);
    relation intent_active(String);
    relation serves_intent(String, String);
    relation has_active_intent(String);
    relation orphaned(String);

    has_active_intent(item.clone()) <--
        serves_intent(item, intent),
        intent_active(intent);

    orphaned(item.clone()) <--
        backlog_item(item),
        !has_active_intent(item);

    relation memory_entry(String);
    relation code_anchor(String, String, String);
    relation current_sha(String, String);
    relation suspect(String);

    suspect(entry.clone()) <--
        memory_entry(entry),
        code_anchor(entry, path, old_sha),
        current_sha(path, new_sha),
        if old_sha != new_sha;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orphaned_rule_finds_backlog_items_without_active_intent() {
        let mut p = SmokeProgram {
            backlog_item: vec![("t-1".into(),), ("t-2".into(),), ("t-3".into(),)],
            intent_active: vec![("i-1".into(),)],
            serves_intent: vec![
                ("t-1".into(), "i-1".into()),
                ("t-2".into(), "i-2".into()), // i-2 not active
                // t-3 has no serves_intent at all
            ],
            ..SmokeProgram::default()
        };
        p.run();

        let mut orphaned: Vec<String> = p.orphaned.into_iter().map(|(s,)| s).collect();
        orphaned.sort();
        assert_eq!(orphaned, vec!["t-2".to_string(), "t-3".to_string()]);
    }

    #[test]
    fn suspect_rule_finds_memory_entries_with_changed_sha() {
        let mut p = SmokeProgram {
            memory_entry: vec![("m-1".into(),), ("m-2".into(),), ("m-3".into(),)],
            code_anchor: vec![
                ("m-1".into(), "src/foo.rs".into(), "old-sha".into()),
                ("m-2".into(), "src/bar.rs".into(), "stable-sha".into()),
                ("m-3".into(), "src/baz.rs".into(), "old-baz".into()),
            ],
            current_sha: vec![
                ("src/foo.rs".into(), "new-sha".into()), // changed
                ("src/bar.rs".into(), "stable-sha".into()), // unchanged
                ("src/baz.rs".into(), "new-baz".into()), // changed
                // src/qux.rs absent → m without anchor not flagged
            ],
            ..SmokeProgram::default()
        };
        p.run();

        let mut suspect: Vec<String> = p.suspect.into_iter().map(|(s,)| s).collect();
        suspect.sort();
        assert_eq!(suspect, vec!["m-1".to_string(), "m-3".to_string()]);
    }

    #[test]
    fn empty_program_runs_clean() {
        let mut p = SmokeProgram::default();
        p.run();
        assert!(p.orphaned.is_empty());
        assert!(p.suspect.is_empty());
    }
}
