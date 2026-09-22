#!/usr/bin/env python3
"""Independent check for task t4: was the rename complete and behaviour-preserving?

Run from the task checkout root. `cargo test --workspace` runs separately and
catches behaviour; this catches a rename that compiled only because a call site
was deleted rather than updated.
"""
import pathlib
import re
import sys

OLD = "est_tokens"
NEW = "estimate_tokens"
# Directories that are build output or version control, not source.
SKIP = {"target", ".git", ".cubtoken"}


def sources(root):
    for path in root.rglob("*.rs"):
        if SKIP.intersection(path.parts):
            continue
        yield path


def main():
    root = pathlib.Path(".").resolve()
    failures = []
    stale, renamed = [], []
    for path in sources(root):
        text = path.read_text(encoding="utf-8", errors="replace")
        # `estimate_tokens` contains `est_tokens` as a substring only at a word
        # boundary check, so match the old name as a whole word.
        if re.search(rf"(?<![A-Za-z0-9_]){OLD}\b", text):
            stale.append(str(path.relative_to(root)))
        if re.search(rf"\b{NEW}\b", text):
            renamed.append(str(path.relative_to(root)))

    if stale:
        failures.append(f"the old name {OLD} still appears in: {', '.join(sorted(stale))}")
    if not renamed:
        failures.append(f"the new name {NEW} appears nowhere")

    definition = [p for p in renamed if p.endswith("estimate.rs")]
    if not definition:
        failures.append(f"{NEW} is not defined in crates/ctk-compress/src/estimate.rs")
    if len(set(renamed)) < 2:
        failures.append(
            f"{NEW} appears in only {len(set(renamed))} file; the call sites were not updated"
        )

    for failure in failures:
        print(f"FAIL {failure}", file=sys.stderr)
    if failures:
        return 1
    print(f"ok rename complete across {len(set(renamed))} files, no stale references")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
