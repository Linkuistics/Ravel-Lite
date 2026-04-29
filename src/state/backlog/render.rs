//! Markdown-table rendering for `state backlog list --format markdown`.
//!
//! The canonical human-readable backlog view. Fixed three columns
//! (`title | status | deps`); one section per group; deterministic
//! ordering (status rank, then dependency depth, then insertion order).
//! Moving presentation into Rust removes per-cycle variability from
//! phase prompts that previously asked the LLM to render a summary.
//!
//! Ids are intentionally not shown: they bloat the table width — the
//! widest column was always `id`, and the bloat caused some downstream
//! TUI consumers to fall back from box-drawn tables to a per-row list
//! layout, breaking visual consistency across sections. Phases that
//! need ids derive them from titles deterministically (the
//! `allocate_id` mapping is stable) or call `--format yaml`/`--format
//! json`, both of which still emit `id`.

use std::collections::{HashMap, HashSet};

use knowledge_graph::ItemStatus;

use super::schema::{BacklogEntry, BacklogFile};
use crate::plan_kg::BacklogStatus;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum GroupBy {
    Category,
    Status,
}

impl GroupBy {
    pub fn parse(input: &str) -> Option<GroupBy> {
        match input {
            "category" => Some(GroupBy::Category),
            "status" => Some(GroupBy::Status),
            _ => None,
        }
    }
}

pub fn render_markdown(backlog: &BacklogFile, group_by: GroupBy) -> String {
    if backlog.items.is_empty() {
        return "_(no tasks)_\n".to_string();
    }

    let depth = compute_depth_ranks(&backlog.items);
    let insertion_order: HashMap<&str, usize> = backlog
        .items
        .iter()
        .enumerate()
        .map(|(i, e)| (e.item.id.as_str(), i))
        .collect();

    let mut groups: Vec<(String, Vec<&BacklogEntry>)> = match group_by {
        GroupBy::Category => group_by_category(&backlog.items),
        GroupBy::Status => group_by_status(&backlog.items),
    };

    for (_, section_entries) in groups.iter_mut() {
        section_entries.sort_by_key(|e| {
            (
                status_rank(e.item.status),
                *depth.get(e.item.id.as_str()).unwrap_or(&usize::MAX),
                *insertion_order.get(e.item.id.as_str()).unwrap_or(&usize::MAX),
            )
        });
    }

    let mut out = String::new();
    for (i, (section, section_entries)) in groups.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("## {section}\n\n"));
        out.push_str("| title | status | deps |\n");
        out.push_str("|---|---|---|\n");
        for entry in section_entries {
            out.push_str(&format!(
                "| {title} | {status} | {deps} |\n",
                title = escape_cell(&entry.item.claim),
                status = entry.item.status.as_str(),
                deps = render_deps(&entry.dependencies),
            ));
        }
    }
    out
}

fn group_by_category(entries: &[BacklogEntry]) -> Vec<(String, Vec<&BacklogEntry>)> {
    let mut by_cat: HashMap<String, Vec<&BacklogEntry>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for entry in entries {
        let key = entry.category.clone();
        if !by_cat.contains_key(&key) {
            order.push(key.clone());
        }
        by_cat.entry(key).or_default().push(entry);
    }
    order.sort_by_key(|a| a.to_lowercase());
    order
        .into_iter()
        .map(|cat| {
            let entries = by_cat.remove(&cat).unwrap_or_default();
            (cat, entries)
        })
        .collect()
}

fn group_by_status(entries: &[BacklogEntry]) -> Vec<(String, Vec<&BacklogEntry>)> {
    // Pinned section order matches the legacy layout intent: actionable
    // (active) first, then blocked, then terminal states (done,
    // defeated, superseded) so the operator's eye lands on what's open.
    let statuses = [
        BacklogStatus::Active,
        BacklogStatus::Blocked,
        BacklogStatus::Done,
        BacklogStatus::Defeated,
        BacklogStatus::Superseded,
    ];
    let mut sections: Vec<(String, Vec<&BacklogEntry>)> = statuses
        .iter()
        .map(|s| (s.as_str().to_string(), Vec::new()))
        .collect();
    for entry in entries {
        let idx = statuses
            .iter()
            .position(|s| *s == entry.item.status)
            .expect("BacklogStatus is exhaustive");
        sections[idx].1.push(entry);
    }
    sections.retain(|(_, ts)| !ts.is_empty());
    sections
}

/// Assign each item a depth in the dependency DAG: 0 for items whose
/// structured deps all live outside the backlog (or whose list is
/// empty), then `1 + max(dep_depth)` for everyone else. Cycle
/// participants — which `set-dependencies` forbids but a hand-edit
/// could still introduce — get `usize::MAX` so they sort to the end
/// rather than the algorithm looping forever.
fn compute_depth_ranks(entries: &[BacklogEntry]) -> HashMap<String, usize> {
    let ids_in_backlog: HashSet<&str> = entries.iter().map(|e| e.item.id.as_str()).collect();
    let mut depth: HashMap<String, usize> = HashMap::new();
    let mut remaining: Vec<&BacklogEntry> = entries.iter().collect();
    loop {
        let before = remaining.len();
        remaining.retain(|entry| {
            let ready = entry.dependencies.iter().all(|dep| {
                !ids_in_backlog.contains(dep.as_str()) || depth.contains_key(dep)
            });
            if !ready {
                return true;
            }
            let my_depth = entry
                .dependencies
                .iter()
                .filter_map(|dep| depth.get(dep).copied())
                .max()
                .map(|d| d + 1)
                .unwrap_or(0);
            depth.insert(entry.item.id.clone(), my_depth);
            false
        });
        if remaining.len() == before {
            break;
        }
    }
    for entry in remaining {
        depth.insert(entry.item.id.clone(), usize::MAX);
    }
    depth
}

fn status_rank(status: BacklogStatus) -> u8 {
    match status {
        BacklogStatus::Active => 0,
        BacklogStatus::Blocked => 1,
        BacklogStatus::Done => 2,
        BacklogStatus::Defeated => 3,
        BacklogStatus::Superseded => 4,
    }
}

fn render_deps(deps: &[String]) -> String {
    if deps.is_empty() {
        "—".to_string()
    } else {
        deps.iter()
            .map(|d| escape_cell(d))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn escape_cell(s: &str) -> String {
    s.replace('\n', " ").replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::backlog::schema::BACKLOG_SCHEMA_VERSION;
    use knowledge_graph::{Item, Justification, KindMarker};

    fn entry(
        id: &str,
        claim: &str,
        category: &str,
        status: BacklogStatus,
        deps: &[&str],
    ) -> BacklogEntry {
        BacklogEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: claim.into(),
                justifications: vec![Justification::Rationale {
                    text: "Body.\n".into(),
                }],
                status,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "test".into(),
                authored_in: "test".into(),
            },
            category: category.into(),
            blocked_reason: if status == BacklogStatus::Blocked {
                Some("upstream".into())
            } else {
                None
            },
            dependencies: deps.iter().map(|s| (*s).into()).collect(),
            results: if status == BacklogStatus::Done {
                Some("Done.\n".into())
            } else {
                None
            },
            handoff: None,
        }
    }

    fn file(items: Vec<BacklogEntry>) -> BacklogFile {
        BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items,
        }
    }

    #[test]
    fn empty_backlog_renders_a_placeholder_and_no_empty_tables() {
        let out = render_markdown(&BacklogFile::default(), GroupBy::Category);
        assert_eq!(out, "_(no tasks)_\n");
        assert!(!out.contains("|---"), "must not emit table scaffolding for empty input");
    }

    #[test]
    fn group_by_status_orders_sections_active_blocked_done() {
        let backlog = file(vec![
            entry("d", "D", "core", BacklogStatus::Done, &[]),
            entry("b", "B", "core", BacklogStatus::Blocked, &[]),
            entry("a", "A", "core", BacklogStatus::Active, &[]),
        ]);
        let out = render_markdown(&backlog, GroupBy::Status);

        let i_active = out.find("## active").expect("section missing");
        let i_blocked = out.find("## blocked").expect("section missing");
        let i_done = out.find("## done").expect("section missing");
        assert!(i_active < i_blocked, "active must come before blocked");
        assert!(i_blocked < i_done, "blocked must come before done");
    }

    #[test]
    fn within_a_group_rows_order_by_status_rank_first() {
        let backlog = file(vec![
            entry("d", "D", "core", BacklogStatus::Done, &[]),
            entry("b", "B", "core", BacklogStatus::Blocked, &[]),
            entry("a", "A", "core", BacklogStatus::Active, &[]),
        ]);
        let out = render_markdown(&backlog, GroupBy::Category);

        let i_a = out.find("| A |").expect("row A missing");
        let i_b = out.find("| B |").expect("row B missing");
        let i_d = out.find("| D |").expect("row D missing");
        assert!(i_a < i_b, "active row must come before blocked");
        assert!(i_b < i_d, "blocked row must come before done");
    }

    #[test]
    fn rows_break_status_ties_by_dependency_depth_roots_first() {
        let backlog = file(vec![
            entry("leaf", "Leaf", "core", BacklogStatus::Active, &["mid"]),
            entry("mid", "Mid", "core", BacklogStatus::Active, &["root"]),
            entry("root", "Root", "core", BacklogStatus::Active, &[]),
        ]);
        let out = render_markdown(&backlog, GroupBy::Category);

        let i_root = out.find("| Root |").expect("row Root missing");
        let i_mid = out.find("| Mid |").expect("row Mid missing");
        let i_leaf = out.find("| Leaf |").expect("row Leaf missing");
        assert!(i_root < i_mid, "root must render before mid: got\n{out}");
        assert!(i_mid < i_leaf, "mid must render before leaf: got\n{out}");
    }

    #[test]
    fn group_by_category_emits_one_section_per_category_alphabetically() {
        let backlog = file(vec![
            entry("a", "A", "infra", BacklogStatus::Active, &[]),
            entry("b", "B", "core", BacklogStatus::Active, &[]),
            entry("c", "C", "docs", BacklogStatus::Active, &[]),
        ]);
        let out = render_markdown(&backlog, GroupBy::Category);
        let i_core = out.find("## core").expect("core section missing");
        let i_docs = out.find("## docs").expect("docs section missing");
        let i_infra = out.find("## infra").expect("infra section missing");
        assert!(i_core < i_docs, "categories must sort alphabetically: got\n{out}");
        assert!(i_docs < i_infra, "categories must sort alphabetically: got\n{out}");
    }

    #[test]
    fn dependency_cell_shows_em_dash_when_no_deps_and_comma_list_otherwise() {
        let backlog = file(vec![
            entry("root", "Root", "core", BacklogStatus::Done, &[]),
            entry("other", "Other", "core", BacklogStatus::Done, &[]),
            entry("leaf", "Leaf", "core", BacklogStatus::Active, &["root", "other"]),
        ]);
        let out = render_markdown(&backlog, GroupBy::Category);
        assert!(out.contains("| Root | done | — |"), "no-dep row must show em dash:\n{out}");
        assert!(
            out.contains("| Leaf | active | root, other |"),
            "multi-dep cell must comma-join in declared order:\n{out}"
        );
    }

    #[test]
    fn cell_contents_escape_pipe_characters_and_collapse_newlines() {
        let backlog = file(vec![entry(
            "weird-id",
            "Title with | pipe\nand newline",
            "core",
            BacklogStatus::Active,
            &[],
        )]);
        let out = render_markdown(&backlog, GroupBy::Category);

        let rows: Vec<&str> = out.lines().filter(|l| l.starts_with("| ")).collect();
        assert_eq!(rows.len(), 2, "expected 1 header + 1 data row, got {rows:?}");

        assert!(
            out.contains("Title with \\| pipe and newline"),
            "pipe must be backslash-escaped and newline replaced with space:\n{out}"
        );
    }

    #[test]
    fn cycle_participants_sort_to_the_end_without_looping() {
        let backlog = file(vec![
            entry("a", "A", "core", BacklogStatus::Active, &["b"]),
            entry("b", "B", "core", BacklogStatus::Active, &["c"]),
            entry("c", "C", "core", BacklogStatus::Active, &["a"]),
            entry("z", "Z", "core", BacklogStatus::Active, &[]),
        ]);
        let out = render_markdown(&backlog, GroupBy::Category);
        let i_z = out.find("| Z |").expect("row Z missing");
        let i_a = out.find("| A |").expect("row A missing");
        assert!(i_z < i_a, "non-cycle root must precede cycle participants:\n{out}");
    }

    #[test]
    fn group_by_parse_rejects_unknown_values() {
        assert_eq!(GroupBy::parse("category"), Some(GroupBy::Category));
        assert_eq!(GroupBy::parse("status"), Some(GroupBy::Status));
        assert_eq!(GroupBy::parse("priority"), None);
        assert_eq!(GroupBy::parse(""), None);
    }
}
