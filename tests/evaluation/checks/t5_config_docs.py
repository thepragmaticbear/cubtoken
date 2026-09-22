#!/usr/bin/env python3
"""Independent check for task t5.

The agent must produce CONFIG_AUDIT.md listing every configuration key the
README documents, with its default. README.md is ~16 KB with no tree-sitter
grammar, so it takes head-and-tail elision: the configuration table sits in the
elided middle and can only be reached through the advertised escape hatch.

This asserts the audit against the README's own table, parsed here rather than
trusted from the agent. Run from the task checkout root.
"""
import pathlib
import re
import sys

README = pathlib.Path("README.md")
AUDIT = pathlib.Path("CONFIG_AUDIT.md")

ROW = re.compile(r"^\|\s*`([a-z]+\.[a-z_]+)`\s*\|\s*`([^`]+)`\s*\|")


def readme_table(text):
    """The key/default pairs the README's configuration table states."""
    return {m.group(1): m.group(2).strip() for m in (ROW.match(line) for line in text.splitlines()) if m}


def normalise(value):
    """Compare defaults by content, not incidental punctuation or spacing."""
    return re.sub(r"[\s\"']", "", value).strip().lower()


def main():
    for path in (README, AUDIT):
        if not path.exists():
            print(f"FAIL {path} is missing", file=sys.stderr)
            return 1

    expected = readme_table(README.read_text(encoding="utf-8"))
    if len(expected) < 5:
        print(
            f"FAIL could not parse the README configuration table "
            f"(found {len(expected)} rows); the check is broken, not the agent",
            file=sys.stderr,
        )
        return 1

    audit = AUDIT.read_text(encoding="utf-8")
    failures = []
    for key, default in sorted(expected.items()):
        # Accept `key = value`, `key: value`, or a table row; require both on
        # the same line so a bare mention of the key cannot pass.
        line = next(
            (
                candidate
                for candidate in audit.splitlines()
                if re.search(rf"(?<![\w.]){re.escape(key)}(?![\w])", candidate)
            ),
            None,
        )
        if line is None:
            failures.append(f"CONFIG_AUDIT.md never lists {key}")
            continue
        if normalise(default) not in normalise(line):
            failures.append(
                f"CONFIG_AUDIT.md lists {key} without its default {default!r} "
                f"(line was: {line.strip()!r})"
            )

    listed = set(re.findall(r"(?<![\w.])([a-z]+\.[a-z_]+)(?![\w])", audit))
    invented = {key for key in listed if key not in expected and key.split(".")[0] in {
        "read", "grep", "glob", "bash", "stats"
    }}
    if invented:
        failures.append(
            "CONFIG_AUDIT.md lists keys the README does not document: "
            + ", ".join(sorted(invented))
        )

    for failure in failures:
        print(f"FAIL {failure}", file=sys.stderr)
    if failures:
        return 1
    print(f"ok CONFIG_AUDIT.md accounts for all {len(expected)} documented keys")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
