#!/usr/bin/env python3
"""
Forbid `.with_context(...)` and `.context(...)` chains under `src/**`
that do not chain `.with_code(ErrorCode::X)` within the same expression.

The convention (see `src/cli/error_context.rs`) is that every fallible
call-site attaches an ErrorCode. Untagged chains regress the JSON
envelope and exit-category to the catch-all `ErrorCode::Internal`.

A chain is "the same expression" — the run from the `.with_context(`
or `.context(` opener up to the next `;` at paren depth 0, or the next
unmatched closing `}` at brace depth 0 (function-tail expressions like
`src/repos.rs:136-138` have no terminating `;`).

Chains carrying an `// errorcode-exempt: <reason>` comment anywhere in
the matched extent are skipped — same vocabulary as the existing
`bail!/anyhow!/ensure!` guard in `scripts/check.sh`.

Exits 0 if clean, 1 if any violations remain.
"""
import bisect
import os
import re
import sys

CHAIN = re.compile(r"\.(with_context|context)\(")
WITH_CODE = re.compile(r"\.with_code\(")
EXEMPT = "errorcode-exempt:"


def sanitize(text: str) -> str:
    """Return `text` with comment and string-literal contents replaced
    by spaces, preserving newlines and absolute positions so the result
    can be scanned for tokens while line numbers and paren-depth still
    line up with the original source.

    Handles: `//` line comments, `/* */` block comments, double-quoted
    strings (with backslash escapes), and `'x'` char literals.
    Lifetimes (`'a`, `'static`) are left intact — they are syntactic
    identifiers, not delimiters.
    """
    out = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                out.append(" ")
                i += 1
            continue
        if c == "/" and i + 1 < n and text[i + 1] == "*":
            out.append("  ")
            i += 2
            while i + 1 < n and not (text[i] == "*" and text[i + 1] == "/"):
                out.append("\n" if text[i] == "\n" else " ")
                i += 1
            if i + 1 < n:
                out.append("  ")
                i += 2
            continue
        if c == '"':
            out.append('"')
            i += 1
            while i < n and text[i] != '"':
                if text[i] == "\\" and i + 1 < n:
                    out.append("  ")
                    i += 2
                    continue
                out.append("\n" if text[i] == "\n" else " ")
                i += 1
            if i < n:
                out.append('"')
                i += 1
            continue
        if c == "'":
            # Disambiguate char literal from lifetime by looking for a
            # closing quote within ~3 chars after the identifier-ish run.
            j = i + 1
            if j < n and text[j] == "\\":
                j += 2
            else:
                while j < n and (text[j].isalnum() or text[j] == "_"):
                    j += 1
            if j < n and text[j] == "'":
                for k in range(i, j + 1):
                    out.append("\n" if text[k] == "\n" else " ")
                i = j + 1
                continue
        out.append(c)
        i += 1
    return "".join(out)


def chain_extent(sanitized: str, start: int) -> int:
    """Return the position one past the end of the expression-statement
    that begins at `sanitized[start]` (the `.` of `.with_context(` or
    `.context(`). The end is the next `;` at paren+brace depth 0, or
    the next unmatched closing `}` at brace depth 0.
    """
    paren = 0
    brace = 0
    i = start
    n = len(sanitized)
    while i < n:
        c = sanitized[i]
        if c in "([":
            paren += 1
        elif c in ")]":
            paren -= 1
        elif c == "{":
            brace += 1
        elif c == "}":
            if brace == 0 and paren == 0:
                return i
            brace -= 1
        elif c == ";" and paren == 0 and brace == 0:
            return i + 1
        i += 1
    return n


def line_starts_of(text: str) -> list[int]:
    starts = [0]
    for i, c in enumerate(text):
        if c == "\n":
            starts.append(i + 1)
    return starts


def line_no(starts: list[int], pos: int) -> int:
    return bisect.bisect_right(starts, pos)


def scan_file(path: str) -> list[tuple[str, int, str]]:
    text = open(path, encoding="utf-8").read()
    sanitized = sanitize(text)
    starts = line_starts_of(text)
    lines = text.splitlines()
    found = []
    for m in CHAIN.finditer(sanitized):
        start = m.start()
        end = chain_extent(sanitized, start)
        sanitized_extent = sanitized[start:end]
        if WITH_CODE.search(sanitized_extent):
            continue
        ln = line_no(starts, start)
        line_text = lines[ln - 1] if ln - 1 < len(lines) else ""
        # Marker may appear (a) inside the chain extent — typical for
        # multi-line chains where the trailing `?;` is on its own line,
        # or (b) on the opener's line itself, after the statement
        # terminator — the convention used by the bail!/anyhow!/ensure!
        # guard that this check parallels.
        if EXEMPT in text[start:end] or EXEMPT in line_text:
            continue
        found.append((path, ln, line_text.strip()))
    return found


def main() -> int:
    violations: list[tuple[str, int, str]] = []
    for root, _, files in os.walk("src"):
        for fn in files:
            if fn.endswith(".rs"):
                violations.extend(scan_file(os.path.join(root, fn)))
    if not violations:
        return 0
    print(
        "check: untagged .with_context(/.context( chains found under src/**.",
        file=sys.stderr,
    )
    print(
        "       Chain .with_code(ErrorCode::X) onto the chain (typically before `?`).",
        file=sys.stderr,
    )
    print(
        "       For unavoidable cases, append '// errorcode-exempt: <reason>'",
        file=sys.stderr,
    )
    print("       inside the chain extent.", file=sys.stderr)
    print("", file=sys.stderr)
    for p, ln, line in violations:
        print(f"{p}:{ln}: {line}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
