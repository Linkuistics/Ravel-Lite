//! Terminal-title OSC escape emission.
//!
//! Writes `<project> <plan> <phase>` to the terminal title so users can
//! see at a glance which plan and phase the orchestrator is currently
//! driving. Useful when several ravel-lite sessions share a window
//! manager, or when a session has scrolled the inline phase-header off
//! screen.
//!
//! OSC 0 (`\x1b]0;…\x07`) is the portable "set icon + window title"
//! sequence. When `$TMUX` is set, the OSC is wrapped in tmux's
//! DCS-passthrough envelope (`\x1bPtmux;<body>\x1b\\`) with inner ESCs
//! doubled — without that, tmux swallows the sequence and the outer
//! emulator never sees the update.
//!
//! Writes go to stdout (not stderr). Ratatui owns stderr for its
//! inline viewport, so stdout is a clean side-channel that doesn't
//! interleave with rendered frames.
//!
//! Failure is silent. The title is decorative; a write error
//! (closed tty, unsupported terminal) must never block a phase
//! transition.

use std::io::{self, Write};

/// Build the OSC escape that sets the terminal title to
/// `<project> <plan> <phase>`.
///
/// When `tmux` is true, wraps the OSC in a `\x1bPtmux;…\x1b\\` DCS
/// passthrough envelope with inner ESCs doubled so the outer emulator
/// receives the update through tmux.
///
/// Pure: takes the tmux flag as a parameter rather than reading the
/// environment, so the byte sequence is fully testable.
pub fn format_title_escape(project: &str, plan: &str, phase: &str, tmux: bool) -> String {
    // Uppercase the phase so it reads as a distinct segment against the
    // project/plan names, matching the phase-header banner convention
    // (`phase_info` in `src/format.rs` also returns uppercased labels).
    let body = format!("{project} {plan} {}", phase.to_uppercase());
    let osc = format!("\x1b]0;{body}\x07");
    if tmux {
        // Inside a tmux DCS passthrough, each ESC in the body must be
        // doubled — tmux consumes one and forwards the other.
        let doubled = osc.replace('\x1b', "\x1b\x1b");
        format!("\x1bPtmux;{doubled}\x1b\\")
    } else {
        osc
    }
}

/// Write the terminal-title escape to stdout. Honours `$TMUX` for
/// passthrough wrapping. Best-effort — any I/O error is swallowed so
/// that a non-interactive stdout (CI, redirected output) cannot stall
/// a phase transition.
pub fn set_title(project: &str, plan: &str, phase: &str) {
    let tmux = std::env::var_os("TMUX").is_some();
    let escape = format_title_escape(project, plan, phase, tmux);
    let mut stdout = io::stdout();
    let _ = stdout.write_all(escape.as_bytes());
    let _ = stdout.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_osc_when_not_in_tmux() {
        let s = format_title_escape("ravel-lite", "core", "work", false);
        assert_eq!(s, "\x1b]0;ravel-lite core WORK\x07");
    }

    #[test]
    fn tmux_passthrough_doubles_inner_esc() {
        let s = format_title_escape("ravel-lite", "core", "work", true);
        // Outer: DCS passthrough open + body + ST.
        // Inner body: original OSC with ESC doubled.
        assert_eq!(s, "\x1bPtmux;\x1b\x1b]0;ravel-lite core WORK\x07\x1b\\");
    }

    #[test]
    fn phase_is_uppercased_for_visual_separation() {
        // Multi-word phase stays hyphen-joined, just uppercased.
        let s = format_title_escape("proj", "plan", "analyse-work", false);
        assert!(s.contains("proj plan ANALYSE-WORK"));
    }

    #[test]
    fn body_joins_with_single_spaces() {
        let s = format_title_escape("proj", "plan", "work", false);
        assert!(s.contains("proj plan WORK"));
    }

    #[test]
    fn tmux_open_and_terminator_present() {
        let s = format_title_escape("p", "q", "r", true);
        assert!(s.starts_with("\x1bPtmux;"));
        assert!(s.ends_with("\x1b\\"));
    }

    #[test]
    fn empty_components_do_not_panic() {
        // A transient read of a missing phase.md could hand us an empty
        // phase string; the formatter must handle it without crashing.
        let s = format_title_escape("", "", "", false);
        assert!(s.starts_with("\x1b]0;"));
        assert!(s.ends_with("\x07"));
    }
}
