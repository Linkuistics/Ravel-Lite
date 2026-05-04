# `migrate-v1-v2` — design

> Status: design ready for implementation. Brainstormed 2026-05-04;
> approved by the user across all sections.

## Goal

Implement `ravel-lite migrate-v1-v2 <old-plan-path> --as <new-name>` —
the one-shot, per-plan structural cutover from a v1 layout
(`<project>/LLM_STATE/<plan>/`) to a v2 layout
(`<config-dir>/plans/<plan>/`). Once shipped, the v2 binary refuses to
operate cycle-shaped verbs against unmigrated v1 plans.

This is the deployment unblocker for v2.x: the
`LLM_STATE-shape-frozen` constraint can be lifted only after this
verb exists and `LLM_STATE/core` itself has been dogfooded through it.

## Non-goals

- Not idempotent / re-runnable. A failed migration is recovered by
  `rm -rf <config-dir>/plans/<new-name>/` and a fresh attempt.
- Not a converter for legacy `.md` state files (`backlog.md`, etc.).
  Every existing plan is already YAML-shaped (verified across 13
  LLM_STATE dirs / 23 plans on 2026-05-04). Plans without YAML state
  are rejected with an actionable error.
- Not a hermetic test of the LLM phase prompts. The three migration
  prompts are validated by the manual dogfood run, not by unit tests.

## Verb shape

```
ravel-lite migrate-v1-v2 <old-plan-path> --as <new-name> [--config <dir>] [--yes]
```

| Flag | Meaning |
|---|---|
| `<old-plan-path>` | Absolute or relative path to a v1 plan dir. Source repo derived as `<old-plan-path>/../..`. |
| `--as <new-name>` | The v2 plan name. Required. Validated by `create::validate_plan_name`. Plan-rename is required because most projects have a `core` plan. |
| `--config <dir>` | Standard config-dir flag. Same shape as `init`/`create`/`repo`. Resolves via `config::resolve_config_dir`. |
| `--yes` | Skip the three confirm-before-apply prompts. |

### Failure modes (loud, no partial-state recovery)

- `<old-plan-path>` lacks `phase.md` and `backlog.yaml` → "not a v1 plan dir."
- `<old-plan-path>` lacks `backlog.yaml` but has `backlog.md` → "no `backlog.yaml` found — this plan has only legacy markdown state. Re-export it as YAML before migrating."
- `<old-plan-path>` already has `intents.yaml` → "this plan looks v2-shaped already."
- `<config-dir>/repos.yaml` missing → "config not initialised — run `ravel-lite init`."
- Source repo not in `repos.yaml` → "source repo not registered — run `ravel-lite repo add <slug> --url <u> --local-path <p>`."
- Source repo missing `.atlas/components.yaml` → "source repo has no Atlas index — run Atlas first."
- `<config-dir>/plans/<new-name>/` already exists → "plan name collision; pick a different `--as`."

## Two-half flow

### Half A — Mechanical (pure Rust)

1. Validate inputs (failure modes above).
2. `mkdir <config-dir>/plans/<new-name>/`.
3. Copy `phase.md`, `backlog.yaml`, `memory.yaml`, `session-log.yaml`, `latest-session.yaml` (whichever exist) into the new dir. Skip stale `.md` siblings entirely.

No `legacy_memory.md` rename. The existing `memory.yaml` is already
TMS-shaped (claim / justifications / status); the LLM phase enriches
it with `attribution` rather than re-extracting from prose.

### Half B — LLM-driven (three sequential headless one-shots)

Each phase: render prompt → spawn `agent.invoke_headless` → agent
writes a scratch YAML in the plan dir → runner reads scratch,
validates, applies, deletes scratch. **Confirm-before-apply** between
agent output and runner application; `--yes` skips all three confirms.

#### 4. `migrate-intent`

- **Input:** `phase.md` + `backlog.yaml` + components catalog (LLM invokes `ravel-lite atlas` CLI on demand).
- **Scratch:** `migrate-intent-proposal.yaml`:
  ```yaml
  intents: [<TMS items...>]
  item_attributions:
    - item_id: <existing-backlog-id>
      serves: <intent-id> | legacy
  ```
- **Apply:** Write `intents.yaml`. Walk `backlog.yaml`; for each item, either add a `serves-intent` justification pointing at the named intent, or stamp `legacy: true` on the item (a new optional bool extension field on `BacklogEntry`).

#### 5. `migrate-targets`

- **Input:** source repo's `.atlas/components.yaml` + plan name.
- **Scratch:** `migrate-targets-proposal.yaml` containing `{ targets: [{ component_ref, ... }] }`.
- **Apply:** Write `targets.yaml`; mount worktrees for each target via the existing target-mount machinery (same code path as `target mount`).

#### 6. `migrate-memory-backfill`

- **Input:** existing `memory.yaml` (full) + components catalog.
- **Scratch:** `migrate-memory-proposal.yaml`:
  ```yaml
  attributions:
    - entry_id: <existing-memory-id>
      attribution: <component-ref> | plan-process | null
  ```
- **Apply:** Update each entry's `attribution` field; entries with `null` attribution also receive `status: legacy` for the user to curate.

## The "v2 refuses v1" gate

**Detection** (path-shape, no marker file). A v2 plan dir is
`<X>/plans/<Y>/` where `<X>/repos.yaml` exists. Function:

```rust
pub fn validate_v2_plan_dir(plan_dir: &Path) -> Result<()>
```

Returns `Ok(())` if grandparent is named `plans/` AND great-grandparent
has `repos.yaml`. Otherwise errors with:

> "Plan dir `<path>` looks like a v1 layout (`<project>/LLM_STATE/<plan>/`). Ravel-lite 2.x does not run v1 plans directly — migrate it first:
>
> &nbsp;&nbsp;&nbsp;&nbsp;`ravel-lite migrate-v1-v2 <old-plan-path> --as <new-name>`
>
> Then run against `<config-dir>/plans/<new-name>/` instead."

**Placement.** Top of these verbs:

- `run`
- `triage` / `reflect` / `analyse-work` / `dream` (standalone phase invocations)
- `state set-phase` (the LLM-facing transition trigger)

**Not on:**

- `state ...` CRUD verbs (file-shape converters, location-agnostic)
- `migrate-v1-v2` itself (its job is to consume a v1 path)
- `survey`, `init`, `create`, `repo`, `atlas`, `findings` (context- or repo-scoped)

## Three new `LlmPhase` variants

Add to `src/types.rs::LlmPhase`:

```rust
MigrateIntent,
MigrateTargets,
MigrateMemoryBackfill,
```

Three new prompt files under `defaults/phases/`:

- `migrate-intent.md`
- `migrate-targets.md`
- `migrate-memory-backfill.md`

Rendered via the existing `compose_prompt` machinery. New tokens
substituted by the migrate verb before invoking:

- `{{OLD_PLAN_PATH}}` — for context referencing
- `{{NEW_PLAN_DIR}}` — `<config-dir>/plans/<new-name>/`
- `{{SOURCE_REPO_SLUG}}` — for `ComponentRef` formatting in proposals
- `{{SOURCE_REPO_PATH}}` — for the LLM to invoke `atlas` CLI against

Each phase's prompt instructs the agent to write a specific scratch
file under `<NEW_PLAN_DIR>/`, then exit. The migrate verb invokes
`agent.invoke_headless` directly, not via `phase_loop` (which is
cycle-shaped and inappropriate here).

## Module layout

```
src/
├── migrate_v1_v2/
│   ├── mod.rs            # pub fn run_migrate_v1_v2(...) -> Result<()>
│   ├── validate.rs       # input validation; v1-shape detection
│   ├── copy.rs           # file copy (Half A step 3)
│   ├── apply_intent.rs   # parse migrate-intent-proposal.yaml; mutate intents.yaml + backlog.yaml
│   ├── apply_targets.rs  # parse proposal; write targets.yaml; mount worktrees
│   └── apply_memory.rs   # parse proposal; mutate memory.yaml
├── types.rs              # add three new LlmPhase variants
└── main.rs               # add MigrateV1V2 top-level command

defaults/phases/
├── migrate-intent.md
├── migrate-targets.md
└── migrate-memory-backfill.md
```

The migrator orchestrator (`mod.rs::run_migrate_v1_v2`) drives:

1. validate
2. copy
3. invoke `MigrateIntent` → confirm → `apply_intent`
4. invoke `MigrateTargets` → confirm → `apply_targets` (mounts worktrees)
5. invoke `MigrateMemoryBackfill` → confirm → `apply_memory`

A new top-level module (not nested under `src/state/`) because the
operation crosses concerns: file moves, repo registry, agent
invocation, worktree mounting.

Coexists with two existing migrators of unrelated scope:

- `src/migrate_v1_to_v2.rs` — config-dir migrator (legacy `.yaml` config → `config.lua`).
- `src/state/migrate.rs` — to be deleted (see next section).

## Companion scope: remove the `state migrate` verb

Verified across all 13 LLM_STATE dirs: every plan has the YAML form;
no plan exists in pure-`.md` form. The legacy `state migrate`
(`.md → .yaml` converter) is no longer needed.

Delete in this task:

- `src/state/migrate.rs` (~800 lines) and its tests.
- The `StateCommands::Migrate` enum variant and its handler in `src/main.rs`.
- The `parse_*_markdown` functions and tests in:
  - `src/state/backlog/` (`parse_backlog_markdown`)
  - `src/state/memory/` (`parse_memory_markdown`)
  - `src/state/session_log/` (`parse_session_log_markdown`, `parse_latest_session_markdown`)
- (Confirm with `grep` first that nothing else uses these parsers.)

## Companion scope: cross-repo cleanup of stale `.md` siblings

Across 5 sibling repos (AppSpec, Ravel, APIAnyware-MacOS, TestAnyware,
IDEs/RacketPro), delete `.md` state files where a `.yaml` sibling
exists in the same dir:

- `backlog.md` (where `backlog.yaml` is present)
- `memory.md` (where `memory.yaml` is present)
- `session-log.md` (where `session-log.yaml` is present)
- `latest-session.md` (where `latest-session.yaml` is present)

**Not** deleted: `phase.md`, `prompt-*.md`, `decisions.md`,
`research-notes.md`, `related-plans.md`, and any other user-authored
`.md` files.

Per-repo: `rm` the matching files, then `git commit` with a
`chore: remove stale legacy .md state siblings` message. Skip any
repo that has pre-existing uncommitted changes unrelated to this and
flag it back to the user.

This cleanup happens after the migrator code lands and is verified
locally; it is mechanical and does not interact with the migrator
implementation.

## Test plan

### Hermetic tests (no LLM calls)

- `validate.rs`: v1 detection accepts YAML+`phase.md`; rejects pure-`.md`; rejects already-v2 (`intents.yaml` present); rejects bad config dir / unregistered repo / missing Atlas index / colliding `--as`.
- `copy.rs`: copies the right files, skips stale `.md` siblings, handles missing optional files (e.g. no `latest-session.yaml`).
- `apply_intent.rs`: synthetic proposal YAML → produces correct `intents.yaml` and adds correct `serves-intent`/`legacy` flags to `backlog.yaml`.
- `apply_targets.rs`: synthetic proposal → writes `targets.yaml`, mounts worktrees in a `TestRepo` fixture.
- `apply_memory.rs`: synthetic proposal → updates `attribution` fields; null attributions get `status: legacy`.
- `validate_v2_plan_dir`: accepts/rejects across path shapes (v1 path, v2 path, partial paths, paths missing `repos.yaml`).
- Orchestrator integration test using the **(α) stub-agent seam**: a test-only `Agent` impl that, when invoked headless for a `Migrate*` phase, writes a pre-baked proposal YAML from a fixture path to the scratch location.

### Manual integration test (post-merge, local-build, user-driven)

1. Duplicate `LLM_STATE/core` to a scratch path (e.g. `/tmp/v1-fixture/core`).
2. Build the binary locally (`cargo build --release`); do NOT install.
3. Run `./target/release/ravel-lite migrate-v1-v2 /tmp/v1-fixture/core --as ravel-lite-core --config <config-dir>`.
4. Inspect `<config-dir>/plans/ravel-lite-core/` for sanity: intents look reasonable, backlog items have justifications, targets mount, memory has attribution.
5. This is the only validation of the actual LLM phase outputs.

## Out-of-scope (explicit non-goals beyond the section above)

- Idempotency / resume after partial failure.
- Auto-registering source repos in `repos.yaml` (the verb errors and points the user at `repo add`).
- Auto-running Atlas on the source repo (the verb errors and points the user at running it).
- Migrating multiple plans in one invocation. One plan per `migrate-v1-v2` run.
- Updating documentation (separate backlog tasks already cover the tutorial / reference doc cutover).

## Settled defaults

| Decision | Choice | Why |
|---|---|---|
| Verb naming | `migrate-v1-v2` (top-level) | Aligns with the v2.x release; disambiguates from existing `state migrate`. |
| Plan rename | Required `--as <new-name>` | Most projects have a `core` plan; collisions are expected. |
| Config-dir resolution | Reuse existing `--config` flag and `resolve_config_dir` | The "context" and "config dir" are the same path in current code. |
| Confirm semantics | Default ON, three Y/N prompts (one per LLM proposal); `--yes` skips | Matches `finish` semantics; lets user abort early on a clearly-wrong intent extraction without wasting LLM time downstream. |
| `legacy: true` fallback | Yes — backlog items the LLM can't attribute get `legacy: true` rather than forcing every item to map to an intent | Pragmatic: not every legacy item is intent-shaped. |
| Test seam for orchestrator | (α) test-only `Agent` impl reading pre-baked YAML | Minimum new abstraction; avoids real subprocess in tests. |
| `related-plans.md` cleanup | Out of scope | Not a sibling of any `.yaml`; tighter scope. |
| Cross-repo cleanup operation | `rm` + per-repo commit; skip dirty trees and flag | Atomic per-repo, surgical, recoverable. |
