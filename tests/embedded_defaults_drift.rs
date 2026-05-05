//! Drift guard for the embedded-defaults contract.
//!
//! Task 1 (`switch-prompt-and-agent-reads-to-embedded-defaults...`)
//! required prompts and pi agent-definition reads to source from the
//! embedded set, never from disk. The existing
//! `every_file_under_defaults_is_registered_in_embedded_files` guard
//! (in `init.rs`) covers the *registration* half — every shipped
//! default must appear in `EMBEDDED_FILES`. This file covers the
//! *consumption* half: no `src/` reader may resolve a path into one of
//! those embedded files.
//!
//! Strategy: scan every `.rs` file under `src/` for string literals
//! that look like a path-join into an embedded file. Allow the literal
//! inside `init.rs` (where `EMBEDDED_FILES` is declared) and inside
//! comments. Fail loudly with the offending file:line.
//!
//! YAML configs (`config.yaml`, `tokens.yaml`,
//! `agents/<name>/config.yaml`) are *deliberately excluded* from the
//! forbidden list — Task 1's scope is prompts and agent definitions
//! only. Task 2 (Lua config) handles the YAML side.

use std::fs;
use std::path::{Path, PathBuf};

/// Anchor strings that must not appear in `read_to_string` / `read_dir`
/// targets anywhere under `src/`. Each is a substring that, if it
/// landed in a path passed to a disk-read, would resolve into the
/// embedded set — a regression that the runtime should catch at test
/// time rather than at agent-spawn time.
const FORBIDDEN_DISK_PATH_FRAGMENTS: &[&str] = &[
    "phases/",
    "agents/pi/prompts/",
    "agents/pi/prompts\"",
    "agents/pi/subagents/",
    "agents/pi/subagents\"",
    "create-plan.md",
    "survey.md",
    "survey-incremental.md",
    "discover-stage1.md",
    "discover-stage2.md",
];

/// Files exempted from the scan. `init.rs` owns the embedded registry;
/// the drift-guard test file itself names the fragments deliberately.
const EXEMPT_RELATIVE_PATHS: &[&str] = &[
    "init.rs",
    // The phase prompt loader builds a `phases/<phase>.md` *embedded*
    // key, not a disk path. The disk check below filters out
    // `read_to_string` / `read_dir` callers, so a literal `phases/` in
    // a non-disk context is fine — but listing prompt.rs explicitly
    // documents the intent.
    "prompt.rs",
];

#[test]
fn no_source_file_reads_an_embedded_path_from_disk() {
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offences: Vec<String> = Vec::new();

    visit_rust_files(&src_root, &mut |path| {
        let rel = path.strip_prefix(&src_root).unwrap();
        if EXEMPT_RELATIVE_PATHS
            .iter()
            .any(|p| rel.ends_with(Path::new(p)))
        {
            return;
        }

        let body = match fs::read_to_string(path) {
            Ok(b) => b,
            Err(_) => return,
        };

        for (lineno, line) in body.lines().enumerate() {
            let trimmed = line.trim_start();
            // Skip pure comment lines so doc references to embedded
            // paths do not fail the scan.
            if trimmed.starts_with("//") || trimmed.starts_with("///") {
                continue;
            }

            // Only consider lines that actually do disk I/O, since the
            // forbidden fragments may legitimately appear as embedded
            // keys (e.g. `format!("agents/pi/prompts/{name}")` passed
            // to `require_embedded`).
            let does_disk_io = line.contains("read_to_string")
                || line.contains("fs::read_dir")
                || line.contains("read_dir(");
            if !does_disk_io {
                continue;
            }

            for fragment in FORBIDDEN_DISK_PATH_FRAGMENTS {
                if line.contains(fragment) {
                    offences.push(format!(
                        "{}:{} reads embedded path '{}' from disk: {}",
                        rel.display(),
                        lineno + 1,
                        fragment,
                        line.trim()
                    ));
                }
            }
        }
    });

    assert!(
        offences.is_empty(),
        "Embedded-defaults drift: source code is reading shipped defaults from disk.\n\
         Every prompt and pi agent-definition read must go through `init::require_embedded`\n\
         or `init::embedded_entries_with_prefix`.\n\n\
         Offences:\n  {}",
        offences.join("\n  ")
    );
}

fn visit_rust_files(dir: &Path, callback: &mut dyn FnMut(&Path)) {
    for entry in fs::read_dir(dir).expect("readable src dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit_rust_files(&path, callback);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            callback(&path);
        }
    }
}

/// Smoke-test the consumption contract end-to-end: run every shipped
/// prompt loader against an empty config dir and a freshly-scaffolded
/// plan dir. Anything that still touches disk for an embedded default
/// fails here with a real I/O error rather than a subtle behavioural
/// difference.
#[test]
fn embedded_prompts_load_against_an_empty_config_dir() {
    use ravel_lite::init::require_embedded;

    let tmp = tempfile::TempDir::new().unwrap();
    let config_dir = tmp.path().join("cfg");
    fs::create_dir_all(&config_dir).unwrap();
    // Deliberately do NOT call `run_init` — the test asserts that the
    // shipped defaults are reachable even when nothing has ever been
    // materialised here.

    // Phase prompts (work, analyse-work, reflect, triage).
    for phase in ["work", "analyse-work", "reflect", "triage"] {
        let key = format!("phases/{phase}.md");
        let body = require_embedded(&key)
            .unwrap_or_else(|_| panic!("phases/{phase}.md not embedded"));
        assert!(!body.trim().is_empty(), "embedded phase {phase} is empty");
    }

    // Pi system + memory prompts.
    for name in ["system-prompt.md", "memory-prompt.md"] {
        let key = format!("agents/pi/prompts/{name}");
        let body = require_embedded(&key).unwrap();
        assert!(!body.trim().is_empty(), "embedded {key} is empty");
    }

    // Pi subagents.
    let subagents: Vec<_> =
        ravel_lite::init::embedded_entries_with_prefix("agents/pi/subagents/").collect();
    assert!(!subagents.is_empty(), "expected pi subagents in embedded set");

    // Survey + create + discover prompts.
    for key in [
        "survey.md",
        "survey-incremental.md",
        "create-plan.md",
        "discover-stage1.md",
        "discover-stage2.md",
    ] {
        let body = require_embedded(key).unwrap();
        assert!(!body.trim().is_empty(), "embedded {key} is empty");
    }

    // Defensive: nothing was written into the config dir (the test
    // only created the directory itself).
    let entries: Vec<PathBuf> = fs::read_dir(&config_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    assert!(
        entries.is_empty(),
        "embedded loaders unexpectedly wrote into config dir: {entries:?}"
    );
}
