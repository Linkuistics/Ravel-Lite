---
title: Ravel-Lite
---

Ravel-Lite is a multi-agent orchestrator for backlog-driven LLM development. You hand it a plan — a directory of YAML files describing tasks, memory, and current phase — and it runs a fixed cycle of phases against that plan, spawning [Claude Code](https://claude.ai/code) or [Pi](https://github.com/mariozechner/pi-coding-agent) as a subprocess for each one. The loop repeats until the backlog is empty or you stop it.

A single Rust binary with a [Ratatui](https://ratatui.rs) TUI. Every state transition is written to a readable file on disk. Nothing is embedded or hidden — all config, prompts, phase state, and memory are files you can inspect and edit. The orchestrator never calls LLM APIs directly; agents are subprocesses.

The phase cycle runs: `work → analyse-work → git-commit-work → reflect → git-commit-reflect → dream → git-commit-dream → triage → git-commit-triage → repeat`. Each git-commit phase writes an audit-trail commit. Ravel-Lite works cleanly in monorepo subtrees by scoping all git operations to the project subtree via pathspec. Versions are released with `cargo-release`; binaries self-identify their build commit via `ravel-lite version`.
