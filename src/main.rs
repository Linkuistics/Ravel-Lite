use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use tokio::sync::mpsc;

use ravel_lite::agent::claude_code::ClaudeCodeAgent;
use ravel_lite::agent::pi::PiAgent;
use ravel_lite::agent::Agent;
use ravel_lite::component_ref::ComponentRef;
use ravel_lite::config::{load_agent_config, load_shared_config, resolve_config_dir};
use ravel_lite::git::project_root_for_plan;
use component_ontology::cli::{parse_edge_kind, parse_evidence_grade, parse_lifecycle_scope};
use ravel_lite::state::filenames::PHASE_FILENAME;
use ravel_lite::types::{AgentConfig, LlmPhase, PlanContext};
use ravel_lite::ui::{run_tui, UI};
use ravel_lite::{
    create, init, multi_plan, phase_loop, related_components, repos, state, survey,
};

/// Force `dangerous: true` for every known LLM phase, overriding
/// whatever was loaded from the config file.
fn force_dangerous(config: &mut AgentConfig) {
    let phases = [
        LlmPhase::Triage,
        LlmPhase::Work,
        LlmPhase::AnalyseWork,
        LlmPhase::Reflect,
    ];
    for phase in phases {
        let params = config.params.entry(phase.as_str().to_string()).or_default();
        params.insert("dangerous".to_string(), serde_yaml::Value::Bool(true));
    }
}

/// Version string baked in at compile time by `build.rs`. Shape:
/// `0.1.0 (v0.1.0-2-g15c2c8c-dirty, built 2026-04-21T06:42:18Z)`.
/// When no tag or no git data is available, the describe slot falls
/// back to the short SHA or literal `unknown`; the timestamp slot
/// falls back to `unknown` only if `date` is unavailable on the
/// build host.
const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("GIT_DESCRIBE"),
    ", built ",
    env!("BUILD_TIMESTAMP"),
    ")"
);

const AFTER_HELP: &str = "\
Source:  https://github.com/Linkuistics/Ravel-Lite
Docs:    https://www.linkuistics.com/projects/ravel-lite/";

#[derive(Parser)]
#[command(
    name = "ravel-lite",
    about = "An orchestration loop for LLM development cycles",
    version = VERSION,
    after_help = AFTER_HELP,
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scaffold (or refresh) a ravel-context directory following the
    /// v2 layout. Path-optional: with no `--config`, init resolves the
    /// same precedence chain as every other subcommand
    /// (`$RAVEL_LITE_CONFIG`, then the XDG default at
    /// `<dirs::config_dir()>/ravel-lite/`). Idempotent: a re-run on an
    /// existing context preserves user content and only fills in
    /// missing pieces.
    Init {
        /// Path to the context directory to scaffold. Overrides
        /// `$RAVEL_LITE_CONFIG` and the default location at
        /// `<dirs::config_dir()>/ravel-lite/`. The directory may
        /// already exist (init is idempotent) or be missing (init
        /// creates it).
        #[arg(long)]
        config: Option<PathBuf>,
        /// Prune retired paths from a previously-scaffolded context.
        /// Never overwrites user-owned files (`config.lua`,
        /// `repos.yaml`, `findings.yaml`).
        #[arg(long)]
        force: bool,
    },
    /// Run the phase loop on one or more plan directories. With a
    /// single plan directory, behaviour is unchanged: the loop runs
    /// continuously, prompting between cycles. With two or more plan
    /// directories, multi-plan mode kicks in: every cycle starts with
    /// a survey across all plans, the user picks one from a numbered
    /// stdout prompt, and one phase cycle runs for the chosen plan.
    /// `--survey-state` is required for multi-plan and rejected for
    /// single-plan; it is read as `--prior` and rewritten at the end
    /// of every survey, so the file is the persistent integration
    /// point with the incremental survey path from item 5b.
    Run {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and the
        /// default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Skip Claude Code permission prompts for every phase (claude-code only).
        #[arg(long)]
        dangerous: bool,
        /// Pass `--debug-file /tmp/ravel-claude-debug.log` to every
        /// claude invocation and tee all ravel ↔ claude interaction
        /// (spawn argv, prompt, raw stdout/stderr, exit status) to
        /// `/tmp/ravel-embedding-debug.log`. Both files are truncated
        /// at the start of the run.
        #[arg(long)]
        debug: bool,
        /// Path to the survey state file used by multi-plan mode. The
        /// file is both the incremental-survey `--prior` input and the
        /// canonical YAML output written at the end of every survey.
        /// Required when more than one plan directory is supplied;
        /// rejected when exactly one is supplied.
        #[arg(long)]
        survey_state: Option<PathBuf>,
        /// One or more plan directories. With a single directory the
        /// behaviour is the original single-plan run loop. With two or
        /// more, multi-plan mode dispatches one cycle per
        /// survey-driven user selection.
        #[arg(required = true, num_args = 1..)]
        plan_dirs: Vec<PathBuf>,
    },
    /// Create a new plan directory via an interactive headful claude
    /// session. Loads the create-plan prompt template from
    /// <config-dir>/create-plan.md, appends the target path, and
    /// hands off to claude with inherited stdio so the user drives
    /// the conversation directly. Reuses the configured work-phase
    /// model; passes `--add-dir <parent>` to scope claude to the
    /// target parent directory.
    Create {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and the
        /// default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Plan name. Resolved to <context_root>/plans/<plan>/.
        /// See `create::validate_plan_name` for accepted characters.
        plan: String,
        /// Seed `target-requests.yaml` with one or more `<repo>:<component>`
        /// component references. Repeatable. Each reference is parsed and
        /// validated at the CLI boundary. The runner drains the queued
        /// requests at the first phase boundary, mounting worktrees the
        /// LLM otherwise would have proposed during the create dialogue.
        #[arg(long = "target")]
        targets: Vec<ComponentRef>,
    },
    /// Produce an LLM-driven plan status overview for one or more plan
    /// directories. Reads each plan's phase/backlog/memory into a single
    /// fresh-context claude session that returns a per-plan summary and
    /// a recommended invocation order, emitted as canonical YAML on
    /// stdout. Use `ravel-lite survey-format <file>` to render a saved
    /// YAML survey as human-readable markdown.
    Survey {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and the
        /// default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Plan directories (each containing phase.md). Replaces the
        /// former plan-root walk: callers now name plans individually.
        /// At least one required.
        #[arg(required = true, num_args = 1..)]
        plan_dirs: Vec<PathBuf>,
        /// Override the model used for the survey call. Overrides
        /// `models.survey` in agents/claude-code/config.yaml, which in
        /// turn overrides the DEFAULT_SURVEY_MODEL constant.
        #[arg(long)]
        model: Option<String>,
        /// Override the timeout (in seconds) for the `claude` subprocess
        /// call. Default is 300 seconds (5 minutes). The survey fails
        /// with a diagnostic error and a partial-stdout dump if claude
        /// does not produce a result within this window.
        #[arg(long)]
        timeout_secs: Option<u64>,
        /// Path to a prior survey YAML to use as the baseline for an
        /// incremental run. Plans whose `input_hash` matches the prior
        /// are carried forward verbatim; only changed and added plans
        /// are sent to the LLM. Rejected schemas and unrecognised
        /// versions produce a loud error with a remediation hint
        /// pointing at `--force`.
        #[arg(long)]
        prior: Option<PathBuf>,
        /// Re-analyse every plan regardless of whether its hash matches
        /// the prior. Has no effect without `--prior`. Intended for
        /// debugging and schema-bump remediation.
        #[arg(long)]
        force: bool,
    },
    /// Render a saved YAML survey file (as produced by `ravel-lite
    /// survey`) as human-readable markdown on stdout. Read-only; no
    /// network, no LLM call.
    SurveyFormat {
        /// Path to a YAML survey file to render.
        file: PathBuf,
    },
    /// Print the installed ravel-lite version. Equivalent to `--version`;
    /// the subcommand form matches the rest of the CLI surface.
    Version,
    /// Mutate plan state from prompts without the Read+Write tool-call
    /// overhead (and permission prompts) of writing files directly.
    /// Expose via a single `Bash(ravel-lite state *)` allowlist entry.
    State {
        #[command(subcommand)]
        command: StateCommands,
    },
    /// Manage the ravel-context repository registry
    /// (`<context>/repos.yaml`). Each entry maps a stable slug — the
    /// `repo_slug` half of every `ComponentRef` — to a clone URL plus an
    /// optional local checkout path. The registry is the per-context
    /// resolver every plan target, edge, and memory attribution leans
    /// on; slugs are intentionally non-renameable in v1 because a
    /// rename would cascade through plan state files.
    Repo {
        #[command(subcommand)]
        command: RepoCommands,
    },
    /// Read-only inspection of a plan's knowledge graph across all
    /// typed stores (intents, backlog items, memory entries). Mutation
    /// verbs continue to live under `state <kind>`; this surface is
    /// for cross-kind queries that the per-kind verbs can't naturally
    /// express.
    Plan {
        #[command(subcommand)]
        command: PlanCommands,
    },
    /// Manage the context-level findings inbox
    /// (`<context>/findings.yaml`). Findings are TMS items that triage
    /// or reflect raise when they observe something out of scope for
    /// the current plan. Nothing reads `findings.yaml` during plan
    /// execution — it is advisory cross-plan, mediated by the user
    /// (promote → new plan, file external bug, mark wontfix). See
    /// `docs/architecture-next.md` §"Findings inbox".
    Findings {
        #[command(subcommand)]
        command: FindingsCommands,
    },
    /// Read-only graph-RAG queries over the union of registered repos'
    /// `.atlas/components.yaml` (component nodes) and
    /// `.atlas/related-components.yaml` (typed edges). The catalog is
    /// produced by the Atlas indexer (sibling `atlas-contracts`
    /// workspace); this surface lets agents query it on demand without
    /// rendering the full catalog into prompts. See
    /// `docs/architecture-next.md` §"Catalog as graph (graph-RAG)".
    Atlas {
        #[command(subcommand)]
        command: AtlasCommands,
    },
    /// Inspect the layered fixed-memory namespace (coding-style guides,
    /// memory-style rules, the cli-tool-design checklist). Each entry is
    /// a slug pinning an embedded shipped file plus an optional user
    /// override at `<config-dir>/fixed-memory/<slug>.md`. `show` emits
    /// the embedded body, then a delimiter, then the user body when both
    /// layers are present so the LLM sees which guidance is the user's.
    FixedMemory {
        #[command(subcommand)]
        command: FixedMemoryCommands,
    },
}

#[derive(Subcommand)]
enum RepoCommands {
    /// Emit the registry as YAML on stdout (empty registry is valid output).
    List {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Register a repo under `<name>` with `--url <u>` and an optional
    /// `--local-path <p>` pointing at the user's regular checkout.
    /// Rejects duplicate names; the local path, when supplied, is
    /// resolved against the current working directory and stored as an
    /// absolute path.
    Add {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Stable slug for the repo. Used as `repo_slug` in every
        /// `ComponentRef` downstream.
        name: String,
        /// Clone URL (any form git accepts: ssh, https, file path, etc.).
        #[arg(long)]
        url: String,
        /// Optional path to an existing local checkout. When omitted,
        /// future operations that need a working tree clone into the
        /// context cache on demand.
        #[arg(long)]
        local_path: Option<PathBuf>,
    },
    /// Remove the entry for `<name>`. Errors if no such entry exists.
    Remove {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        name: String,
    },
}

#[derive(Subcommand)]
enum FixedMemoryCommands {
    /// Enumerate every fixed-memory slug across the embedded set and the
    /// `<config-dir>/fixed-memory/` overlay. Each entry surfaces `slug`,
    /// `description` (the file's first H1, if any), and `sources`
    /// (`embedded`, `user`, or both).
    List {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// `yaml` (default), `json`, or `markdown`. The yaml form matches
        /// the existing `state <kind> list` verbs.
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Emit the body of one fixed-memory entry. With both layers present,
    /// the embedded body is printed first, then a delimiter, then the
    /// user body — signalling that the user content takes precedence.
    Show {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Bare slug (no extension, no path prefix). Round-trips with the
        /// `slug` field emitted by `list`.
        slug: String,
    },
}

#[derive(Subcommand)]
enum AtlasCommands {
    /// Emit the registered repos (the entry points to the catalog
    /// graph) as YAML on stdout. Bit-identical to `ravel-lite repo
    /// list`; surfaced under `atlas` to match the graph-RAG mental
    /// model where the registry is the catalog graph's root set.
    ListRepos {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Per-repo `.atlas/components.yaml` presence + age check. Always
    /// emits a YAML report on stdout; with `--require-fresh`, errors
    /// non-zero when any repo's catalog is missing, unparseable, or
    /// has no local checkout to read from.
    Freshness {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Exit non-zero when any repo lacks a fresh catalog. Intended
        /// for high-stakes pre-flight checks before graph queries that
        /// would silently return stale results.
        #[arg(long)]
        require_fresh: bool,
    },
    /// List every component in every fresh repo as
    /// `<repo_slug>/<component_id>  <kind>`, one per line. Use `--repo`
    /// to restrict to a single repo and/or `--kind` to restrict to a
    /// single component kind. Non-fresh repos are skipped silently;
    /// inspect freshness with `atlas freshness` first if needed.
    ListComponents {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Restrict output to this repo slug. Errors with the available
        /// fresh repos if the slug is unknown.
        #[arg(long)]
        repo: Option<String>,
        /// Restrict output to components whose `kind` matches exactly.
        #[arg(long)]
        kind: Option<String>,
    },
    /// Per-repo component counts grouped by kind. Lists each fresh repo
    /// followed by `<count>  <kind>` rows in alphabetical kind order.
    /// `--repo` restricts to a single repo.
    Summary {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Restrict output to this repo slug. Errors with the available
        /// fresh repos if the slug is unknown.
        #[arg(long)]
        repo: Option<String>,
    },
    /// Emit one component's full record as YAML, including a computed
    /// list of children. `<reference>` accepts the qualified form
    /// `<repo_slug>/<component_id>` or a bare `<component_id>` (the
    /// latter must resolve uniquely across fresh repos).
    Describe {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Component reference: `<repo_slug>/<component_id>` or bare
        /// `<component_id>` (must be unambiguous across fresh repos).
        reference: String,
    },
    /// Read the component's per-repo `.atlas/memory.yaml` and emit it
    /// as YAML. A missing file is reported as an empty memory file
    /// (the expected first-time state). With `--search`, restrict
    /// output to entries whose claim, attribution, or any
    /// justification's string fields contain the term (case-
    /// insensitive substring).
    Memory {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Component reference: `<repo_slug>/<component_id>` or bare
        /// `<component_id>` (must be unambiguous across fresh repos).
        reference: String,
        /// Restrict output to entries whose claim, attribution, or any
        /// justification's string fields contain this substring
        /// (case-insensitive).
        #[arg(long)]
        search: Option<String>,
    },
    /// List direct edges touching `<reference>` from the union of every
    /// fresh repo's `.atlas/related-components.yaml`. Default is
    /// `--both` (every edge involving the component); `--in` restricts
    /// to directed edges where the component is the destination,
    /// `--out` to directed edges where it is the source. Symmetric
    /// edges always surface regardless of the direction flag.
    Edges {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Component reference: `<repo_slug>/<component_id>` or bare
        /// `<component_id>` (must be unambiguous across fresh repos).
        reference: String,
        /// Show only directed edges where `<reference>` is the
        /// destination. Mutually exclusive with `--out` and `--both`.
        #[arg(long = "in", conflicts_with_all = ["outgoing", "both_dirs"])]
        incoming: bool,
        /// Show only directed edges where `<reference>` is the source.
        /// Mutually exclusive with `--in` and `--both`.
        #[arg(long = "out", conflicts_with_all = ["incoming", "both_dirs"])]
        outgoing: bool,
        /// Show every edge involving `<reference>` (the default when no
        /// direction flag is given). Mutually exclusive with `--in`
        /// and `--out`.
        #[arg(long = "both", conflicts_with_all = ["incoming", "outgoing"])]
        both_dirs: bool,
    },
    /// Bounded-depth BFS from `<reference>` over the undirected edge
    /// graph (every edge is traversable in either direction). Output
    /// is one line per reached component as `<hops>  <component>`,
    /// starting with the reference at hop 0. `--depth` defaults to 1.
    Neighbors {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Component reference: `<repo_slug>/<component_id>` or bare
        /// `<component_id>` (must be unambiguous across fresh repos).
        reference: String,
        /// Maximum hop count from `<reference>`. Defaults to 1; `0`
        /// emits only the starting component.
        #[arg(long, default_value_t = 1)]
        depth: usize,
    },
    /// Components with no incoming directed edges, qualified as
    /// `<repo_slug>/<component_id>`. Symmetric edges (e.g.
    /// `co-implements`) do not disqualify either endpoint because peer
    /// relationships do not establish hierarchy. Isolated components
    /// (those that appear in no edge at all) also surface.
    Roots {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// BFS shortest path from `<from>` to `<to>` over the directed
    /// component graph. Symmetric edges (`co-implements`,
    /// `communicates-with`) are excluded; only directed edge kinds
    /// (`depends-on`, `generates`, etc.) participate. Output is one
    /// bare component id per line in traversal order. Exits non-zero
    /// with "no path found" if no path within `--max-hops` exists.
    Path {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Source component reference: `<repo_slug>/<component_id>` or
        /// bare `<component_id>` (must be unambiguous across fresh repos).
        from: String,
        /// Destination component reference: same resolution rules as `<from>`.
        to: String,
        /// Maximum edge count of any returned path. Defaults to 10.
        #[arg(long = "max-hops", default_value_t = 10)]
        max_hops: usize,
    },
    /// Strongly connected components of the directed component graph
    /// via Tarjan's algorithm. Each SCC is printed on its own line as
    /// a comma-separated list of bare component ids. By default only
    /// non-trivial SCCs (size > 1) surface — useful for detecting
    /// circular dependencies — pass `--all` to include singletons.
    Scc {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Include trivial single-node SCCs in the output.
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand)]
enum StateCommands {
    /// Rewrite `<plan-dir>/phase.md` to the given phase. Validates the
    /// phase string and requires phase.md to already exist.
    SetPhase {
        /// Path to the plan directory whose phase.md to rewrite.
        plan_dir: PathBuf,
        /// Phase name to write (e.g. `analyse-work`, `git-commit-work`).
        phase: String,
    },
    /// Deprecated by the architecture-next migration. The per-user
    /// `projects.yaml` catalog has been replaced by the per-context
    /// `repos.yaml` registry. Any invocation of `state projects ...`
    /// prints a migration message and exits non-zero — use
    /// `ravel-lite repo {add,list,remove}` instead.
    Projects {
        /// Captured for backwards compatibility; ignored.
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        _args: Vec<String>,
    },
    /// Backlog CRUD verbs. Every prompt-side mutation of backlog.yaml
    /// goes through one of these.
    Backlog {
        #[command(subcommand)]
        command: BacklogCommands,
    },
    /// Intents CRUD verbs. `intents.yaml` is the canonical intent
    /// source under the architecture-next plan KG; `phase.md` becomes a
    /// rendered overview generated from intents content. Minimal verb
    /// surface at v1: `add`, `list`, `show`, `set-status`.
    Intents {
        #[command(subcommand)]
        command: IntentsCommands,
    },
    /// Memory CRUD verbs. Dream rewrites memory.yaml per-entry through
    /// these verbs rather than bulk-swapping the file.
    Memory {
        #[command(subcommand)]
        command: MemoryCommands,
    },
    /// Session-log verbs. `latest-session.yaml` is a single-record file
    /// written by analyse-work; `session-log.yaml` is the append-only
    /// history. `phase_loop::GitCommitWork` appends latest → log
    /// programmatically between phases.
    SessionLog {
        #[command(subcommand)]
        command: SessionLogCommands,
    },
    /// Targets CRUD verbs. `targets.yaml` is the runtime mount record
    /// of which Atlas components are projected into the plan as
    /// per-repo worktrees on plan-namespaced branches. Worktree
    /// mounting (the side that actually invokes `git worktree add`)
    /// is a separate task; these verbs operate on the data layer.
    Targets {
        #[command(subcommand)]
        command: TargetsCommands,
    },
    /// target-requests.yaml CRUD verbs and the manual `drain` trigger.
    /// `target-requests.yaml` is the scratch queue between request
    /// (work-phase LLM, or `ravel-lite create` seeding the initial
    /// targets) and mount; the runner drains it at every phase
    /// boundary. See `docs/architecture-next.md` §Dynamic mounting.
    TargetRequests {
        #[command(subcommand)]
        command: TargetRequestsCommands,
    },
    /// this-cycle-focus.yaml CRUD verbs. The focus record is the
    /// triage→work hand-off naming the target component, the backlog
    /// items to attempt, and any cycle-specific notes. Single-document
    /// surface (`set` / `show` / `clear`) — there is at most one focus
    /// at a time. See `docs/architecture-next.md` §TRIAGE step 6.
    ThisCycleFocus {
        #[command(subcommand)]
        command: ThisCycleFocusCommands,
    },
    /// focus-objections.yaml CRUD verbs. The work phase appends
    /// objections here when triage's focus is wrong; the next triage
    /// drains the file. Per-kind add verbs (`add-wrong-target`,
    /// `add-skip-item`, `add-premature`) keep the objection vocabulary
    /// closed so a hallucinated kind fails at the CLI boundary rather
    /// than landing on disk. See `docs/architecture-next.md` §WORK.
    FocusObjections {
        #[command(subcommand)]
        command: FocusObjectionsCommands,
    },
    /// commits.yaml read-only verbs. The file is the one-shot
    /// work-commit spec authored by analyse-work and consumed by
    /// `git-commit-work`; these verbs let an operator inspect it
    /// before or after the consume cycle. No `set` / `add` / `remove`
    /// because the LLM phase is the sole writer.
    Commits {
        #[command(subcommand)]
        command: CommitsCommands,
    },
    /// Single-plan conversion of legacy .md files into typed .yaml
    /// siblings. Covers backlog.md, memory.md, session-log.md and
    /// latest-session.md (each written when present).
    Migrate {
        plan_dir: PathBuf,
        #[arg(long)]
        dry_run: bool,
        /// Keep the .md originals on disk after migration (default).
        #[arg(long, conflicts_with = "delete_originals")]
        keep_originals: bool,
        /// Delete the .md originals only after write and validation both succeed.
        #[arg(long)]
        delete_originals: bool,
        /// Overwrite an existing backlog.yaml that differs from the re-migration output.
        #[arg(long)]
        force: bool,
    },
    /// Global component-relationship graph at
    /// `<config-dir>/related-components.yaml`. Edges follow the
    /// component-ontology v2 schema (see docs/component-ontology.md);
    /// participants reference components by name (resolved per-user via
    /// the projects catalog), so the file is shareable between users.
    RelatedComponents {
        #[command(subcommand)]
        command: RelatedComponentsCommands,
    },
    /// Stage 2 discovery emits each edge through `add-proposal` instead
    /// of writing `discover-proposals.yaml` directly. A hallucinated
    /// `--kind` is rejected by clap with the full valid vocabulary in
    /// the error message, so the LLM retries that single call rather
    /// than nuking the whole file.
    DiscoverProposals {
        #[command(subcommand)]
        command: DiscoverProposalsCommands,
    },
    /// Deterministic labelled-line summary of what changed in
    /// backlog.yaml (triage) or memory.yaml (reflect/dream) between a
    /// baseline commit and the current working-tree state. Replaces the
    /// LLM's manual re-transcription of its own tool calls at the end
    /// of each phase; the narrative preamble stays in the LLM.
    PhaseSummary {
        #[command(subcommand)]
        command: PhaseSummaryCommands,
    },
}

#[derive(Subcommand)]
enum PhaseSummaryCommands {
    /// Emit the labelled summary for a phase given its baseline SHA.
    Render {
        /// Path to the plan directory.
        plan_dir: PathBuf,
        /// Which phase's summary to render: `triage`, `reflect`, or `dream`.
        #[arg(long)]
        phase: String,
        /// Git SHA holding the phase-start snapshot of backlog.yaml /
        /// memory.yaml. Empty or absent means "first cycle, no prior
        /// state" — only additions are reported.
        #[arg(long, default_value = "")]
        baseline: String,
        /// Output format: `text` (default, one labelled line per mutation)
        /// or `yaml` (structured sequence for machine consumption).
        #[arg(long, default_value = "text")]
        format: String,
    },
}

#[derive(Subcommand)]
enum BacklogCommands {
    /// Emit tasks matching the given filters.
    List {
        plan_dir: PathBuf,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        category: Option<String>,
        /// Shorthand for `status=active AND every dep is done`.
        #[arg(long)]
        ready: bool,
        /// Match tasks that carry a hand-off block.
        #[arg(long)]
        has_handoff: bool,
        /// Match done tasks missing a Results block.
        #[arg(long)]
        missing_results: bool,
        #[arg(long, default_value = "yaml")]
        format: String,
        /// Section layout when `--format markdown` is set: `category` (default) or `status`.
        #[arg(long, default_value = "category")]
        group_by: String,
    },
    /// Emit a single task by id.
    Show {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Append a new task.
    Add {
        plan_dir: PathBuf,
        #[arg(long)]
        title: String,
        #[arg(long)]
        category: String,
        #[arg(long, value_delimiter = ',')]
        dependencies: Vec<String>,
        /// Path to a file containing the markdown description body.
        #[arg(long, conflicts_with = "description")]
        description_file: Option<PathBuf>,
        /// `-` reads stdin; any other value is taken as the description inline.
        #[arg(long)]
        description: Option<String>,
    },
    /// One-shot bulk initialisation for create-plan. Refuses a non-empty backlog.
    Init {
        plan_dir: PathBuf,
        #[arg(long)]
        body_file: PathBuf,
    },
    /// Update a task's status. `--reason <text>` is required when setting to `blocked`.
    SetStatus {
        plan_dir: PathBuf,
        id: String,
        status: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Set a task's Results block from a file or stdin.
    SetResults {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Rewrite a task's Description (the brief authored at `add` time).
    ///
    /// Use when external references in the body — e.g. doc section
    /// anchors or file paths — have moved and the brief needs to catch
    /// up. For recording what a completed task produced, use
    /// `set-results` instead; for promote-vs-archive hand-offs use
    /// `set-handoff`.
    SetDescription {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Set a task's hand-off block from a file or stdin.
    SetHandoff {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Clear a task's hand-off block (triage uses after promote/archive).
    ClearHandoff {
        plan_dir: PathBuf,
        id: String,
    },
    /// Update a task's title. Id is preserved.
    SetTitle {
        plan_dir: PathBuf,
        id: String,
        new_title: String,
    },
    /// Replace a task's dependency list. Validates ids, rejects self-reference and cycles.
    SetDependencies {
        plan_dir: PathBuf,
        id: String,
        /// Comma-separated list of task ids. Pass `--deps ""` to clear all deps.
        #[arg(long, value_delimiter = ',')]
        deps: Vec<String>,
    },
    /// Move a task before or after another in the backlog list.
    Reorder {
        plan_dir: PathBuf,
        id: String,
        position: String,
        target_id: String,
    },
    /// Delete a task. Refuses if the task is a dependency of another unless `--force`.
    Delete {
        plan_dir: PathBuf,
        id: String,
        #[arg(long)]
        force: bool,
    },
    /// Report drift between prose task-id mentions in task descriptions
    /// and the structured `dependencies:` field. Read-only; reconciliation
    /// is still done via `set-dependencies`.
    LintDependencies {
        plan_dir: PathBuf,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Repair stale task statuses: flip `blocked` tasks whose
    /// structural dependencies are all `done` back to `active`.
    /// Emits a report and (unless `--dry-run`) writes the repaired
    /// backlog. Exit code: 0 if no repairs applied, 1 if any repairs
    /// applied (scripting signal).
    RepairStaleStatuses {
        plan_dir: PathBuf,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
}

#[derive(Subcommand)]
enum MemoryCommands {
    /// Emit every memory entry.
    List {
        plan_dir: PathBuf,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Emit a single memory entry by id.
    Show {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Append a new memory entry. `--title` becomes the TMS claim;
    /// `--body` becomes a single rationale justification.
    Add {
        plan_dir: PathBuf,
        #[arg(long)]
        title: String,
        /// Path to a file containing the markdown body.
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// `-` reads stdin; any other value is taken as the body inline.
        #[arg(long)]
        body: Option<String>,
        /// Authoring timestamp (RFC-3339). Defaults to current UTC.
        #[arg(long)]
        authored_at: Option<String>,
        /// Phase or process that authored this entry. Defaults to `unspecified`.
        #[arg(long)]
        authored_in: Option<String>,
        /// Component this entry should attach to at plan-finish promotion (`<repo_slug>:<component_id>`).
        #[arg(long)]
        attribution: Option<String>,
        /// Attach a `code-anchor` justification (repeatable). Format:
        /// `component=<ref>,path=<rel-path>,sha=<blob-sha>[,lines=<start>-<end>]`.
        /// `sha` is the git blob SHA at assertion time (`git hash-object <path>`).
        #[arg(long = "code-anchor")]
        code_anchor: Vec<String>,
    },
    /// One-shot bulk initialisation for create-plan. Refuses a non-empty memory.
    Init {
        plan_dir: PathBuf,
        #[arg(long)]
        body_file: PathBuf,
    },
    /// Rewrite the rationale justification text from a file or stdin.
    /// Verb name retained for phase-prompt continuity; under the TMS
    /// schema the "body" is the first `Justification::Rationale`.
    SetBody {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Update an entry's claim (formerly: title). Id is preserved.
    SetTitle {
        plan_dir: PathBuf,
        id: String,
        new_title: String,
    },
    /// Set an entry's status. Validates against the typed transition
    /// table (`active` → `defeated` | `superseded`).
    SetStatus {
        plan_dir: PathBuf,
        id: String,
        /// One of `active`, `defeated`, `superseded`.
        status: String,
    },
    /// Delete an entry by id.
    Delete {
        plan_dir: PathBuf,
        id: String,
    },
    /// Bounded-TMS check: walk every active memory entry's `code-anchor`
    /// justifications and report those whose path is missing or whose
    /// blob SHA no longer matches `sha_at_assertion`. Output is a YAML
    /// `SuspectReport` for the reflect phase to act on.
    CheckAnchors {
        plan_dir: PathBuf,
        /// Project root the anchor `path` fields resolve against. Defaults
        /// to the `<subtree>/<state-dir>/<plan>` derivation from `plan_dir`.
        #[arg(long)]
        project_root: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum IntentsCommands {
    /// Emit every intent.
    List {
        plan_dir: PathBuf,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Emit a single intent by id.
    Show {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Append a new intent. `--claim` becomes the TMS claim;
    /// `--body` becomes a single rationale justification.
    Add {
        plan_dir: PathBuf,
        #[arg(long)]
        claim: String,
        /// Path to a file containing the markdown body.
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// `-` reads stdin; any other value is taken as the body inline.
        #[arg(long)]
        body: Option<String>,
        /// Authoring timestamp (RFC-3339). Defaults to current UTC.
        #[arg(long)]
        authored_at: Option<String>,
        /// Phase or process that authored this entry. Defaults to `unspecified`.
        #[arg(long)]
        authored_in: Option<String>,
    },
    /// Set an intent's status. Validates against the typed transition
    /// table (`active` → `satisfied` | `defeated` | `superseded`).
    SetStatus {
        plan_dir: PathBuf,
        id: String,
        /// One of `active`, `satisfied`, `defeated`, `superseded`.
        status: String,
    },
}

#[derive(Subcommand)]
enum TargetsCommands {
    /// Emit every mounted target.
    List {
        plan_dir: PathBuf,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Emit a single target by `<repo_slug>:<component_id>`.
    Show {
        plan_dir: PathBuf,
        /// `<repo_slug>:<component_id>` reference.
        reference: String,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Append a new mounted-target record. The git-side worktree
    /// creation is the worktree-mounting task's job; this verb only
    /// writes the cache row.
    Add {
        plan_dir: PathBuf,
        /// Repo slug, matching a `repos.yaml` registry entry.
        #[arg(long)]
        repo: String,
        /// Atlas component id, unique within the repo.
        #[arg(long)]
        component: String,
        /// Worktree mount path, relative to the plan directory.
        /// Conventionally `.worktrees/<repo_slug>`.
        #[arg(long)]
        working_root: String,
        /// Plan-namespaced branch, conventionally
        /// `ravel-lite/<plan>/main`.
        #[arg(long)]
        branch: String,
        /// One path segment locating the component within its
        /// worktree. Repeat the flag for nested paths
        /// (e.g. `--path-segment crates --path-segment atlas-ontology`).
        #[arg(long = "path-segment")]
        path_segments: Vec<String>,
    },
    /// Drop a mounted-target record by `<repo_slug>:<component_id>`.
    /// Worktree teardown (`git worktree remove`) is the
    /// worktree-mounting task's job.
    Remove {
        plan_dir: PathBuf,
        /// `<repo_slug>:<component_id>` reference.
        reference: String,
    },
    /// Mount a target as a git worktree under `<plan>/.worktrees/`.
    /// Resolves `<repo>` against the context's `repos.yaml`, creates
    /// the worktree on the plan-namespaced branch
    /// `ravel-lite/<plan>/main`, and writes the resulting Target row
    /// into `<plan>/targets.yaml`. Idempotent.
    Mount {
        plan_dir: PathBuf,
        /// `<repo_slug>:<component_id>` reference.
        reference: String,
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum TargetRequestsCommands {
    /// Emit every queued request.
    List {
        plan_dir: PathBuf,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Emit a single request by `<repo_slug>:<component_id>`.
    Show {
        plan_dir: PathBuf,
        reference: String,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Append a new mount request. The runner drains the queue at the
    /// next phase boundary; until then the request is visible to
    /// `list`/`show` but no worktree exists yet.
    Add {
        plan_dir: PathBuf,
        /// `<repo_slug>:<component_id>` reference.
        reference: String,
        /// Free-form explanation surfaced to a human inspecting the queue.
        #[arg(long)]
        reason: String,
    },
    /// Drop a queued request before the next drain.
    Remove {
        plan_dir: PathBuf,
        reference: String,
    },
    /// Drain the queue now: mount each request via `mount_target` and
    /// delete the file. The runner calls this between phases; this
    /// verb exists so an operator can drain manually too.
    Drain {
        plan_dir: PathBuf,
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum CommitsCommands {
    /// Emit the whole commits.yaml. Missing file renders as an empty
    /// list at the current schema version.
    List {
        plan_dir: PathBuf,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Emit a single commit entry by 1-based position. Positional
    /// addressing is used because commit specs have no stable identity
    /// field — the message is free-form prose, not a key.
    Show {
        plan_dir: PathBuf,
        /// 1-based index into the `commits` list.
        index: usize,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
}

#[derive(Subcommand)]
enum ThisCycleFocusCommands {
    /// Emit the current focus record, or error when no focus is set.
    Show {
        plan_dir: PathBuf,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Write the focus record, replacing any prior content. `--target`
    /// is a `<repo_slug>:<component_id>` ComponentRef. `--item` is a
    /// backlog item id; pass it once per item to attempt this cycle.
    /// `--notes` is free-form prose surfaced in the work-phase prompt.
    Set {
        plan_dir: PathBuf,
        /// `<repo_slug>:<component_id>` reference for the focus target.
        #[arg(long)]
        target: String,
        /// Backlog item id. Repeat to add more (`--item t-001 --item t-005`).
        #[arg(long = "item")]
        items: Vec<String>,
        /// Free-form notes surfaced verbatim into the work-phase prompt.
        #[arg(long)]
        notes: Option<String>,
    },
    /// Remove the focus file. Idempotent.
    Clear { plan_dir: PathBuf },
}

#[derive(Subcommand)]
enum FocusObjectionsCommands {
    /// Emit the queue of objections (empty queue is valid output).
    List {
        plan_dir: PathBuf,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Append a `wrong-target` objection.
    AddWrongTarget {
        plan_dir: PathBuf,
        /// `<repo_slug>:<component_id>` reference proposing a replacement target.
        #[arg(long)]
        suggested_target: ComponentRef,
        /// Free-form explanation surfaced verbatim into the next triage prompt.
        #[arg(long)]
        reasoning: String,
    },
    /// Append a `skip-item` objection.
    AddSkipItem {
        plan_dir: PathBuf,
        /// Backlog item id that should be skipped this cycle.
        #[arg(long)]
        item_id: String,
        /// Free-form explanation surfaced verbatim into the next triage prompt.
        #[arg(long)]
        reasoning: String,
    },
    /// Append a `premature` objection (the whole focus is premature).
    AddPremature {
        plan_dir: PathBuf,
        /// Free-form explanation surfaced verbatim into the next triage prompt.
        #[arg(long)]
        reasoning: String,
    },
    /// Drain the queue (delete the file). Idempotent.
    Clear { plan_dir: PathBuf },
}

#[derive(Subcommand)]
enum FindingsCommands {
    /// Emit every finding as YAML on stdout (an empty inbox is valid output).
    List {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location at <dirs::config_dir()>/ravel-lite/.
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Emit a single finding by id.
    Show {
        #[arg(long)]
        config: Option<PathBuf>,
        id: String,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Append a new finding. `--claim` becomes the TMS claim;
    /// `--body` becomes a single rationale justification. `--component`
    /// records optional component attribution; `--raised-in` records
    /// the plan that surfaced the finding.
    Add {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        claim: String,
        /// Path to a file containing the markdown body.
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// `-` reads stdin; any other value is taken as the body inline.
        #[arg(long)]
        body: Option<String>,
        /// Optional component attribution (e.g. `atlas:atlas-ontology`).
        #[arg(long)]
        component: Option<String>,
        /// Optional plan reference for the plan that surfaced the finding.
        #[arg(long)]
        raised_in: Option<String>,
        /// Authoring timestamp (RFC-3339). Defaults to current UTC.
        #[arg(long)]
        authored_at: Option<String>,
        /// Phase or process that authored this entry. Defaults to `unspecified`.
        #[arg(long)]
        authored_in: Option<String>,
    },
    /// Set a finding's status. Validates against the typed transition
    /// table (`new` → `promoted` | `wontfix` | `superseded`).
    SetStatus {
        #[arg(long)]
        config: Option<PathBuf>,
        id: String,
        /// One of `new`, `promoted`, `wontfix`, `superseded`.
        status: String,
    },
}

#[derive(Subcommand)]
enum PlanCommands {
    /// List items across the plan's typed stores. Without `--kind`,
    /// emits a unified `items:` list spanning intents + backlog +
    /// memory. With `--kind`, emits the matching kind's full file
    /// (same shape as `state <kind> list`).
    ListItems {
        plan_dir: PathBuf,
        /// One of `intent`, `backlog-item`, `memory-entry`, `finding`.
        /// Omit to list every kind in one document.
        #[arg(long)]
        kind: Option<String>,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Find an item by id without specifying its kind. Searches
    /// intents, backlog, and memory; errors if the id is ambiguous
    /// across kinds (use the per-kind `state <kind> show` verb to
    /// disambiguate).
    ShowItem {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Items whose status matches `--status`. With `--kind`, the
    /// status is parsed against that kind's vocabulary. Without
    /// `--kind`, every kind whose vocabulary includes the status
    /// string contributes to the unified result.
    QueryByStatus {
        plan_dir: PathBuf,
        /// One of `intent`, `backlog-item`, `memory-entry`, `finding`.
        /// Omit for cross-kind union.
        #[arg(long)]
        kind: Option<String>,
        /// Status string. Legal values depend on `--kind`:
        /// intent: `active`/`satisfied`/`defeated`/`superseded`;
        /// backlog-item: adds `done`/`blocked`;
        /// memory-entry: only `active`/`defeated`/`superseded`.
        #[arg(long)]
        status: String,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Items carrying at least one justification of the given kind.
    /// Useful for queries like "which backlog items serve an intent?"
    /// (`--justification-kind serves-intent`) or "which memory
    /// entries cite a code anchor?" (`--justification-kind code-anchor`).
    QueryByJustification {
        plan_dir: PathBuf,
        /// One of `intent`, `backlog-item`, `memory-entry`, `finding`.
        /// Omit for cross-kind union.
        #[arg(long)]
        kind: Option<String>,
        /// One of `code-anchor`, `rationale`, `serves-intent`,
        /// `defeats`, `supersedes`, `external`.
        #[arg(long)]
        justification_kind: String,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
}

#[derive(Subcommand)]
enum SessionLogCommands {
    /// List sessions from session-log.yaml (id + timestamp + phase + body).
    List {
        plan_dir: PathBuf,
        /// Truncate output to the last N sessions (newest-kept).
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Show a single session record by id.
    Show {
        plan_dir: PathBuf,
        id: String,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
    /// Append a session record to session-log.yaml. Idempotent on id:
    /// a record with the same id already present is a no-op.
    Append {
        plan_dir: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        timestamp: String,
        #[arg(long)]
        phase: Option<String>,
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Overwrite latest-session.yaml with a new single record. Used by
    /// analyse-work to hand the session to git-commit-work.
    SetLatest {
        plan_dir: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        timestamp: String,
        #[arg(long)]
        phase: Option<String>,
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        body: Option<String>,
    },
    /// Emit latest-session.yaml's record.
    ShowLatest {
        plan_dir: PathBuf,
        #[arg(long, default_value = "yaml")]
        format: String,
    },
}

#[derive(Subcommand)]
enum RelatedComponentsCommands {
    /// Emit the file as YAML. With `--plan`, filter to edges that
    /// involve the component derived from the plan dir. `--kind` and
    /// `--lifecycle` compose with `--plan` (all filters AND-combine).
    List {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Restrict output to edges involving the component that owns
        /// `<plan>` (derived as `<plan>/../..`).
        #[arg(long)]
        plan: Option<PathBuf>,
        /// Only emit edges whose kind matches this ontology v2 kebab-case
        /// name (e.g. `generates`, `co-implements`).
        #[arg(long)]
        kind: Option<String>,
        /// Only emit edges whose lifecycle matches this ontology v2
        /// kebab-case scope (e.g. `runtime`, `codegen`, `dev-workflow`).
        #[arg(long)]
        lifecycle: Option<String>,
    },
    /// Add an edge with the full ontology v2 field set. `kind` and
    /// `lifecycle` are positional; every other field is a flag.
    /// Symmetric kinds are participant-order-insensitive; directed
    /// kinds use canonical order from docs/component-ontology.md §6.
    /// Refuses unknown component names.
    AddEdge {
        #[arg(long)]
        config: Option<PathBuf>,
        /// One of the v2 kebab-case kinds (see
        /// docs/component-ontology.md §5).
        kind: String,
        /// One of the v2 kebab-case lifecycles (see
        /// docs/component-ontology.md §3.2).
        lifecycle: String,
        /// First participant. For directed kinds, the canonical-order
        /// "from" component.
        a: String,
        /// Second participant. For directed kinds, the canonical-order
        /// "to" component.
        b: String,
        /// Evidence grade: `strong`, `medium`, or `weak`. `strong`/`medium`
        /// require at least one `--evidence-field`; `weak` may omit.
        #[arg(long)]
        evidence_grade: String,
        /// Surface-field path that justifies this edge (e.g.
        /// `Ravel-Lite.produces_files`). Repeat for multiple fields.
        #[arg(long = "evidence-field", value_name = "FIELD")]
        evidence_fields: Vec<String>,
        /// One-paragraph human justification. Required; non-empty.
        #[arg(long)]
        rationale: String,
    },
    /// Remove the unique edge matching `(kind, lifecycle, canonicalised
    /// participants)`. Errors if no match. A v1-style invocation
    /// omitting `lifecycle` is rejected by clap's required-arg check.
    RemoveEdge {
        #[arg(long)]
        config: Option<PathBuf>,
        kind: String,
        lifecycle: String,
        a: String,
        b: String,
    },
    /// Run the two-stage LLM discovery pipeline over all catalogued
    /// components (or just `--project <name>`). Writes proposals to
    /// `<config-dir>/discover-proposals.yaml` for user review.
    Discover {
        #[arg(long)]
        config: Option<PathBuf>,
        /// Restrict Stage 1 re-analysis to a single component; Stage 2
        /// still operates over the full catalog's cached surfaces.
        #[arg(long)]
        project: Option<String>,
        /// Maximum parallel Stage 1 subagents. Default 4.
        #[arg(long)]
        concurrency: Option<usize>,
        /// Skip the review gate: run `discover-apply` immediately after
        /// proposals are written.
        #[arg(long)]
        apply: bool,
    },
    /// Merge a previously-produced `discover-proposals.yaml` into
    /// `related-components.yaml`. Idempotent; reports and rejects
    /// directional conflicts without aborting.
    DiscoverApply {
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum DiscoverProposalsCommands {
    /// Append a Stage 2 edge proposal to `<config-dir>/discover-proposals.yaml`.
    /// Every validation is enforced here rather than at batch-parse time —
    /// clap rejects an unknown `--kind`/`--lifecycle`/`--evidence-grade`,
    /// `Edge::validate()` rejects self-loops and empty-evidence misuse, and
    /// the catalog check rejects participants not in `repos.yaml`.
    AddProposal {
        /// Path to the config directory. Overrides $RAVEL_LITE_CONFIG and
        /// the default location.
        #[arg(long)]
        config: Option<PathBuf>,
        /// One of the v2 kebab-case kinds (see
        /// docs/component-ontology.md §5).
        #[arg(long)]
        kind: String,
        /// One of the v2 kebab-case lifecycles (see
        /// docs/component-ontology.md §3.2).
        #[arg(long)]
        lifecycle: String,
        /// Component name (repeat twice). For directed kinds, the first
        /// `--participant` is the canonical-order "from" component and
        /// the second is the "to" component. Symmetric kinds are
        /// participant-order-insensitive; the verb canonicalises to
        /// sorted order before storage.
        #[arg(long = "participant", value_name = "NAME")]
        participants: Vec<String>,
        /// Evidence grade: `strong`, `medium`, or `weak`. `strong`/`medium`
        /// require at least one `--evidence-field`; `weak` may omit.
        #[arg(long)]
        evidence_grade: String,
        /// Surface-field path that justifies this edge (e.g.
        /// `Alpha.surface.produces_files`). Repeat for multiple fields.
        #[arg(long = "evidence-field", value_name = "FIELD")]
        evidence_fields: Vec<String>,
        /// One-paragraph human justification citing specific surface
        /// fields from the input. Required; non-empty.
        #[arg(long)]
        rationale: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { config, force } => {
            let target = ravel_lite::config::resolve_config_dir_for_init(config)?;
            init::run_init(&target, force)
        }
        Commands::Run { config, dangerous, debug, survey_state, plan_dirs } => {
            let config_root = resolve_config_dir(config)?;
            if debug {
                ravel_lite::debug_log::enable(ravel_lite::debug_log::EMBEDDING_DEBUG_FILE)?;
            }
            match plan_dirs.len() {
                0 => unreachable!("clap requires at least one plan_dir"),
                1 => {
                    if survey_state.is_some() {
                        anyhow::bail!(
                            "--survey-state is only meaningful with multiple plan \
                             directories; remove it or pass two or more plan_dirs."
                        );
                    }
                    run_phase_loop(&config_root, &plan_dirs[0], dangerous).await
                }
                _ => {
                    let state_path = survey_state.ok_or_else(|| {
                        anyhow::anyhow!(
                            "--survey-state <path> is required when more than one \
                             plan directory is supplied. The file holds the survey \
                             YAML between cycles and is read as `--prior` on each \
                             subsequent survey."
                        )
                    })?;
                    multi_plan::run_multi_plan(
                        &config_root,
                        &plan_dirs,
                        &state_path,
                        dangerous,
                    )
                    .await
                }
            }
        }
        Commands::Create { config, plan, targets } => {
            let config_root = resolve_config_dir(config)?;
            create::run_create(&config_root, &plan, &targets).await
        }
        Commands::Survey { config, plan_dirs, model, timeout_secs, prior, force } => {
            let config_root = resolve_config_dir(config)?;
            survey::run_survey(
                &config_root,
                &plan_dirs,
                model,
                timeout_secs,
                prior.as_deref(),
                force,
            )
            .await
        }
        Commands::SurveyFormat { file } => {
            survey::run_survey_format(&file)
        }
        Commands::Version => {
            println!("ravel-lite {VERSION}");
            Ok(())
        }
        Commands::State { command } => dispatch_state(command).await,
        Commands::Repo { command } => dispatch_repo(command),
        Commands::Plan { command } => dispatch_plan(command),
        Commands::Findings { command } => dispatch_findings(command),
        Commands::Atlas { command } => dispatch_atlas(command),
        Commands::FixedMemory { command } => dispatch_fixed_memory(command),
    }
}

fn parse_fixed_memory_format(input: &str) -> Result<ravel_lite::fixed_memory::OutputFormat> {
    ravel_lite::fixed_memory::OutputFormat::parse(input).ok_or_else(|| {
        anyhow::anyhow!(
            "invalid --format value {input:?}; expected `yaml`, `json`, or `markdown`"
        )
    })
}

fn dispatch_fixed_memory(command: FixedMemoryCommands) -> Result<()> {
    use ravel_lite::fixed_memory;

    match command {
        FixedMemoryCommands::List { config, format } => {
            let config_root = resolve_config_dir(config)?;
            let fmt = parse_fixed_memory_format(&format)?;
            let map = fixed_memory::discover(&config_root)?;
            let rendered = fixed_memory::render_list(&map, fmt)?;
            print!("{rendered}");
            Ok(())
        }
        FixedMemoryCommands::Show { config, slug } => {
            let config_root = resolve_config_dir(config)?;
            match fixed_memory::compose(&slug, &config_root) {
                Ok(body) => {
                    print!("{body}");
                    Ok(())
                }
                Err(err) => {
                    // UnknownSlug already names the remediation; surface
                    // the formatted message as the anyhow error so it
                    // lands on stderr at exit-1, per cli-tool-design §3.
                    bail!("{err}")
                }
            }
        }
    }
}

fn dispatch_atlas(command: AtlasCommands) -> Result<()> {
    use ravel_lite::atlas;
    match command {
        AtlasCommands::ListRepos { config } => {
            let context_root = resolve_config_dir(config)?;
            atlas::run_list_repos(&context_root)
        }
        AtlasCommands::Freshness { config, require_fresh } => {
            let context_root = resolve_config_dir(config)?;
            atlas::run_freshness(&context_root, require_fresh)
        }
        AtlasCommands::ListComponents { config, repo, kind } => {
            let context_root = resolve_config_dir(config)?;
            atlas::run_list_components(&context_root, repo.as_deref(), kind.as_deref())
        }
        AtlasCommands::Summary { config, repo } => {
            let context_root = resolve_config_dir(config)?;
            atlas::run_summary(&context_root, repo.as_deref())
        }
        AtlasCommands::Describe { config, reference } => {
            let context_root = resolve_config_dir(config)?;
            atlas::run_describe(&context_root, &reference)
        }
        AtlasCommands::Memory {
            config,
            reference,
            search,
        } => {
            let context_root = resolve_config_dir(config)?;
            atlas::run_memory(&context_root, &reference, search.as_deref())
        }
        AtlasCommands::Edges {
            config,
            reference,
            incoming,
            outgoing,
            both_dirs: _,
        } => {
            let context_root = resolve_config_dir(config)?;
            // No flag set → default to Both (the documented behavior).
            // clap's conflicts_with_all guarantees at most one of the
            // three is true.
            let direction = if incoming {
                atlas::EdgeDirection::In
            } else if outgoing {
                atlas::EdgeDirection::Out
            } else {
                atlas::EdgeDirection::Both
            };
            atlas::run_edges(&context_root, &reference, direction)
        }
        AtlasCommands::Neighbors {
            config,
            reference,
            depth,
        } => {
            let context_root = resolve_config_dir(config)?;
            atlas::run_neighbors(&context_root, &reference, depth)
        }
        AtlasCommands::Roots { config } => {
            let context_root = resolve_config_dir(config)?;
            atlas::run_roots(&context_root)
        }
        AtlasCommands::Path {
            config,
            from,
            to,
            max_hops,
        } => {
            let context_root = resolve_config_dir(config)?;
            atlas::run_path(&context_root, &from, &to, max_hops)
        }
        AtlasCommands::Scc { config, all } => {
            let context_root = resolve_config_dir(config)?;
            atlas::run_scc(&context_root, all)
        }
    }
}

fn parse_plan_format(input: &str) -> Result<ravel_lite::plan_inspect::OutputFormat> {
    ravel_lite::plan_inspect::OutputFormat::parse(input)
        .ok_or_else(|| anyhow::anyhow!("invalid --format value {input:?}; expected `yaml` or `json`"))
}

fn dispatch_plan(command: PlanCommands) -> Result<()> {
    use ravel_lite::plan_inspect::{
        run_list_items, run_query_by_justification, run_query_by_status, run_show_item,
        JustificationKindFilter, PlanItemKind,
    };

    match command {
        PlanCommands::ListItems { plan_dir, kind, format } => {
            let kind = kind.map(|s| PlanItemKind::parse(&s)).transpose()?;
            let fmt = parse_plan_format(&format)?;
            run_list_items(&plan_dir, kind, fmt)
        }
        PlanCommands::ShowItem { plan_dir, id, format } => {
            let fmt = parse_plan_format(&format)?;
            run_show_item(&plan_dir, &id, fmt)
        }
        PlanCommands::QueryByStatus {
            plan_dir,
            kind,
            status,
            format,
        } => {
            let kind = kind.map(|s| PlanItemKind::parse(&s)).transpose()?;
            let fmt = parse_plan_format(&format)?;
            run_query_by_status(&plan_dir, kind, &status, fmt)
        }
        PlanCommands::QueryByJustification {
            plan_dir,
            kind,
            justification_kind,
            format,
        } => {
            let kind = kind.map(|s| PlanItemKind::parse(&s)).transpose()?;
            let jk = JustificationKindFilter::parse(&justification_kind)?;
            let fmt = parse_plan_format(&format)?;
            run_query_by_justification(&plan_dir, kind, jk, fmt)
        }
    }
}

fn dispatch_repo(command: RepoCommands) -> Result<()> {
    match command {
        RepoCommands::List { config } => {
            let context_root = resolve_config_dir(config)?;
            repos::run_list(&context_root)
        }
        RepoCommands::Add {
            config,
            name,
            url,
            local_path,
        } => {
            let context_root = resolve_config_dir(config)?;
            repos::run_add(&context_root, &name, &url, local_path.as_deref())
        }
        RepoCommands::Remove { config, name } => {
            let context_root = resolve_config_dir(config)?;
            repos::run_remove(&context_root, &name)
        }
    }
}

async fn dispatch_state(command: StateCommands) -> Result<()> {
    match command {
        StateCommands::SetPhase { plan_dir, phase } => {
            state::run_set_phase(&plan_dir, &phase)
        }
        StateCommands::Projects { _args: _ } => Err(repos::migrate_projects_yaml_error()),
        StateCommands::Backlog { command } => dispatch_backlog(command),
        StateCommands::Intents { command } => dispatch_intents(command),
        StateCommands::Memory { command } => dispatch_memory(command),
        StateCommands::SessionLog { command } => dispatch_session_log(command),
        StateCommands::Targets { command } => dispatch_targets(command),
        StateCommands::TargetRequests { command } => dispatch_target_requests(command),
        StateCommands::ThisCycleFocus { command } => dispatch_this_cycle_focus(command),
        StateCommands::FocusObjections { command } => dispatch_focus_objections(command),
        StateCommands::Commits { command } => dispatch_commits(command),
        StateCommands::Migrate {
            plan_dir,
            dry_run,
            keep_originals: _,
            delete_originals,
            force,
        } => {
            let options = state::migrate::MigrateOptions {
                dry_run,
                original_policy: if delete_originals {
                    state::migrate::OriginalPolicy::Delete
                } else {
                    state::migrate::OriginalPolicy::Keep
                },
                force,
            };
            state::migrate::run_migrate(&plan_dir, &options)
        }
        StateCommands::RelatedComponents { command } => dispatch_related_components(command).await,
        StateCommands::DiscoverProposals { command } => dispatch_discover_proposals(command),
        StateCommands::PhaseSummary { command } => dispatch_phase_summary(command),
    }
}

fn dispatch_phase_summary(command: PhaseSummaryCommands) -> Result<()> {
    use ravel_lite::phase_summary::{self, Phase, RenderFormat};

    match command {
        PhaseSummaryCommands::Render {
            plan_dir,
            phase,
            baseline,
            format,
        } => {
            let phase = Phase::parse(&phase).ok_or_else(|| {
                anyhow::anyhow!(
                    "invalid --phase value {phase:?}; expected `triage`, `reflect`, or `dream`"
                )
            })?;
            let format = RenderFormat::parse(&format).ok_or_else(|| {
                anyhow::anyhow!(
                    "invalid --format value {format:?}; expected `text` or `yaml`"
                )
            })?;
            phase_summary::run_render(&plan_dir, phase, &baseline, format)
        }
    }
}

fn dispatch_discover_proposals(command: DiscoverProposalsCommands) -> Result<()> {
    match command {
        DiscoverProposalsCommands::AddProposal {
            config,
            kind,
            lifecycle,
            participants,
            evidence_grade,
            evidence_fields,
            rationale,
        } => {
            let config_root = resolve_config_dir(config)?;
            let kind = parse_edge_kind(&kind)?;
            let lifecycle = parse_lifecycle_scope(&lifecycle)?;
            let evidence_grade = parse_evidence_grade(&evidence_grade)?;
            let req = state::discover_proposals::AddProposalRequest {
                kind,
                lifecycle,
                participants: &participants,
                evidence_grade,
                evidence_fields,
                rationale,
            };
            state::discover_proposals::run_add_proposal(&config_root, &req)
        }
    }
}

async fn dispatch_related_components(command: RelatedComponentsCommands) -> Result<()> {
    match command {
        RelatedComponentsCommands::List { config, plan, kind, lifecycle } => {
            let config_root = resolve_config_dir(config)?;
            let kind = kind.as_deref().map(parse_edge_kind).transpose()?;
            let lifecycle = lifecycle.as_deref().map(parse_lifecycle_scope).transpose()?;
            let filter = related_components::ListFilter {
                plan: plan.as_deref(),
                kind,
                lifecycle,
            };
            related_components::run_list(&config_root, &filter)
        }
        RelatedComponentsCommands::AddEdge {
            config,
            kind,
            lifecycle,
            a,
            b,
            evidence_grade,
            evidence_fields,
            rationale,
        } => {
            let config_root = resolve_config_dir(config)?;
            let kind = parse_edge_kind(&kind)?;
            let lifecycle = parse_lifecycle_scope(&lifecycle)?;
            let evidence_grade = parse_evidence_grade(&evidence_grade)?;
            let req = related_components::AddEdgeRequest {
                kind,
                lifecycle,
                a: &a,
                b: &b,
                evidence_grade,
                evidence_fields,
                rationale,
            };
            related_components::run_add_edge(&config_root, &req)
        }
        RelatedComponentsCommands::RemoveEdge {
            config,
            kind,
            lifecycle,
            a,
            b,
        } => {
            let config_root = resolve_config_dir(config)?;
            let kind = parse_edge_kind(&kind)?;
            let lifecycle = parse_lifecycle_scope(&lifecycle)?;
            related_components::run_remove_edge(&config_root, kind, lifecycle, &a, &b)
        }
        RelatedComponentsCommands::Discover {
            config,
            project,
            concurrency,
            apply: apply_flag,
        } => {
            let config_root = resolve_config_dir(config)?;
            let options = ravel_lite::discover::RunDiscoverOptions {
                project_filter: project,
                concurrency,
                apply: apply_flag,
            };
            ravel_lite::discover::run_discover(&config_root, options).await
        }
        RelatedComponentsCommands::DiscoverApply { config } => {
            let config_root = resolve_config_dir(config)?;
            ravel_lite::discover::apply::run_discover_apply(&config_root)
        }
    }
}

fn dispatch_backlog(command: BacklogCommands) -> Result<()> {
    use knowledge_graph::ItemStatus;
    use ravel_lite::plan_kg::BacklogStatus;
    use ravel_lite::state::backlog::{self, GroupBy, ListFilter, ReorderPosition};

    match command {
        BacklogCommands::List {
            plan_dir,
            status,
            category,
            ready,
            has_handoff,
            missing_results,
            format,
            group_by,
        } => {
            let status = status
                .as_deref()
                .map(|s| {
                    BacklogStatus::parse(s).ok_or_else(|| {
                        anyhow::anyhow!(
                            "invalid --status value {s:?}; expected one of active, done, blocked, defeated, superseded"
                        )
                    })
                })
                .transpose()?;
            let filter = ListFilter {
                status,
                category,
                ready,
                has_handoff,
                missing_results,
            };
            let fmt = parse_output_format(&format)?;
            let grouping = GroupBy::parse(&group_by).ok_or_else(|| {
                anyhow::anyhow!(
                    "invalid --group-by value {group_by:?}; expected `category` or `status`"
                )
            })?;
            backlog::run_list(&plan_dir, &filter, fmt, grouping)
        }
        BacklogCommands::Show { plan_dir, id, format } => {
            let fmt = parse_output_format(&format)?;
            backlog::run_show(&plan_dir, &id, fmt)
        }
        BacklogCommands::Add {
            plan_dir,
            title,
            category,
            dependencies,
            description_file,
            description,
        } => {
            let description_body = resolve_body(description_file, description)?;
            let req = backlog::AddRequest {
                title,
                category,
                dependencies,
                description: description_body,
            };
            backlog::run_add(&plan_dir, &req)
        }
        BacklogCommands::Init { plan_dir, body_file } => {
            let text = std::fs::read_to_string(&body_file)
                .with_context(|| format!("failed to read {}", body_file.display()))?;
            let seed: backlog::BacklogFile = serde_yaml::from_str(&text)
                .with_context(|| format!("failed to parse {} as backlog.yaml", body_file.display()))?;
            backlog::run_init(&plan_dir, &seed)
        }
        BacklogCommands::SetStatus {
            plan_dir,
            id,
            status,
            reason,
        } => {
            let status = BacklogStatus::parse(&status).ok_or_else(|| {
                anyhow::anyhow!(
                    "invalid status {status:?}; expected one of active, done, blocked, defeated, superseded"
                )
            })?;
            backlog::run_set_status(&plan_dir, &id, status, reason.as_deref())
        }
        BacklogCommands::SetResults { plan_dir, id, body_file, body } => {
            let body = resolve_body(body_file, body)?;
            backlog::run_set_results(&plan_dir, &id, &body)
        }
        BacklogCommands::SetDescription { plan_dir, id, body_file, body } => {
            let body = resolve_body(body_file, body)?;
            backlog::run_set_description(&plan_dir, &id, &body)
        }
        BacklogCommands::SetHandoff { plan_dir, id, body_file, body } => {
            let body = resolve_body(body_file, body)?;
            backlog::run_set_handoff(&plan_dir, &id, &body)
        }
        BacklogCommands::ClearHandoff { plan_dir, id } => {
            backlog::run_clear_handoff(&plan_dir, &id)
        }
        BacklogCommands::SetTitle { plan_dir, id, new_title } => {
            backlog::run_set_title(&plan_dir, &id, &new_title)
        }
        BacklogCommands::SetDependencies { plan_dir, id, deps } => {
            // clap parses `--deps ""` as a single empty string; normalise to
            // an empty vec so the documented clearing form works.
            let deps: Vec<String> = deps.into_iter().filter(|d| !d.is_empty()).collect();
            backlog::run_set_dependencies(&plan_dir, &id, &deps)
        }
        BacklogCommands::Reorder { plan_dir, id, position, target_id } => {
            let pos = ReorderPosition::parse(&position).ok_or_else(|| {
                anyhow::anyhow!(
                    "invalid reorder position {position:?}; expected `before` or `after`"
                )
            })?;
            backlog::run_reorder(&plan_dir, &id, pos, &target_id)
        }
        BacklogCommands::LintDependencies { plan_dir, format } => {
            let fmt = parse_output_format(&format)?;
            backlog::run_lint_dependencies(&plan_dir, fmt)
        }
        BacklogCommands::RepairStaleStatuses { plan_dir, dry_run, format } => {
            let fmt = parse_output_format(&format)?;
            let count = backlog::run_repair_stale_statuses(&plan_dir, dry_run, fmt)?;
            // Non-zero exit iff any repair would apply — scripts poll
            // this verb before a mutating run to detect status drift
            // without parsing YAML.
            if count > 0 {
                std::process::exit(1);
            }
            Ok(())
        }
        BacklogCommands::Delete { plan_dir, id, force } => {
            backlog::run_delete(&plan_dir, &id, force)
        }
    }
}

fn parse_output_format(input: &str) -> Result<ravel_lite::state::backlog::OutputFormat> {
    ravel_lite::state::backlog::OutputFormat::parse(input)
        .ok_or_else(|| {
            anyhow::anyhow!("invalid --format value {input:?}; expected `yaml`, `json`, or `markdown`")
        })
}

fn parse_memory_format(input: &str) -> Result<ravel_lite::state::memory::OutputFormat> {
    ravel_lite::state::memory::OutputFormat::parse(input)
        .ok_or_else(|| anyhow::anyhow!("invalid --format value {input:?}; expected `yaml` or `json`"))
}

fn dispatch_memory(command: MemoryCommands) -> Result<()> {
    use ravel_lite::state::memory;

    match command {
        MemoryCommands::List { plan_dir, format } => {
            let fmt = parse_memory_format(&format)?;
            memory::run_list(&plan_dir, fmt)
        }
        MemoryCommands::Show { plan_dir, id, format } => {
            let fmt = parse_memory_format(&format)?;
            memory::run_show(&plan_dir, &id, fmt)
        }
        MemoryCommands::Add {
            plan_dir,
            title,
            body_file,
            body,
            authored_at,
            authored_in,
            attribution,
            code_anchor,
        } => {
            let body = resolve_body(body_file, body)?;
            let code_anchors = code_anchor
                .iter()
                .map(|raw| memory::parse_code_anchor(raw))
                .collect::<Result<Vec<_>>>()?;
            let req = memory::AddRequest {
                title,
                body,
                authored_at,
                authored_in,
                attribution,
                code_anchors,
            };
            memory::run_add(&plan_dir, &req)
        }
        MemoryCommands::Init { plan_dir, body_file } => {
            let text = std::fs::read_to_string(&body_file)
                .with_context(|| format!("failed to read {}", body_file.display()))?;
            let seed: memory::MemoryFile = serde_yaml::from_str(&text)
                .with_context(|| format!("failed to parse {} as memory.yaml", body_file.display()))?;
            memory::run_init(&plan_dir, &seed)
        }
        MemoryCommands::SetBody { plan_dir, id, body_file, body } => {
            let body = resolve_body(body_file, body)?;
            memory::run_set_body(&plan_dir, &id, &body)
        }
        MemoryCommands::SetTitle { plan_dir, id, new_title } => {
            memory::run_set_title(&plan_dir, &id, &new_title)
        }
        MemoryCommands::SetStatus { plan_dir, id, status } => {
            memory::run_set_status(&plan_dir, &id, &status)
        }
        MemoryCommands::Delete { plan_dir, id } => {
            memory::run_delete(&plan_dir, &id)
        }
        MemoryCommands::CheckAnchors { plan_dir, project_root } => {
            let root = match project_root {
                Some(p) => p,
                None => memory::default_project_root(&plan_dir)?,
            };
            let report = memory::check_anchors_from_disk(&plan_dir, &root)?;
            let yaml = serde_yaml::to_string(&report)
                .context("failed to serialise SuspectReport as YAML")?;
            print!("{yaml}");
            Ok(())
        }
    }
}

fn parse_intents_format(input: &str) -> Result<ravel_lite::state::intents::OutputFormat> {
    ravel_lite::state::intents::OutputFormat::parse(input)
        .ok_or_else(|| anyhow::anyhow!("invalid --format value {input:?}; expected `yaml` or `json`"))
}

fn dispatch_intents(command: IntentsCommands) -> Result<()> {
    use ravel_lite::state::intents;

    match command {
        IntentsCommands::List { plan_dir, format } => {
            let fmt = parse_intents_format(&format)?;
            intents::run_list(&plan_dir, fmt)
        }
        IntentsCommands::Show { plan_dir, id, format } => {
            let fmt = parse_intents_format(&format)?;
            intents::run_show(&plan_dir, &id, fmt)
        }
        IntentsCommands::Add {
            plan_dir,
            claim,
            body_file,
            body,
            authored_at,
            authored_in,
        } => {
            let body = resolve_body(body_file, body)?;
            let req = intents::AddRequest {
                claim,
                body,
                authored_at,
                authored_in,
            };
            intents::run_add(&plan_dir, &req)
        }
        IntentsCommands::SetStatus { plan_dir, id, status } => {
            intents::run_set_status(&plan_dir, &id, &status)
        }
    }
}

fn parse_targets_format(input: &str) -> Result<ravel_lite::state::targets::OutputFormat> {
    ravel_lite::state::targets::OutputFormat::parse(input)
        .ok_or_else(|| anyhow::anyhow!("invalid --format value {input:?}; expected `yaml` or `json`"))
}

fn dispatch_targets(command: TargetsCommands) -> Result<()> {
    use ravel_lite::state::targets;

    match command {
        TargetsCommands::List { plan_dir, format } => {
            let fmt = parse_targets_format(&format)?;
            targets::run_list(&plan_dir, fmt)
        }
        TargetsCommands::Show { plan_dir, reference, format } => {
            let fmt = parse_targets_format(&format)?;
            targets::run_show(&plan_dir, &reference, fmt)
        }
        TargetsCommands::Add {
            plan_dir,
            repo,
            component,
            working_root,
            branch,
            path_segments,
        } => {
            let req = targets::AddRequest {
                repo_slug: repo,
                component_id: component,
                working_root,
                branch,
                path_segments,
            };
            targets::run_add(&plan_dir, &req)
        }
        TargetsCommands::Remove { plan_dir, reference } => {
            targets::run_remove(&plan_dir, &reference)
        }
        TargetsCommands::Mount {
            plan_dir,
            reference,
            config,
        } => {
            let context_root = resolve_config_dir(config)?;
            let (repo_slug, component_id) = parse_target_reference(&reference)?;
            let mounted = targets::mount_target(&plan_dir, &context_root, &repo_slug, &component_id)?;
            println!(
                "mounted {}:{} at {}/{} on {}",
                mounted.repo_slug,
                mounted.component_id,
                plan_dir.display(),
                mounted.working_root,
                mounted.branch
            );
            Ok(())
        }
    }
}

fn parse_target_reference(reference: &str) -> Result<(String, String)> {
    match reference.split_once(':') {
        Some((repo, component)) if !repo.is_empty() && !component.is_empty() => {
            Ok((repo.to_string(), component.to_string()))
        }
        _ => anyhow::bail!(
            "target reference {reference:?} must be `<repo_slug>:<component_id>` with both parts non-empty"
        ),
    }
}

fn parse_target_requests_format(
    input: &str,
) -> Result<ravel_lite::state::target_requests::OutputFormat> {
    ravel_lite::state::target_requests::OutputFormat::parse(input)
        .ok_or_else(|| anyhow::anyhow!("invalid --format value {input:?}; expected `yaml` or `json`"))
}

fn dispatch_target_requests(command: TargetRequestsCommands) -> Result<()> {
    use ravel_lite::state::target_requests;

    match command {
        TargetRequestsCommands::List { plan_dir, format } => {
            let fmt = parse_target_requests_format(&format)?;
            target_requests::run_list(&plan_dir, fmt)
        }
        TargetRequestsCommands::Show { plan_dir, reference, format } => {
            let fmt = parse_target_requests_format(&format)?;
            target_requests::run_show(&plan_dir, &reference, fmt)
        }
        TargetRequestsCommands::Add { plan_dir, reference, reason } => {
            target_requests::run_add(&plan_dir, &reference, &reason)
        }
        TargetRequestsCommands::Remove { plan_dir, reference } => {
            target_requests::run_remove(&plan_dir, &reference)
        }
        TargetRequestsCommands::Drain { plan_dir, config } => {
            let context_root = resolve_config_dir(config)?;
            let mounted = target_requests::drain_target_requests(&plan_dir, &context_root)?;
            println!("drained {mounted} request(s) from {}/target-requests.yaml", plan_dir.display());
            Ok(())
        }
    }
}

fn parse_commits_format(
    input: &str,
) -> Result<ravel_lite::state::commits::OutputFormat> {
    ravel_lite::state::commits::OutputFormat::parse(input)
        .ok_or_else(|| anyhow::anyhow!("invalid --format value {input:?}; expected `yaml` or `json`"))
}

fn dispatch_commits(command: CommitsCommands) -> Result<()> {
    use ravel_lite::state::commits;

    match command {
        CommitsCommands::List { plan_dir, format } => {
            let fmt = parse_commits_format(&format)?;
            commits::run_list(&plan_dir, fmt)
        }
        CommitsCommands::Show { plan_dir, index, format } => {
            let fmt = parse_commits_format(&format)?;
            commits::run_show(&plan_dir, index, fmt)
        }
    }
}

fn parse_this_cycle_focus_format(
    input: &str,
) -> Result<ravel_lite::state::this_cycle_focus::OutputFormat> {
    ravel_lite::state::this_cycle_focus::OutputFormat::parse(input)
        .ok_or_else(|| anyhow::anyhow!("invalid --format value {input:?}; expected `yaml` or `json`"))
}

fn dispatch_this_cycle_focus(command: ThisCycleFocusCommands) -> Result<()> {
    use ravel_lite::state::this_cycle_focus;

    match command {
        ThisCycleFocusCommands::Show { plan_dir, format } => {
            let fmt = parse_this_cycle_focus_format(&format)?;
            this_cycle_focus::run_show(&plan_dir, fmt)
        }
        ThisCycleFocusCommands::Set {
            plan_dir,
            target,
            items,
            notes,
        } => this_cycle_focus::run_set(&plan_dir, &target, &items, notes.as_deref()),
        ThisCycleFocusCommands::Clear { plan_dir } => this_cycle_focus::run_clear(&plan_dir),
    }
}

fn parse_focus_objections_format(
    input: &str,
) -> Result<ravel_lite::state::focus_objections::OutputFormat> {
    ravel_lite::state::focus_objections::OutputFormat::parse(input)
        .ok_or_else(|| anyhow::anyhow!("invalid --format value {input:?}; expected `yaml` or `json`"))
}

fn dispatch_focus_objections(command: FocusObjectionsCommands) -> Result<()> {
    use ravel_lite::state::focus_objections;

    match command {
        FocusObjectionsCommands::List { plan_dir, format } => {
            let fmt = parse_focus_objections_format(&format)?;
            focus_objections::run_list(&plan_dir, fmt)
        }
        FocusObjectionsCommands::AddWrongTarget {
            plan_dir,
            suggested_target,
            reasoning,
        } => focus_objections::run_add_wrong_target(&plan_dir, suggested_target, &reasoning),
        FocusObjectionsCommands::AddSkipItem {
            plan_dir,
            item_id,
            reasoning,
        } => focus_objections::run_add_skip_item(&plan_dir, &item_id, &reasoning),
        FocusObjectionsCommands::AddPremature { plan_dir, reasoning } => {
            focus_objections::run_add_premature(&plan_dir, &reasoning)
        }
        FocusObjectionsCommands::Clear { plan_dir } => focus_objections::run_clear(&plan_dir),
    }
}

fn parse_findings_format(input: &str) -> Result<ravel_lite::state::findings::OutputFormat> {
    ravel_lite::state::findings::OutputFormat::parse(input)
        .ok_or_else(|| anyhow::anyhow!("invalid --format value {input:?}; expected `yaml` or `json`"))
}

fn dispatch_findings(command: FindingsCommands) -> Result<()> {
    use ravel_lite::state::findings;

    match command {
        FindingsCommands::List { config, format } => {
            let context_root = resolve_config_dir(config)?;
            let fmt = parse_findings_format(&format)?;
            findings::run_list(&context_root, fmt)
        }
        FindingsCommands::Show { config, id, format } => {
            let context_root = resolve_config_dir(config)?;
            let fmt = parse_findings_format(&format)?;
            findings::run_show(&context_root, &id, fmt)
        }
        FindingsCommands::Add {
            config,
            claim,
            body_file,
            body,
            component,
            raised_in,
            authored_at,
            authored_in,
        } => {
            let context_root = resolve_config_dir(config)?;
            let body = resolve_body(body_file, body)?;
            let req = findings::AddRequest {
                claim,
                body,
                component,
                raised_in,
                authored_at,
                authored_in,
            };
            findings::run_add(&context_root, &req)
        }
        FindingsCommands::SetStatus { config, id, status } => {
            let context_root = resolve_config_dir(config)?;
            findings::run_set_status(&context_root, &id, &status)
        }
    }
}

fn parse_session_log_format(input: &str) -> Result<ravel_lite::state::session_log::OutputFormat> {
    ravel_lite::state::session_log::OutputFormat::parse(input)
        .ok_or_else(|| anyhow::anyhow!("invalid --format value {input:?}; expected `yaml` or `json`"))
}

fn dispatch_session_log(command: SessionLogCommands) -> Result<()> {
    use ravel_lite::state::session_log;

    match command {
        SessionLogCommands::List { plan_dir, limit, format } => {
            let fmt = parse_session_log_format(&format)?;
            session_log::run_list(&plan_dir, limit, fmt)
        }
        SessionLogCommands::Show { plan_dir, id, format } => {
            let fmt = parse_session_log_format(&format)?;
            session_log::run_show(&plan_dir, &id, fmt)
        }
        SessionLogCommands::Append {
            plan_dir,
            id,
            timestamp,
            phase,
            body_file,
            body,
        } => {
            let body = resolve_body(body_file, body)?;
            let record = session_log::build_record_for_append(
                Some(id),
                Some(timestamp),
                phase,
                &body,
            )?;
            session_log::run_append(&plan_dir, &record)
        }
        SessionLogCommands::SetLatest {
            plan_dir,
            id,
            timestamp,
            phase,
            body_file,
            body,
        } => {
            let body = resolve_body(body_file, body)?;
            let record = session_log::build_record_for_append(
                Some(id),
                Some(timestamp),
                phase,
                &body,
            )?;
            session_log::run_set_latest(&plan_dir, &record)
        }
        SessionLogCommands::ShowLatest { plan_dir, format } => {
            let fmt = parse_session_log_format(&format)?;
            session_log::run_show_latest(&plan_dir, fmt)
        }
    }
}

/// Resolve `--body-file <path>` vs `--body <value>` vs `--body -` (stdin).
/// Exactly one of the two arguments must be set; if neither is set,
/// returns an empty string (used for optional bodies like an add with no
/// description).
fn resolve_body(body_file: Option<PathBuf>, body: Option<String>) -> Result<String> {
    match (body_file, body) {
        (Some(path), None) => std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display())),
        (None, Some(value)) if value == "-" => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("failed to read body from stdin")?;
            Ok(buf)
        }
        (None, Some(value)) => Ok(value),
        (None, None) => Ok(String::new()),
        (Some(_), Some(_)) => bail!("pass only one of --body-file or --body"),
    }
}

async fn run_phase_loop(config_root: &Path, plan_dir: &Path, dangerous: bool) -> Result<()> {
    if !plan_dir.join(PHASE_FILENAME).exists() {
        anyhow::bail!(
            "{}/{PHASE_FILENAME} not found. Is this a valid plan directory?",
            plan_dir.display()
        );
    }

    let shared_config = load_shared_config(config_root)?;
    let mut agent_config = load_agent_config(config_root, &shared_config.agent)?;
    let project_dir = project_root_for_plan(plan_dir)?;

    if dangerous {
        if shared_config.agent == "claude-code" {
            force_dangerous(&mut agent_config);
        } else {
            eprintln!(
                "warning: --dangerous has no effect for agent '{}' (claude-code only)",
                shared_config.agent
            );
        }
    }

    let ctx = PlanContext {
        plan_dir: plan_dir.to_string_lossy().to_string(),
        project_dir: project_dir.clone(),
        dev_root: Path::new(&project_dir)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        related_plans: related_components::read_related_plans_markdown(plan_dir),
        config_root: config_root.to_string_lossy().to_string(),
    };

    let agent: Arc<dyn Agent> = match shared_config.agent.as_str() {
        "claude-code" => Arc::new(ClaudeCodeAgent::new(
            agent_config,
            config_root.to_string_lossy().to_string(),
        )),
        "pi" => Arc::new(PiAgent::new(
            agent_config,
            config_root.to_string_lossy().to_string(),
        )),
        other => anyhow::bail!("Unknown agent: {other}"),
    };

    let (tx, rx) = mpsc::unbounded_channel();
    let ui = UI::new(tx);

    let tui_handle = tokio::spawn(run_tui(rx));

    let result = phase_loop::run_single_plan(agent, ctx, &ui).await;

    if let Err(ref e) = result {
        // Show the error inside the TUI first so the user sees it in
        // context, then wait for acknowledgement before tearing down.
        ui.log("");
        ui.log(&format!("  ✗  Fatal error: {e:#}"));
        let _ = ui.confirm("Exit ravel-lite?").await;
    }

    ui.quit();
    tui_handle.await??;

    // Also emit to stderr so the error is preserved in the terminal
    // scrollback after the alternate screen has been torn down.
    if let Err(ref e) = result {
        eprintln!("\nravel-lite exited with error:\n{e:#}");
    }

    result
}
