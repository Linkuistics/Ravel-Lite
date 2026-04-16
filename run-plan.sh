#!/usr/bin/env bash
# Usage: run-plan.sh <plan-dir>
#
# Drives the four-phase work cycle for a backlog plan, using pi
# (@mariozechner/pi-coding-agent) as the LLM harness.

set -eu

# -----------------------------------------------------------------------------
# Self-location and configuration
# -----------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LLM_CONTEXT_PI_DIR="$SCRIPT_DIR"

# Defensive defaults — overridden by config.sh below if present.
HEADROOM=1500
PROVIDER="anthropic"
WORK_MODEL=""
REFLECT_MODEL=""
COMPACT_MODEL=""
TRIAGE_MODEL=""
WORK_THINKING=""
REFLECT_THINKING=""
COMPACT_THINKING=""
TRIAGE_THINKING=""

if [ -f "$LLM_CONTEXT_PI_DIR/config.sh" ]; then
    # shellcheck source=/dev/null
    . "$LLM_CONTEXT_PI_DIR/config.sh"
fi

# -----------------------------------------------------------------------------
# format_pi_stream — turn pi's --mode json JSONL into a readable trace
# showing tool calls and final assistant text. Requires jq.
# -----------------------------------------------------------------------------

format_pi_stream() {
    jq -j --unbuffered '
        def tool_summary:
          .toolName as $n |
          (.args // {}) as $a |
          "\n→ " + $n +
          (if $n == "read" or $n == "write" or $n == "edit" then
             (if $a.path then " " + $a.path
              elif $a.file_path then " " + $a.file_path
              else "" end)
           elif $n == "find" then
             (if $a.pattern then " " + $a.pattern else "" end)
             + (if $a.path then " (in " + $a.path + ")" else "" end)
           elif $n == "grep" then
             (if $a.pattern then " /" + $a.pattern + "/" else "" end)
             + (if $a.path then " in " + $a.path else "" end)
           elif $n == "ls" then
             (if $a.path then " " + $a.path else "" end)
           elif $n == "bash" then
             (if $a.command then
                " " + ($a.command | gsub("\n"; " ⏎ ")
                                  | if length > 120 then .[0:117] + "…" else . end)
              else "" end)
           else "" end)
          + "\n";

        if .type == "tool_execution_start" then
            tool_summary
        elif .type == "message_end" then
            (.message.content // []
             | map(select(.type == "text") | .text)
             | join(""))
        elif .type == "tool_execution_end" and (.isError == true) then
            "\n[tool error: " + (.toolName // "?") + "]\n"
        else empty end
    '
}

# -----------------------------------------------------------------------------
# parse_propagation — read propagation.out.yaml on stdin, emit one
# tab-separated line per entry:
#   <kind>\t<target>\t<summary-joined-to-one-line>
# -----------------------------------------------------------------------------

parse_propagation() {
    awk '
        function flush() {
            if (have_entry) {
                gsub(/\t/, " ", summary)
                gsub(/[[:space:]]+$/, "", summary)
                printf "%s\t%s\t%s\n", kind, target, summary
            }
            target = ""; kind = ""; summary = ""
            in_summary = 0; summary_indent = -1
            have_entry = 0
        }
        BEGIN { have_entry = 0; in_summary = 0 }
        /^[[:space:]]*$/ {
            if (in_summary && summary != "") summary = summary " "
            next
        }
        /^propagations:[[:space:]]*$/ { next }
        /^[[:space:]]*-[[:space:]]+target:/ {
            flush()
            sub(/^[[:space:]]*-[[:space:]]+target:[[:space:]]*/, "")
            target = $0
            have_entry = 1
            next
        }
        /^[[:space:]]+kind:/ {
            sub(/^[[:space:]]+kind:[[:space:]]*/, "")
            kind = $0
            next
        }
        /^[[:space:]]+summary:[[:space:]]*\|[[:space:]]*$/ {
            in_summary = 1
            summary_indent = -1
            next
        }
        {
            if (in_summary) {
                line = $0
                if (summary_indent == -1) {
                    match(line, /^[[:space:]]*/)
                    summary_indent = RLENGTH
                }
                if (length(line) >= summary_indent) {
                    line = substr(line, summary_indent + 1)
                }
                if (summary == "") summary = line
                else summary = summary " " line
            }
        }
        END { flush() }
    '
}

# -----------------------------------------------------------------------------
# list_plans_in — discover plan directories under a project's LLM_STATE/ tree.
# -----------------------------------------------------------------------------

list_plans_in() {
    local project_root="$1"
    if [ ! -d "$project_root/LLM_STATE" ]; then
        return 0
    fi
    find "$project_root/LLM_STATE" -type f -name phase.md -print 2>/dev/null | \
        while IFS= read -r phase_file; do
            dirname "$phase_file"
        done
}

# -----------------------------------------------------------------------------
# parse_related_projects — extract entries from related-plans.md sections.
# -----------------------------------------------------------------------------

parse_related_projects() {
    local file="$1"
    local section="$2"
    if [ ! -f "$file" ]; then
        return 0
    fi
    awk -v section="$section" '
        /^## / {
            if (tolower($0) ~ tolower(section)) {
                in_section = 1
            } else {
                in_section = 0
            }
            next
        }
        in_section && /^- / {
            line = $0
            sub(/^- /, "", line)
            sub(/ [—-].*$/, "", line)
            sub(/[[:space:]]+$/, "", line)
            print line
        }
    ' "$file"
}

# -----------------------------------------------------------------------------
# build_related_plans — synthesize the {{RELATED_PLANS}} block for a plan.
# -----------------------------------------------------------------------------

build_related_plans() {
    local plan_dir="$1"
    local project_root="$2"
    local dev_root="$3"

    local siblings=()
    local parents=()
    local children=()

    while IFS= read -r p; do
        if [ -n "$p" ] && [ "$p" != "$plan_dir" ]; then
            siblings+=("$p")
        fi
    done < <(list_plans_in "$project_root")

    local related_file="$plan_dir/related-plans.md"
    while IFS= read -r proj_entry; do
        if [ -z "$proj_entry" ]; then continue; fi
        local proj_path
        proj_path="${proj_entry//\{\{DEV_ROOT\}\}/$dev_root}"
        while IFS= read -r p; do
            if [ -n "$p" ]; then
                parents+=("$p")
            fi
        done < <(list_plans_in "$proj_path")
    done < <(parse_related_projects "$related_file" "Parents")

    while IFS= read -r proj_entry; do
        if [ -z "$proj_entry" ]; then continue; fi
        local proj_path
        proj_path="${proj_entry//\{\{DEV_ROOT\}\}/$dev_root}"
        while IFS= read -r p; do
            if [ -n "$p" ]; then
                children+=("$p")
            fi
        done < <(list_plans_in "$proj_path")
    done < <(parse_related_projects "$related_file" "Children")

    if [ ${#siblings[@]} -eq 0 ] && [ ${#parents[@]} -eq 0 ] && [ ${#children[@]} -eq 0 ]; then
        echo "Related plans: (none)"
        return 0
    fi

    echo "Related plans:"
    echo ""
    if [ ${#siblings[@]} -gt 0 ]; then
        echo "Siblings (same project):"
        for p in "${siblings[@]}"; do echo "- $p"; done
        echo ""
    fi
    if [ ${#parents[@]} -gt 0 ]; then
        echo "Parents (from declared peer projects):"
        for p in "${parents[@]}"; do echo "- $p"; done
        echo ""
    fi
    if [ ${#children[@]} -gt 0 ]; then
        echo "Children (from declared peer projects):"
        for p in "${children[@]}"; do echo "- $p"; done
        echo ""
    fi
}

# -----------------------------------------------------------------------------
# Main loop placeholder (guarded — only runs when executed directly)
# -----------------------------------------------------------------------------

if [ "${BASH_SOURCE[0]:-}" = "${0:-}" ]; then
    echo "run-plan.sh: main loop not yet implemented" >&2
    exit 1
fi
