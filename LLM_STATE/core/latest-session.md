### Session 8 (2026-04-22T03:44:10Z) — Implement state projects catalog (R4)

- Implemented R4: `state projects` catalog mapping project names to absolute paths
- Created `src/projects.rs` with `ProjectsCatalog` struct (schema_version: 1, projects list), atomic save, `auto_add` pure logic returning `AlreadyCatalogued`/`Added`/`NameCollision`, and `ensure_in_catalog_interactive` generic over `Read + Write`
- Wired CLI verbs `list`/`add`/`remove`/`rename` in `main.rs` under `StateCommands::Projects`
- Added `register_projects_from_plan_dirs` in `main.rs`, called before TUI startup in `Commands::Run`, so collision prompts reach a real tty before Ratatui's alternate-screen takeover
- `add` rejects relative paths (catalog is path-anchored; relative paths resolve differently from different CWDs)
- `rename` is scoped to catalog only — R5 adds the `related-projects.yaml` cascade
- 18 unit tests in module + 2 CLI integration tests (round-trip add→list→rename→remove; relative-path rejection); full suite 238+27 green; clippy clean
- All changes committed; R4 status already correctly marked `done` in backlog
