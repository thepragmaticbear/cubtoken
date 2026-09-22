#!/usr/bin/env python3
"""Independent check for task t2: does TRACE.md describe the real call chain?

Checks that the write-up names each link by identifier and puts them in call
order. The agent is not told which identifiers are checked.
"""
import re
import sys

# In the order they are reached, from hook stdin to the pass-through decision.
# These identifiers must all exist at the revision the task pins (fc6d7d5):
# a chain written against a newer tree makes the task unpassable.
CHAIN = [
    ("run_hook", "the hook entry point"),
    ("dispatch", "per-tool dispatch"),
    ("ledger_for", "opening the session ledger for this payload"),
    ("is_protected", "the protection lookup that forces pass-through"),
]
# The persistence side: recorded on Edit/Write/NotebookEdit, replayed on open.
PERSISTENCE = ["note_edit"]
FILES = ["lib.rs", "ledger.rs"]


def main(path):
    try:
        with open(path, encoding="utf-8") as handle:
            text = handle.read()
    except OSError as error:
        print(f"FAIL cannot read {path}: {error}", file=sys.stderr)
        return 1

    failures = []
    positions = {}
    for name, description in CHAIN + [(n, "persistence") for n in PERSISTENCE]:
        match = re.search(rf"\b{re.escape(name)}\b", text)
        if match is None:
            failures.append(f"TRACE.md never names {name} ({description})")
        else:
            positions[name] = match.start()

    for name in FILES:
        if name not in text:
            failures.append(f"TRACE.md never names the file {name}")

    ordered = [name for name, _ in CHAIN if name in positions]
    for earlier, later in zip(ordered, ordered[1:]):
        if positions[earlier] > positions[later]:
            failures.append(
                f"{earlier} is described after {later}; the chain is out of call order"
            )

    for failure in failures:
        print(f"FAIL {failure}", file=sys.stderr)
    if failures:
        return 1
    print("ok TRACE.md names the full chain in call order")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1] if len(sys.argv) > 1 else "TRACE.md"))
