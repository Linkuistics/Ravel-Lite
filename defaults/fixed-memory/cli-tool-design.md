# Designing CLIs for Agentic (LLM) Use

Instructions for an LLM tasked with writing, auditing, or refactoring a CLI tool so that other LLM agents can use it reliably and efficiently.

## Core principle

LLM-friendliness is not a feature you bolt on with a special command. It is a property of the entire CLI surface — help text, flag conventions, output formats, error messages, exit codes, defaults. An agent's experience of your tool is dominated by what it sees the first time it runs `--help` and what it sees when something goes wrong. Optimise those, in that order, before anything else.

A separate `llm-instructions` command can supplement this, but it cannot substitute for it. Many agents will never call it. Treat it as the manual; treat `--help` as the reference.

## Discovery loop assumptions

Assume the agent's loop looks like this:

1. Run `tool --help` to see top-level subcommands.
2. Pick a likely subcommand, run `tool <sub> --help`.
3. Possibly recurse one more level.
4. Execute, parse output, branch on exit code.
5. On failure, read stderr and try once more.

Every friction point in this loop costs tokens, latency, or correctness. Design accordingly.

---

## The high-impact checklist

These are ordered by ROI. If you only fix a few things, fix them in this order.

### 1. Structured output on every data-producing command

Every command that emits data must support `--json` (or `--output json`) with a stable schema. Human-readable output is for humans and is not a parsing target. Document this explicitly in help text.

- Schema must be stable across patch versions. Breaking changes go in major versions and are documented.
- Prefer one object per logical record, not nested prose-like structures.
- Include enough metadata in each record that the agent doesn't need a second call (IDs, timestamps, status).
- For streaming/long outputs, prefer JSON Lines (`--output jsonl`) over a single giant array.
- Errors in `--json` mode should also be JSON. Don't mix prose stderr with JSON stdout silently.

```
# Good
$ tool list users --json
{"id":"u_1","name":"Ada","status":"active","created":"2025-01-01T00:00:00Z"}
{"id":"u_2","name":"Ben","status":"locked","created":"2025-01-02T00:00:00Z"}

# Bad
$ tool list users
👤 Ada    (active, joined Jan 1)
👤 Ben    (locked, joined Jan 2)
```

### 2. Help text with examples on every command

Every subcommand's `--help` output must include at least two concrete invocation examples at the bottom. LLMs pattern-match on examples far more reliably than on flag listings. A help page without examples is a help page that produces guesswork.

Help text structure, top to bottom:

1. One-line summary.
2. One-paragraph description (when the tool does, when it doesn't, key caveats).
3. Usage synopsis.
4. Arguments and flags, grouped logically.
5. Exit codes, if non-trivial.
6. Examples — at least two, ideally three, covering the most common real uses.
7. See-also references to related subcommands.

### 3. Actionable error messages

Every error message that has a known remediation must name it. Errors that don't have one should say so, so the agent stops retrying.

```
# Bad
Error: permission denied

# Good
Error: permission denied — no valid credentials found.
Run `tool auth login` to authenticate, or set TOOL_API_KEY in the environment.
```

For machine consumption, add an error code or category. In `--json` mode, errors should include a stable `code` field (e.g., `"AUTH_REQUIRED"`, `"NOT_FOUND"`, `"RATE_LIMITED"`) that agents can branch on without parsing the message.

### 4. Consistent flag vocabulary

Pick one name for each concept and use it everywhere. Inconsistencies here produce a long tail of agent failures that are hard to debug.

Recommended baseline vocabulary:

- `--json` / `--output <fmt>` for structured output.
- `--quiet` / `-q` for suppressing non-essential stderr.
- `--verbose` / `-v` for diagnostic detail (stderr, never stdout).
- `--dry-run` for preview without side effects.
- `--yes` / `-y` for non-interactive confirmation.
- `--force` for overriding safety checks (distinct from `--yes`).
- `--filter <expr>` or `--<field> <value>` for narrowing list output.
- `--limit <n>` for pagination size.
- `--all` for explicitly opting into unbounded results.
- `--format <tmpl>` for custom output templating, when relevant.

Don't mix synonyms across commands. If `delete` uses `--yes`, `purge` should not use `--confirm`.

### 5. Consistent verb/noun ordering

Pick one of these and apply it across the whole tool:

- **Noun-first** (`tool user create`, `tool user delete`) — like `kubectl`. Easier to discover related operations on one resource.
- **Verb-first** (`tool create user`, `tool delete user`) — like `git`. Easier when verbs are universal across many resource types.

Either works. Mixing them is what causes problems.

Add aliases for common synonyms (`rm`/`remove`/`delete`, `ls`/`list`, `mv`/`rename`). These are cheap and meaningfully reduce guess-the-verb failures.

### 6. Sensible default output limits

A `list` command that returns 50,000 rows by default can blow out an agent's context in a single call. Default to a reasonable page size (e.g., 50 or 100) with `--limit N` and `--all` overrides.

When truncating, say so explicitly in both human and JSON output:

```
$ tool list events
... 100 rows ...
Showing 100 of 12,438 results. Use --limit N or --all to see more.

$ tool list events --json
{"items":[...], "total":12438, "returned":100, "truncated":true}
```

### 7. Non-interactive by default when stdout isn't a TTY

Detect TTY and adapt:

- No pagers (`less`, `more`) when piped or captured.
- No progress spinners or carriage-return animations in captured output.
- No interactive confirmation prompts without a `--yes` flag available as an alternative.
- No colour codes unless `--color always` or a TTY is detected.

Destructive operations especially need a non-interactive escape hatch. An agent cannot answer a `[y/N]` prompt.

### 8. Meaningful, documented exit codes

`0` for success, non-zero for failure is the floor. Distinguishing categories pays off:

- `0` — success
- `1` — generic failure
- `2` — usage error (bad flags, missing args)
- `3` — not found
- `4` — auth required / forbidden
- `5` — conflict / precondition failed
- `6` — rate limited / try again later

Pick a scheme, document it, stick to it. Agents can then branch without parsing stderr.

### 9. Idempotency and side-effect clarity

For every mutating command, document:

- Is it idempotent? (Running twice = running once.)
- Is it safe to retry on transient failure?
- Does it have partial-failure semantics? (Did half the operation succeed?)

If a command is _not_ safe to retry, say so prominently in its help text. Agents retry by default.

Where possible, support idempotency keys (`--idempotency-key <uuid>`) on operations with non-trivial side effects.

### 10. Stable, parseable identifiers

Whatever IDs your tool returns, make them:

- Unambiguous (prefix-typed like `usr_abc123` is better than bare integers).
- Stable across calls.
- Usable as input to other commands without transformation.

Round-tripping IDs through other commands is a core agent pattern. If `tool list users --json` returns `id` fields, `tool show user <id>` must accept those exact values.

---

## Optional but valuable

### `tool capabilities` or `tool version --json`

A machine-readable summary of what the tool can do — version, supported subcommands, supported output formats, available features. Lets agents check feature availability without parsing help text. Cheap to implement, useful in long-running contexts.

```json
{
  "version": "2.4.1",
  "subcommands": ["user", "project", "deploy"],
  "output_formats": ["text", "json", "jsonl"],
  "features": { "idempotency_keys": true, "streaming": true }
}
```

### `tool schema <command>`

Emits the JSON schema for a command's `--json` output. Lets agents validate parsing and fail loudly when assumptions break. Especially useful if your tool has many subcommands with distinct output shapes.

### `tool llm-instructions` (optional)

A single command that prints a focused supplementary manual covering things that don't fit in `--help`:

- One-paragraph "what this tool is and isn't."
- Mental model of the command tree (naming conventions, verb/noun pattern).
- Two or three full workflow recipes showing real command chaining.
- Common mistakes section ("don't grep `list` output, use `--filter`"; "don't pipe `list` to `xargs do-thing`, use `do-thing --all`").
- Authentication and state assumptions.
- Pointers to `--json`, exit codes, idempotency conventions.

Keep it under a few thousand tokens. If it grows past that, add `--topic <name>` and `--section <name>` flags rather than introducing subcommands. Subcommands here add a round-trip and complicate discovery for no gain.

---

## Audit checklist

When auditing an existing CLI, walk this list. Each "no" is a finding.

**Output**

- [ ] Every data-producing command supports `--json` (or equivalent).
- [ ] JSON schema is documented and stable.
- [ ] Errors in JSON mode are also JSON, with a stable `code` field.
- [ ] List commands have a sensible default limit and indicate truncation.
- [ ] No ANSI colour or progress animation in non-TTY output.

**Help**

- [ ] Top-level `--help` lists all subcommands with one-line descriptions.
- [ ] Every subcommand `--help` includes at least two concrete examples.
- [ ] Help text states which output formats are stable and which are not.
- [ ] Exit codes are documented if non-trivial.

**Errors**

- [ ] Common errors name their remediation.
- [ ] Errors distinguish "your fault" from "system fault" via exit code or code field.
- [ ] No silent failures (commands either succeed or exit non-zero).

**Conventions**

- [ ] Flag vocabulary is consistent across all subcommands.
- [ ] Verb/noun ordering is consistent across all subcommands.
- [ ] Synonyms have aliases (rm/remove/delete, ls/list).
- [ ] Identifiers are prefix-typed and round-trippable.

**Behaviour**

- [ ] Destructive commands support `--yes` or `--force` for non-interactive use.
- [ ] Mutating commands document idempotency and retry safety.
- [ ] No interactive prompts trigger when stdout is non-TTY without an escape flag.
- [ ] `--dry-run` is available on commands with significant side effects.

**Discoverability (optional but valuable)**

- [ ] `tool version --json` or `tool capabilities` exists.
- [ ] `tool schema <cmd>` exists for commands with complex output.
- [ ] `tool llm-instructions` exists if there's meaningful workflow guidance that doesn't fit in `--help`.

---

## Anti-patterns to flag

- **Pretty output as the only output.** If parsing requires regex on prose, the command is broken for agents.
- **Unstable JSON.** Adding fields is fine; renaming or removing them between minor versions is not.
- **Help text without examples.** Flag listings alone produce guesswork.
- **Vague errors.** "Something went wrong" with no code, no remediation, no context.
- **Mixed flag vocabulary.** `--yes` here, `--confirm` there, `--noprompt` somewhere else.
- **Required interactive prompts.** Anything that blocks on stdin without a flag override.
- **Unbounded default lists.** `tool list everything` returning the entire dataset.
- **Bare-integer IDs.** Forces agents to track types externally.
- **Hidden state.** Commands whose behaviour depends on a config file or environment variable that isn't surfaced in help or errors.
- **Inconsistent exit codes.** Returning `1` for everything, or `0` on partial failures.
- **Opaque retries.** Mutating commands with no documented idempotency story.

---

## When refactoring an existing tool

Order of operations:

1. Add `--json` everywhere it's missing. Don't change defaults.
2. Stabilise and document the JSON schema.
3. Audit and rewrite error messages — actionable remediation, error codes.
4. Add examples to every `--help`.
5. Normalise flag vocabulary. Introduce new flag names; keep old ones as aliases for backwards compatibility.
6. Add exit code documentation; tighten exit code usage.
7. Add `--yes` / `--dry-run` / non-TTY behaviour.
8. Optionally: add `capabilities`, `schema`, `llm-instructions`.

Steps 1–4 are nearly always backwards-compatible and deliver most of the value. Steps 5–7 may require a major version bump. Step 8 is pure addition.

---

## Summary

The single highest-leverage change is **structured output with a stable schema, on every data command.** The second is **examples in every `--help`.** The third is **actionable error messages with stable error codes.** Everything else compounds on top of those three.

A tool that does those three things well is more agent-friendly than one with an elaborate `llm-instructions` command and inconsistent everything else.
