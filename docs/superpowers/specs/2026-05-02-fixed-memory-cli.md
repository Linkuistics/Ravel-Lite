# Spec: `fixed-memory` command tree + phase-prompt sweep

Date: 2026-05-02
Status: Approved

## Goal

Restore the broken `Read({{ORCHESTRATOR}}/fixed-memory/...)` references in
phase prompts, and make user extension/override of shipped fixed-memory
entries first-class via a new CLI command tree.

## Context

`init` no longer materialises `defaults/fixed-memory/*.md` into the
config dir. The dir itself is still scaffolded (empty) by
`src/init.rs:139`'s `LAYOUT_DIRS`, but the files are not written. Phase
prompts (`work.md`, `reflect.md`, `dream.md`) still instruct the LLM to
`Read {{ORCHESTRATOR}}/fixed-memory/<name>.md` — those reads now silently
miss.

The blocked backlog task
`add-ravel-lite-show-verb-for-embedded-fixed-memory-content` proposed a
thin `ravel-lite show <embedded-path>` wrapper. It was correctly blocked
because that shape would not honour user-extensibility: a user adding
`coding-style-haskell.md` to their config-dir overlay would be silently
ignored by an embedded-only `show` verb. This spec resolves that block by
making the new verb layered (embedded + user overlay) from day one.

## Settled decisions

- **Verbs.** `ravel-lite fixed-memory list` and `ravel-lite fixed-memory
  show <slug>`. Matches the codebase's universal `list` verb and the
  conventional `show` read-by-id verb (no other read-verb pattern is
  introduced).
- **Identifier shape.** Bare slug, no extension or path prefix
  (`coding-style-rust`, `memory-style`, `cli-tool-design`). The command
  namespace pins the directory; the `.md` extension is noise. Round-trips
  cleanly: `list` emits the slug, `show <slug>` accepts it verbatim.
- **Overlay semantics.** When both an embedded entry and a user file
  exist for the same slug, `show` emits embedded content first, then a
  clearly-marked addendum delimiter, then the user content. The delimiter
  signals to the LLM that the addendum takes precedence over conflicting
  guidance in the embedded section.
- **`list` output.** Defaults to YAML (matches the existing
  `state <kind> list` verbs); supports `--format yaml|json|markdown`.
  Each entry surfaces `slug`, `description` (= the file's first H1), and
  `sources: [embedded|user|both]`.

## Audit posture

Existing CLI surface partially conforms to the cli-tool-design.md
guidance: noun-first ordering is consistent, `--format` is widely
available across `state` verbs, flag vocabulary is largely uniform. Gaps
are concentrated in three areas — help-text examples, error
taxonomy/exit-code categories, and output-format consistency on a few
non-`state` verbs. A broader audit-and-refactor task is captured under
"Out of scope" below; this spec adheres to the parts of the guidance
that are cheap to honour now (noun-first verbs, `--format` parity with
`state`, actionable errors with remediation), and does not invent new
patterns the audit task would have to undo later.

## Architecture

One new module, `src/fixed_memory.rs`, exposing:

```rust
pub fn discover(config_dir: &Path) -> Result<BTreeMap<String, EntrySources>>;
pub fn compose(slug: &str, config_dir: &Path) -> Result<String, ShowError>;
pub fn extract_description(content: &str) -> Option<String>;

pub struct EntrySources {
    pub embedded: Option<&'static str>,
    pub user_path: Option<PathBuf>,
}

pub enum OutputFormat { Yaml, Json, Markdown }
impl OutputFormat { pub fn parse(s: &str) -> Option<Self>; }

pub enum ShowError {
    UnknownSlug { slug: String, available: Vec<String> },
    Io(std::io::Error),
}
```

Two new clap subcommands under a new top-level `Commands::FixedMemory`:

```rust
enum FixedMemoryCommands {
    List { config: Option<PathBuf>, format: OutputFormat },
    Show { config: Option<PathBuf>, slug: String },
}
```

Each handler is a thin wrapper that calls `discover` / `compose` and
renders the result. The `OutputFormat` enum is local to
`fixed_memory.rs`, matching the existing per-kind pattern under
`src/state/<kind>/verbs.rs` — consolidation of those enums into a
shared type is the audit task's job.

## Data flow

### `list`

1. Resolve config dir via the existing `resolve_config_dir`.
2. Walk `EMBEDDED_FILES` (via `embedded_entries_with_prefix("fixed-memory/")`)
   to collect embedded slugs.
3. Walk `<config-dir>/fixed-memory/*.md` to collect user slugs. Only
   `*.md` files become entries; non-`.md` files are silently ignored
   (so a user can drop a README in there without it polluting `list`).
   Missing directory is treated as "no user entries" (not an error).
4. Merge into `BTreeMap<slug, EntrySources>`.
5. For each entry, `extract_description` from the embedded content if
   present, else from the user content (`compose`-style precedence
   doesn't apply to descriptions because the *first* H1 is what we
   want, and the embedded H1 is the canonical title). When no H1 is
   present in either source, the `description` field is omitted from
   the rendered output (serde `skip_serializing_if = "Option::is_none"`)
   rather than emitted as `null`.
6. Render in the chosen format. The YAML/JSON shapes are documented in
   the schema appendix below; the markdown shape is a simple table
   `slug | description | sources`.

### `show`

1. Resolve config dir.
2. Look up slug in embedded set (`embedded_content("fixed-memory/<slug>.md")`).
3. Look up `<config-dir>/fixed-memory/<slug>.md` on disk.
4. Branch on the resolution:
   - **Embedded only** → print embedded content unchanged.
   - **User only** → print user content unchanged. No addendum delimiter
     because there's nothing to take precedence over.
   - **Both** → print embedded content, then the delimiter (below), then
     user content.
   - **Neither** → return `ShowError::UnknownSlug` → exit 1, stderr names
     all available slugs as remediation.

### Addendum delimiter

```
\n---\n\n## User addendum (takes precedence over the above)\n\n
```

(Surrounding blank lines for separation; `##` H2 heading is
structurally meaningful in markdown without colliding with H1s in the
source files. The user's content is emitted unchanged — if their file
also opens with `# ...` H1, that H1 nests under the addendum H2. This
is structurally unusual but readable and avoids the presumption of
stripping the user's heading.)

## Phase prompt sweep

Three files change. Every reference to `{{ORCHESTRATOR}}/fixed-memory/...`
is replaced with a `Bash(ravel-lite fixed-memory show <slug>)` invocation,
plus an instruction to `list` first when the relevant slug isn't fixed.

- **`defaults/phases/work.md`** lines 33–51: replace the
  `{{ORCHESTRATOR}}/fixed-memory/coding-style*.md` Read prose with a
  block instructing the LLM to first run `ravel-lite fixed-memory list`
  (so it sees user-overlay entries like a hypothetical `coding-style-
  haskell`) and then `ravel-lite fixed-memory show <slug>` for each
  applicable entry. This also fixes a latent bug: the previous prose
  hard-coded the language slugs the LLM should consider, so any user-
  added language guide was invisible.
- **`defaults/phases/reflect.md`** lines 18 and 84: swap
  `{{ORCHESTRATOR}}/fixed-memory/memory-style.md` for
  `Bash(ravel-lite fixed-memory show memory-style)`. Because
  `memory-style` is a fixed slug consulted unconditionally, no `list`
  preamble is needed here.
- **`defaults/phases/dream.md`** lines 9 and 22: same swap as reflect.

After the sweep, the drift-guard test
`shipped_pi_prompts_have_no_dangling_tokens` must still pass. (The
`{{ORCHESTRATOR}}` token continues to appear elsewhere in prompts; the
sweep removes only its `fixed-memory/` uses.)

## Allowlist

Add `Bash(ravel-lite fixed-memory:*)` to whatever per-phase allowlist
constants govern the work, reflect, and dream phases. The
implementation step locates the constants (`CREATE_ALLOWED_TOOLS` in
`src/create.rs:40` is the existing pattern).

## Error handling

- **Unknown slug on `show`**: exit 1, stderr `no fixed-memory entry for
  slug 'X'. Available slugs: a, b, c. Run 'ravel-lite fixed-memory list'
  to inspect.` (Per cli-tool-design.md §3, the message names the
  remediation.)
- **Empty config dir or missing `<config-dir>/fixed-memory/`**: not an
  error — treated as "no user entries". The list still has embedded
  entries; `show` of an embedded slug works.
- **Filesystem errors** (permission denied on user dir, etc.): bubble
  up as anyhow errors with file-path context.
- Exit-code categories stay at 0/1 for now. Full taxonomy
  (0/1/2/3/4/5/6 per cli-tool-design.md §8) is the audit task's job;
  introducing a fragmentary scheme here would create a second-system
  inconsistency.

## Schema appendix

### `list --format yaml`

```yaml
schema_version: 1
entries:
  - slug: coding-style
    description: "Universal coding style"
    sources: [embedded]
  - slug: coding-style-rust
    description: "Rust coding style"
    sources: [embedded, user]
  - slug: coding-style-haskell
    description: "Haskell coding style (user)"
    sources: [user]
```

### `list --format json`

The same shape as YAML, JSON-encoded. `schema_version` is at the top
level; entries is an array of objects with `slug`, `description`, and
`sources` (array of `"embedded"|"user"`).

### `list --format markdown`

```markdown
| slug | description | sources |
|---|---|---|
| coding-style | Universal coding style | embedded |
| coding-style-rust | Rust coding style | embedded, user |
| coding-style-haskell | Haskell coding style (user) | user |
```

## Testing

Unit tests in `src/fixed_memory.rs`:

- `extract_description` returns the H1 text for a normal file, `None`
  for a file with no H1, and handles leading whitespace / blank lines
  before the H1.
- `discover` against a temp config dir with mixed embedded/user/both
  entries returns the correct `EntrySources` per slug.
- `compose` for embedded-only, user-only, and both cases. The "both"
  case asserts the delimiter appears verbatim between the two sections.

Integration tests (in `tests/`):

- `ravel-lite fixed-memory list` against a temp config dir populated
  with one user-only file: YAML, JSON, and markdown renderings each
  contain the embedded entries plus the user one with correct `sources`.
- `ravel-lite fixed-memory show <slug>` for an embedded-only slug, a
  user-only slug, and a both-sources slug: each prints the expected
  content (and only the expected content).
- `ravel-lite fixed-memory show <unknown-slug>` exits non-zero; stderr
  contains both the slug name and "Available slugs:".

The existing drift-guard
(`every_file_under_defaults_is_registered_in_embedded_files`) covers
the embedded-side invariant unchanged.

## Out of scope (becomes new backlog task)

A new `not_started` backlog task `cli-audit-against-cli-tool-design`
(category: `architecture-next`) covers the broader audit:

- Add `--format json` to every data-producing verb (currently uneven:
  `repo list`, `atlas list-components`, etc. are YAML- or text-only).
- Stabilise and document JSON schemas; surface `schema_version` and a
  stable structure in every JSON-mode output.
- Consolidate the 11+ near-identical per-kind `OutputFormat` enums into
  a shared crate-level type.
- Add ≥2 examples to every subcommand `--help` block.
- Move from anyhow's 0/1 exit codes to the documented 0/1/2/3/4/5/6
  scheme; surface a `code` field in JSON-mode error envelopes.
- Optional discoverability: `ravel-lite capabilities`, `ravel-lite
  schema <command>`, `ravel-lite llm-instructions`.

The task body cites cli-tool-design.md §§1–10 directly so the audit is
checklist-driven. It is deliberately deferred until the in-flight
architecture-next feature work and v1→v2 migrator land — the
constraint from the
`LLM_STATE-shape-frozen-until-v1-v2-migrator-exists` memory entry
applies.

## Blocked-task disposition

`add-ravel-lite-show-verb-for-embedded-fixed-memory-content` is
**superseded** by this work. After the implementation lands, mark that
task `done` with a results block pointing at the layered overlay shape
shipped here. The underlying need (LLM-accessible fixed-memory) is fully
resolved; the originally-scoped `show <embedded-path>` verb shape is
explicitly *not* what we want, so leaving the task open as `not_started`
under a new shape would be misleading.
