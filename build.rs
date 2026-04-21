// Emit build-time metadata for the `--version` / `version` output so a
// user staring at a misbehaving binary can answer "which build is this?"
// without guessing from file mtimes.
//
// Three env vars reach the compiled binary via `env!()`:
//   BUILD_TIMESTAMP — UTC ISO-8601 at compile time
//   GIT_DESCRIBE    — `git describe --tags --always --dirty` output
//   GIT_SHA         — short commit hash
//
// All three fall back to "unknown" when their source isn't available
// (e.g. building from a tarball where `.git` is absent). No new runtime
// deps; the build script shells out to `date` and `git`.

use std::process::Command;

fn main() {
    println!("cargo:rustc-env=BUILD_TIMESTAMP={}", utc_timestamp());
    println!("cargo:rustc-env=GIT_DESCRIBE={}", git_describe());
    println!("cargo:rustc-env=GIT_SHA={}", git_short_sha());
    // Recompile the version metadata when HEAD moves or a tag is
    // created — otherwise `cargo build` would cache the first value
    // and GIT_DESCRIBE would stale out.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");
    println!("cargo:rerun-if-changed=.git/refs/tags");
}

fn utc_timestamp() -> String {
    run(Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"]))
        .unwrap_or_else(|| "unknown".into())
}

fn git_describe() -> String {
    run(Command::new("git").args(["describe", "--tags", "--always", "--dirty"]))
        .unwrap_or_else(|| "unknown".into())
}

fn git_short_sha() -> String {
    run(Command::new("git").args(["rev-parse", "--short", "HEAD"]))
        .unwrap_or_else(|| "unknown".into())
}

fn run(cmd: &mut Command) -> Option<String> {
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8(output.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
