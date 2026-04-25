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
//! need ids (for `set-status`, `set-results`, etc.) derive them from
//! titles deterministically (the `allocate_id` mapping is stable) or
//! call `--format yaml`/`--format json`, both of which still emit `id`.

use std::collections::{HashMap, HashSet};

use super::schema::{BacklogFile, Status, Task};

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
    if backlog.tasks.is_empty() {
        return "_(no tasks)_\n".to_string();
    }

    let depth = compute_depth_ranks(&backlog.tasks);
    let insertion_order: HashMap<&str, usize> = backlog
        .tasks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();

    let mut groups: Vec<(String, Vec<&Task>)> = match group_by {
        GroupBy::Category => group_by_category(&backlog.tasks),
        GroupBy::Status => group_by_status(&backlog.tasks),
    };

    for (_, section_tasks) in groups.iter_mut() {
        section_tasks.sort_by_key(|t| {
            (
                status_rank(t.status),
                *depth.get(t.id.as_str()).unwrap_or(&usize::MAX),
                *insertion_order.get(t.id.as_str()).unwrap_or(&usize::MAX),
            )
        });
    }

    let mut out = String::new();
    for (i, (section, section_tasks)) in groups.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("## {section}\n\n"));
        out.push_str("| title | status | deps |\n");
        out.push_str("|---|---|---|\n");
        for task in section_tasks {
            out.push_str(&format!(
                "| {title} | {status} | {deps} |\n",
                title = escape_cell(&task.title),
                status = status_to_cli_str(task.status),
                deps = render_deps(&task.dependencies),
            ));
        }
    }
    out
}

fn group_by_category(tasks: &[Task]) -> Vec<(String, Vec<&Task>)> {
    let mut by_cat: HashMap<String, Vec<&Task>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for task in tasks {
        let key = task.category.clone();
        if !by_cat.contains_key(&key) {
            order.push(key.clone());
        }
        by_cat.entry(key).or_default().push(task);
    }
    order.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    order
        .into_iter()
        .map(|cat| {
            let tasks = by_cat.remove(&cat).unwrap_or_default();
            (cat, tasks)
        })
        .collect()
}

fn group_by_status(tasks: &[Task]) -> Vec<(String, Vec<&Task>)> {
    let statuses = [Status::NotStarted, Status::InProgress, Status::Blocked, Status::Done];
    let mut sections: Vec<(String, Vec<&Task>)> = statuses
        .iter()
        .map(|s| (status_to_cli_str(*s).to_string(), Vec::new()))
        .collect();
    for task in tasks {
        let idx = statuses.iter().position(|s| *s == task.status).unwrap();
        sections[idx].1.push(task);
    }
    sections.retain(|(_, ts)| !ts.is_empty());
    sections
}

/// Assign each task a depth in the dependency DAG: 0 for tasks whose
/// structured deps all live outside the backlog (or whose list is
/// empty), then `1 + max(dep_depth)` for everyone else. Cycle
/// participants — which `set-dependencies` forbids but a hand-edit could
/// still introduce — get `usize::MAX` so they sort to the end rather
/// than the algorithm looping forever.
fn compute_depth_ranks(tasks: &[Task]) -> HashMap<String, usize> {
    let ids_in_backlog: HashSet<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
    let mut depth: HashMap<String, usize> = HashMap::new();
    let mut remaining: Vec<&Task> = tasks.iter().collect();
    loop {
        let before = remaining.len();
        remaining.retain(|task| {
            let ready = task.dependencies.iter().all(|dep| {
                !ids_in_backlog.contains(dep.as_str()) || depth.contains_key(dep)
            });
            if !ready {
                return true;
            }
            let my_depth = task
                .dependencies
                .iter()
                .filter_map(|dep| depth.get(dep).copied())
                .max()
                .map(|d| d + 1)
                .unwrap_or(0);
            depth.insert(task.id.clone(), my_depth);
            false
        });
        if remaining.len() == before {
            break;
        }
    }
    for task in remaining {
        depth.insert(task.id.clone(), usize::MAX);
    }
    depth
}

fn status_rank(status: Status) -> u8 {
    match status {
        Status::NotStarted => 0,
        Status::InProgress => 1,
        Status::Blocked => 2,
        Status::Done => 3,
    }
}

fn status_to_cli_str(status: Status) -> &'static str {
    match status {
        Status::NotStarted => "not_started",
        Status::InProgress => "in_progress",
        Status::Blocked => "blocked",
        Status::Done => "done",
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

    fn task(id: &str, title: &str, category: &str, status: Status, deps: &[&str]) -> Task {
        Task {
            id: id.into(),
            title: title.into(),
            category: category.into(),
            status,
            blocked_reason: if status == Status::Blocked {
                Some("upstream".into())
            } else {
                None
            },
            dependencies: deps.iter().map(|s| (*s).into()).collect(),
            description: "Body.\n".into(),
            results: if status == Status::Done {
                Some("Done.\n".into())
            } else {
                None
            },
            handoff: None,
        }
    }

    #[test]
    fn empty_backlog_renders_a_placeholder_and_no_empty_tables() {
        let out = render_markdown(&BacklogFile::default(), GroupBy::Category);
        assert_eq!(out, "_(no tasks)_\n");
        assert!(!out.contains("|---"), "must not emit table scaffolding for empty input");
    }

    #[test]
    fn group_by_status_orders_sections_not_started_in_progress_blocked_done() {
        let backlog = BacklogFile {
            tasks: vec![
                task("d", "D", "core", Status::Done, &[]),
                task("b", "B", "core", Status::Blocked, &[]),
                task("i", "I", "core", Status::InProgress, &[]),
                task("n", "N", "core", Status::NotStarted, &[]),
            ],
            extra: Default::default(),
        };
        let out = render_markdown(&backlog, GroupBy::Status);

        // Sections appear in the pinned status order.
        let i_not = out.find("## not_started").expect("section missing");
        let i_in = out.find("## in_progress").expect("section missing");
        let i_blk = out.find("## blocked").expect("section missing");
        let i_done = out.find("## done").expect("section missing");
        assert!(i_not < i_in, "not_started must come before in_progress");
        assert!(i_in < i_blk, "in_progress must come before blocked");
        assert!(i_blk < i_done, "blocked must come before done");
    }

    #[test]
    fn within_a_group_rows_order_by_status_rank_first() {
        let backlog = BacklogFile {
            tasks: vec![
                task("d", "D", "core", Status::Done, &[]),
                task("b", "B", "core", Status::Blocked, &[]),
                task("n", "N", "core", Status::NotStarted, &[]),
                task("i", "I", "core", Status::InProgress, &[]),
            ],
            extra: Default::default(),
        };
        let out = render_markdown(&backlog, GroupBy::Category);

        let i_n = out.find("| N |").expect("row N missing");
        let i_i = out.find("| I |").expect("row I missing");
        let i_b = out.find("| B |").expect("row B missing");
        let i_d = out.find("| D |").expect("row D missing");
        assert!(i_n < i_i, "not_started row must come before in_progress");
        assert!(i_i < i_b, "in_progress row must come before blocked");
        assert!(i_b < i_d, "blocked row must come before done");
    }

    #[test]
    fn rows_break_status_ties_by_dependency_depth_roots_first() {
        // Three not_started tasks in a linear dep chain: leaf → mid → root.
        // Chain inserted leaf-first to prove order comes from topology,
        // not from file order.
        let backlog = BacklogFile {
            tasks: vec![
                task("leaf", "Leaf", "core", Status::NotStarted, &["mid"]),
                task("mid", "Mid", "core", Status::NotStarted, &["root"]),
                task("root", "Root", "core", Status::NotStarted, &[]),
            ],
            extra: Default::default(),
        };
        let out = render_markdown(&backlog, GroupBy::Category);

        let i_root = out.find("| Root |").expect("row Root missing");
        let i_mid = out.find("| Mid |").expect("row Mid missing");
        let i_leaf = out.find("| Leaf |").expect("row Leaf missing");
        assert!(i_root < i_mid, "root must render before mid: got\n{out}");
        assert!(i_mid < i_leaf, "mid must render before leaf: got\n{out}");
    }

    #[test]
    fn group_by_category_emits_one_section_per_category_alphabetically() {
        let backlog = BacklogFile {
            tasks: vec![
                task("a", "A", "infra", Status::NotStarted, &[]),
                task("b", "B", "core", Status::NotStarted, &[]),
                task("c", "C", "docs", Status::NotStarted, &[]),
            ],
            extra: Default::default(),
        };
        let out = render_markdown(&backlog, GroupBy::Category);
        let i_core = out.find("## core").expect("core section missing");
        let i_docs = out.find("## docs").expect("docs section missing");
        let i_infra = out.find("## infra").expect("infra section missing");
        assert!(i_core < i_docs, "categories must sort alphabetically: got\n{out}");
        assert!(i_docs < i_infra, "categories must sort alphabetically: got\n{out}");
    }

    #[test]
    fn dependency_cell_shows_em_dash_when_no_deps_and_comma_list_otherwise() {
        let backlog = BacklogFile {
            tasks: vec![
                task("root", "Root", "core", Status::Done, &[]),
                task("other", "Other", "core", Status::Done, &[]),
                task("leaf", "Leaf", "core", Status::NotStarted, &["root", "other"]),
            ],
            extra: Default::default(),
        };
        let out = render_markdown(&backlog, GroupBy::Category);
        assert!(out.contains("| Root | done | — |"), "no-dep row must show em dash:\n{out}");
        assert!(
            out.contains("| Leaf | not_started | root, other |"),
            "multi-dep cell must comma-join in declared order:\n{out}"
        );
    }

    #[test]
    fn cell_contents_escape_pipe_characters_and_collapse_newlines() {
        let backlog = BacklogFile {
            tasks: vec![task(
                "weird-id",
                "Title with | pipe\nand newline",
                "core",
                Status::NotStarted,
                &[],
            )],
            extra: Default::default(),
        };
        let out = render_markdown(&backlog, GroupBy::Category);

        // Table structure must stay intact: exactly one row + header, not split.
        let rows: Vec<&str> = out.lines().filter(|l| l.starts_with("| ")).collect();
        assert_eq!(rows.len(), 2, "expected 1 header + 1 data row, got {rows:?}");

        assert!(
            out.contains("Title with \\| pipe and newline"),
            "pipe must be backslash-escaped and newline replaced with space:\n{out}"
        );
    }

    #[test]
    fn cycle_participants_sort_to_the_end_without_looping() {
        // a → b → c → a. Plus a clean root `z` that should sort first.
        let backlog = BacklogFile {
            tasks: vec![
                task("a", "A", "core", Status::NotStarted, &["b"]),
                task("b", "B", "core", Status::NotStarted, &["c"]),
                task("c", "C", "core", Status::NotStarted, &["a"]),
                task("z", "Z", "core", Status::NotStarted, &[]),
            ],
            extra: Default::default(),
        };
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
