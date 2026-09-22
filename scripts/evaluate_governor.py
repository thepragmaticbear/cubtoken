#!/usr/bin/env python3
"""Validate exported governor evaluation runs and render a Markdown report.

Read-only by contract: this script launches nothing, installs nothing, changes
no configuration, and never writes to its inputs. It reads JSONL run records
from explicit paths and writes one report to an explicit output path.

Run `--self-test` for the built-in checks. Python standard library only.

Record shape is documented in `tests/evaluation/README.md`; `SCHEMA_VERSION`
below is the version this script accepts.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path

SCHEMA_VERSION = 1

ARMS = ("disabled", "static", "observe", "safe")
OUTCOMES = ("pass", "fail", "timeout", "infrastructure_error")

# How a provider reports prompt tokens. Getting this wrong is the difference
# between a real saving and a doubled cache read, so the record must say which
# applies and an unrecognised value is a data error rather than a guess.
#
#   components_additive  input_tokens EXCLUDES cached tokens; the total is
#                        input + cache_creation + cache_read.
#                        (Anthropic Messages API `usage`.)
#   total_inclusive      input_tokens ALREADY includes cached reads; cache_read
#                        is a subset kept for reporting only. Adding it again
#                        double-counts.
INPUT_ACCOUNTING = ("components_additive", "total_inclusive")

IDENTITY_FIELDS = (
    "schema_version",
    "run_id",
    "task_id",
    "repetition",
    "arm",
    "source_revision",
    "cubtoken_revision",
    "config_hash",
    "checkpoint_hash",
)
ENVIRONMENT_FIELDS = (
    "host_version",
    "model",
    "model_settings",
    "started_at_utc",
    "elapsed_ms",
    "time_limit_ms",
)
OUTCOME_FIELDS = (
    "status",
    "check_command",
    "check_exit_status",
    "evidence_path",
)
USAGE_FIELDS = (
    "input_accounting",
    "input_tokens",
    "cache_creation_input_tokens",
    "cache_read_input_tokens",
    "output_tokens",
    "artifact_path",
    "field_mapping",
)
GOVERNOR_FIELDS = (
    "status_before_path",
    "status_after_path",
    "stats_before_path",
    "stats_after_path",
    "gross_savings_tokens",
    "attributed_recovery_tokens",
    "attributed_recovery_events",
    "legacy_refetch_count",
    "policy_applied",
    # Read by the integrity and coverage passes. Absent, the "this arm ran as
    # static" warning silently never fires — and that warning is the whole
    # reason `ctk adaptive status` reports the field.
    "learning_enabled",
    "policy_evidence",
)

# Token counts and durations: never negative, never a bool, never a string.
NON_NEGATIVE_FIELDS = (
    ("usage", "input_tokens"),
    ("usage", "cache_creation_input_tokens"),
    ("usage", "cache_read_input_tokens"),
    ("usage", "output_tokens"),
    ("environment", "elapsed_ms"),
    ("environment", "time_limit_ms"),
    ("governor", "gross_savings_tokens"),
    ("governor", "attributed_recovery_tokens"),
    ("governor", "attributed_recovery_events"),
    ("governor", "legacy_refetch_count"),
)


class DataError(Exception):
    """A defect in the run records themselves, not in the thing measured."""


# --------------------------------------------------------------------------
# Loading and validation
# --------------------------------------------------------------------------


def load_records(paths):
    """Parse every JSONL line under `paths`. Returns (records, errors)."""
    records, errors = [], []
    for path in paths:
        path = Path(path)
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as error:
            errors.append(f"{path}: cannot read ({error})")
            continue
        for number, line in enumerate(text.splitlines(), start=1):
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            try:
                record = json.loads(line)
            except json.JSONDecodeError as error:
                errors.append(f"{path}:{number}: not valid JSON ({error})")
                continue
            if not isinstance(record, dict):
                errors.append(f"{path}:{number}: record is not an object")
                continue
            record["_origin"] = f"{path}:{number}"
            records.append(record)
    return records, errors


def _missing(container, fields, where, origin):
    out = []
    if not isinstance(container, dict):
        return [f"{origin}: {where} is missing or not an object"]
    for field in fields:
        if field not in container:
            out.append(f"{origin}: {where}.{field} is missing")
    return out


def validate_record(record):
    """Structural validation of one record. Returns a list of error strings."""
    origin = record.get("_origin", record.get("run_id", "<unknown>"))
    errors = []
    for field in IDENTITY_FIELDS:
        if field not in record:
            errors.append(f"{origin}: {field} is missing")
    errors += _missing(record.get("environment"), ENVIRONMENT_FIELDS, "environment", origin)
    errors += _missing(record.get("outcome"), OUTCOME_FIELDS, "outcome", origin)
    errors += _missing(record.get("usage"), USAGE_FIELDS, "usage", origin)
    errors += _missing(record.get("governor"), GOVERNOR_FIELDS, "governor", origin)
    if errors:
        return errors

    if record["schema_version"] != SCHEMA_VERSION:
        errors.append(
            f"{origin}: schema_version {record['schema_version']!r} is not the "
            f"supported version {SCHEMA_VERSION}"
        )
    if record["arm"] not in ARMS:
        errors.append(f"{origin}: arm {record['arm']!r} is not one of {ARMS}")
    status = record["outcome"]["status"]
    if status not in OUTCOMES:
        errors.append(f"{origin}: outcome.status {status!r} is not one of {OUTCOMES}")
    accounting = record["usage"]["input_accounting"]
    if accounting not in INPUT_ACCOUNTING:
        errors.append(
            f"{origin}: usage.input_accounting {accounting!r} is ambiguous; "
            f"expected one of {INPUT_ACCOUNTING}"
        )
    if not record["usage"].get("field_mapping"):
        errors.append(
            f"{origin}: usage.field_mapping must document how the host's usage "
            "result was mapped onto these fields"
        )
    if not isinstance(record["repetition"], int) or isinstance(record["repetition"], bool):
        errors.append(f"{origin}: repetition must be an integer")
    for group, field in NON_NEGATIVE_FIELDS:
        value = record[group].get(field)
        if value is None:
            continue
        if isinstance(value, bool) or not isinstance(value, int):
            errors.append(
                f"{origin}: {group}.{field} must be an integer or null, got {value!r}"
            )
        elif value < 0:
            errors.append(f"{origin}: {group}.{field} is negative ({value})")
    return errors


def check_cohort(records, expected=None):
    """Cross-record integrity. Mixed revisions and duplicates are data errors.

    `expected` is the matrix from `tasks.json`. Without it the expected runs can
    only be inferred from the records present, which cannot notice that a whole
    repetition or task never arrived.
    """
    errors, warnings = [], []

    seen = {}
    for record in records:
        run_id = record["run_id"]
        if run_id in seen:
            errors.append(
                f"duplicate run_id {run_id!r} ({seen[run_id]} and {record['_origin']}); "
                "run records must be unique"
            )
        seen[run_id] = record["_origin"]

    cells = defaultdict(list)
    for record in records:
        cells[(record["task_id"], record["repetition"], record["arm"])].append(record)
    for key, group in sorted(cells.items()):
        if len(group) > 1:
            errors.append(
                f"task {key[0]} repetition {key[1]} arm {key[2]} has "
                f"{len(group)} records; expected exactly one"
            )

    accounting = {record["usage"]["input_accounting"] for record in records}
    if len(accounting) > 1:
        errors.append(
            "records mix usage accounting modes "
            f"({sorted(accounting)}); the mode is a property of the provider, so a "
            "mixed cohort compares totals that were never measured the same way"
        )

    revisions = {record["cubtoken_revision"] for record in records}
    if len(revisions) > 1:
        errors.append(
            "records span multiple cubtoken revisions "
            f"({sorted(revisions)}); a scored comparison must name one immutable "
            "implementation"
        )

    per_task = defaultdict(set)
    for record in records:
        per_task[record["task_id"]].add(record["source_revision"])
    for task_id, found in sorted(per_task.items()):
        if len(found) > 1:
            errors.append(
                f"task {task_id} was run against multiple source revisions "
                f"({sorted(found)}); results cannot be pooled"
            )

    # A configuration that cannot adapt invalidates the arm's label rather than
    # the data: report it loudly but let the reader see the numbers.
    for record in records:
        governor = record["governor"]
        if record["arm"] in ("observe", "safe") and governor.get("learning_enabled") is False:
            warnings.append(
                f"{record['run_id']}: arm is {record['arm']!r} but the recorded "
                "configuration reports learning_enabled=false (statistics off); "
                "this arm ran as static"
            )

    if expected is None:
        tasks = sorted({record["task_id"] for record in records})
        repetitions = sorted({record["repetition"] for record in records})
        arms = ARMS
        warnings.append(
            "no expected matrix given (--expect-matrix): missing runs can only be "
            "detected within the tasks and repetitions that arrived, so a wholly "
            "absent task or repetition will not be reported"
        )
    else:
        tasks, repetitions, arms = expected["tasks"], expected["repetitions"], expected["arms"]
        for record in records:
            if record["task_id"] not in tasks:
                warnings.append(
                    f"{record['run_id']}: task {record['task_id']} is not in the manifest"
                )
    for task_id in tasks:
        for repetition in repetitions:
            for arm in arms:
                if (task_id, repetition, arm) not in cells:
                    warnings.append(
                        f"missing run: task {task_id} repetition {repetition} arm {arm}"
                    )
    return errors, warnings


# --------------------------------------------------------------------------
# Normalization
# --------------------------------------------------------------------------


def normalized_input_tokens(usage):
    """Total prompt tokens under the record's declared semantics.

    Returns (tokens, reason_unavailable). A missing component is never treated
    as zero: the answer is None and the run drops out of token comparisons.
    """
    accounting = usage.get("input_accounting")
    base = usage.get("input_tokens")
    if accounting not in INPUT_ACCOUNTING:
        return None, "input_accounting is ambiguous"
    if base is None:
        return None, "input_tokens is null"
    if accounting == "total_inclusive":
        # cache_read is a subset of base here. Adding it would double-count.
        return base, None
    total = base
    for field in ("cache_creation_input_tokens", "cache_read_input_tokens"):
        value = usage.get(field)
        if value is None:
            return None, f"{field} is null and cannot be assumed zero"
        total += value
    return total, None


def task_tokens(record):
    """`normalized_total_input_tokens + total_output_tokens`, or None."""
    usage = record["usage"]
    prompt, reason = normalized_input_tokens(usage)
    if prompt is None:
        return None, reason
    output = usage.get("output_tokens")
    if output is None:
        return None, "output_tokens is null"
    return prompt + output, None


def estimated_net_context_saved(record):
    """Gross savings less attributed recovery. May be negative; an estimate.

    Computed from components because the CLI's net total saturates at zero and
    so cannot express a loss.
    """
    governor = record["governor"]
    gross = governor.get("gross_savings_tokens")
    recovery = governor.get("attributed_recovery_tokens")
    if gross is None or recovery is None:
        return None
    return gross - recovery


# --------------------------------------------------------------------------
# Analysis
# --------------------------------------------------------------------------


def load_expected_matrix(path):
    """Tasks, repetitions, and arms the manifest says should exist."""
    manifest = json.loads(Path(path).read_text(encoding="utf-8"))
    return {
        "tasks": sorted(task["task_id"] for task in manifest["tasks"]),
        "repetitions": list(range(1, int(manifest["repetitions"]) + 1)),
        "arms": tuple(manifest.get("arms", ARMS)),
    }


def analyze(records, expected=None):
    """Build the report model. Raises DataError when records cannot be pooled."""
    errors = []
    for record in records:
        errors += validate_record(record)
    if errors:
        raise DataError("\n".join(errors))
    cohort_errors, warnings = check_cohort(records, expected)
    if cohort_errors:
        raise DataError("\n".join(cohort_errors))

    by_cell = {}
    for record in records:
        tokens, reason = task_tokens(record)
        by_cell[(record["task_id"], record["repetition"], record["arm"])] = {
            "record": record,
            "task_tokens": tokens,
            "unavailable": reason,
            "net_context": estimated_net_context_saved(record),
        }

    tasks = sorted({record["task_id"] for record in records})
    repetitions = sorted({record["repetition"] for record in records})

    # Every attempt, every arm, including failures. Reported before any
    # comparison so excluded runs stay visible.
    outcomes = []
    for task_id in tasks:
        for repetition in repetitions:
            for arm in ARMS:
                cell = by_cell.get((task_id, repetition, arm))
                if cell is None:
                    continue
                record = cell["record"]
                outcomes.append(
                    {
                        "task_id": task_id,
                        "repetition": repetition,
                        "arm": arm,
                        "run_id": record["run_id"],
                        "status": record["outcome"]["status"],
                        "exit_status": record["outcome"]["check_exit_status"],
                        "elapsed_ms": record["environment"]["elapsed_ms"],
                        "task_tokens": cell["task_tokens"],
                        "unavailable": cell["unavailable"],
                        "exclusion_reason": record["outcome"].get("exclusion_reason"),
                    }
                )

    # Totals across ALL attempts, so a comparison over successes alone cannot
    # conceal a regression paid for in failed runs.
    totals = {}
    for arm in ARMS:
        cells = [c for key, c in by_cell.items() if key[2] == arm]
        if not cells:
            continue
        counted = [c["task_tokens"] for c in cells if c["task_tokens"] is not None]
        statuses = defaultdict(int)
        for cell in cells:
            statuses[cell["record"]["outcome"]["status"]] += 1
        usage_rows = [c["record"]["usage"] for c in cells]
        totals[arm] = {
            "attempts": len(cells),
            "statuses": dict(statuses),
            "task_tokens": sum(counted) if counted else None,
            "runs_with_usage": len(counted),
            "runs_without_usage": len(cells) - len(counted),
            "input_tokens": _sum_field(usage_rows, "input_tokens"),
            "output_tokens": _sum_field(usage_rows, "output_tokens"),
            "cache_read": _sum_field(usage_rows, "cache_read_input_tokens"),
            "cache_creation": _sum_field(usage_rows, "cache_creation_input_tokens"),
            "elapsed_ms": sum(
                c["record"]["environment"]["elapsed_ms"]
                for c in cells
                if c["record"]["environment"]["elapsed_ms"] is not None
            ),
        }

    comparisons = {
        "compression_savings": _matched(by_cell, tasks, repetitions, "disabled", "static"),
        "adaptive_savings": _matched(by_cell, tasks, repetitions, "static", "safe"),
        "observe_versus_static": _matched(by_cell, tasks, repetitions, "static", "observe"),
    }

    coverage = []
    for task_id in tasks:
        for repetition in repetitions:
            for arm in ("observe", "safe"):
                cell = by_cell.get((task_id, repetition, arm))
                if cell is None:
                    continue
                governor = cell["record"]["governor"]
                coverage.append(
                    {
                        "task_id": task_id,
                        "repetition": repetition,
                        "arm": arm,
                        "policy_applied": governor.get("policy_applied"),
                        "evidence": governor.get("policy_evidence"),
                        "gross": governor.get("gross_savings_tokens"),
                        "recovery_tokens": governor.get("attributed_recovery_tokens"),
                        "recovery_events": governor.get("attributed_recovery_events"),
                        "legacy_refetches": governor.get("legacy_refetch_count"),
                        "net_context": cell["net_context"],
                    }
                )

    safe_applied = [row for row in coverage if row["arm"] == "safe" and row["policy_applied"]]
    if not safe_applied:
        warnings.append(
            "no safe-mode run reports policy_applied=true: the policy comparison "
            "is inconclusive, not a zero-cost result"
        )

    return {
        "records": records,
        "warnings": warnings,
        "outcomes": outcomes,
        "totals": totals,
        "comparisons": comparisons,
        "coverage": coverage,
        "tasks": tasks,
        "repetitions": repetitions,
        "cubtoken_revision": records[0]["cubtoken_revision"],
    }


def _sum_field(usage_rows, field):
    values = [row.get(field) for row in usage_rows]
    present = [value for value in values if value is not None]
    if not present:
        return None
    return sum(present)


def _elapsed_difference(baseline, treatment):
    """Signed elapsed difference, or None when either side did not record one."""
    first = baseline["record"]["environment"]["elapsed_ms"]
    second = treatment["record"]["environment"]["elapsed_ms"]
    if first is None or second is None:
        return None
    return first - second


def _matched(by_cell, tasks, repetitions, baseline_arm, treatment_arm):
    """Signed baseline-minus-treatment differences over comparable pairs.

    A pair is comparable only when both runs exist, both passed, and both
    report usage. A failed run is never a token win: it is listed as skipped.
    """
    pairs, skipped = [], []
    for task_id in tasks:
        for repetition in repetitions:
            baseline = by_cell.get((task_id, repetition, baseline_arm))
            treatment = by_cell.get((task_id, repetition, treatment_arm))
            label = f"{task_id} rep {repetition}"
            if baseline is None or treatment is None:
                skipped.append((label, "one arm has no run"))
                continue
            statuses = (
                baseline["record"]["outcome"]["status"],
                treatment["record"]["outcome"]["status"],
            )
            if statuses != ("pass", "pass"):
                skipped.append(
                    (label, f"{baseline_arm}={statuses[0]}, {treatment_arm}={statuses[1]}")
                )
                continue
            if baseline["task_tokens"] is None or treatment["task_tokens"] is None:
                reason = baseline["unavailable"] or treatment["unavailable"]
                skipped.append((label, f"usage unavailable ({reason})"))
                continue
            difference = baseline["task_tokens"] - treatment["task_tokens"]
            percent = (
                (difference / baseline["task_tokens"] * 100)
                if baseline["task_tokens"]
                else None
            )
            pairs.append(
                {
                    "label": label,
                    "task_id": task_id,
                    "repetition": repetition,
                    "baseline_tokens": baseline["task_tokens"],
                    "treatment_tokens": treatment["task_tokens"],
                    "difference": difference,
                    "percent": percent,
                    "elapsed_difference_ms": _elapsed_difference(baseline, treatment),
                }
            )
    total_difference = sum(pair["difference"] for pair in pairs) if pairs else None
    baseline_total = sum(pair["baseline_tokens"] for pair in pairs) if pairs else None
    return {
        "baseline_arm": baseline_arm,
        "treatment_arm": treatment_arm,
        "pairs": pairs,
        "skipped": skipped,
        "total_difference": total_difference,
        "total_percent": (
            total_difference / baseline_total * 100
            if total_difference is not None and baseline_total
            else None
        ),
    }


# --------------------------------------------------------------------------
# Rendering
# --------------------------------------------------------------------------


def _n(value):
    return "unavailable" if value is None else f"{value:,}"


def _signed(value):
    if value is None:
        return "unavailable"
    return f"{value:+,}"


def render_markdown(report):
    out = []
    add = out.append
    add("# Adaptive governor evaluation report")
    add("")
    add(f"Generated by `scripts/evaluate_governor.py` (record schema v{SCHEMA_VERSION}).")
    add(f"cubtoken revision under test: `{report['cubtoken_revision']}`.")
    add(f"Tasks: {len(report['tasks'])}. Repetitions: {report['repetitions']}.")
    add(f"Run records analysed: {len(report['records'])}.")
    add("")

    add("## Data integrity")
    add("")
    if report["warnings"]:
        for warning in report["warnings"]:
            add(f"- {warning}")
    else:
        add("No integrity warnings.")
    add("")

    add("## Task outcomes, every arm and attempt")
    add("")
    add("| task | rep | arm | run | outcome | check exit | elapsed ms | task tokens |")
    add("| --- | --- | --- | --- | --- | --- | --- | --- |")
    for row in report["outcomes"]:
        tokens = _n(row["task_tokens"])
        if row["unavailable"]:
            tokens = f"unavailable ({row['unavailable']})"
        add(
            f"| {row['task_id']} | {row['repetition']} | {row['arm']} | `{row['run_id']}` "
            f"| {row['status']} | {row['exit_status']} | {_n(row['elapsed_ms'])} | {tokens} |"
        )
    add("")

    add("## Total consumption across all attempts")
    add("")
    add(
        "Includes failed, timed-out, and errored runs. A comparison restricted to "
        "successful pairs cannot conceal a regression paid for here."
    )
    add("")
    add(
        "| arm | attempts | outcomes | task tokens | input | output | cache read "
        "| cache write | runs without usage | elapsed ms |"
    )
    add("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
    for arm in ARMS:
        totals = report["totals"].get(arm)
        if totals is None:
            continue
        statuses = ", ".join(f"{k}={v}" for k, v in sorted(totals["statuses"].items()))
        add(
            f"| {arm} | {totals['attempts']} | {statuses} | {_n(totals['task_tokens'])} "
            f"| {_n(totals['input_tokens'])} | {_n(totals['output_tokens'])} "
            f"| {_n(totals['cache_read'])} | {_n(totals['cache_creation'])} "
            f"| {totals['runs_without_usage']} | {_n(totals['elapsed_ms'])} |"
        )
    add("")

    titles = {
        "compression_savings": "Compression savings (disabled - static)",
        "adaptive_savings": "Adaptive savings (static - safe)",
        "observe_versus_static": "Observe versus static (static - observe)",
    }
    add("## Matched successful comparisons")
    add("")
    add(
        "Only pairs where both arms passed and both reported usage. A positive "
        "difference means the treatment arm used fewer tokens."
    )
    add("")
    for key, title in titles.items():
        comparison = report["comparisons"][key]
        add(f"### {title}")
        add("")
        if comparison["pairs"]:
            add(
                "| pair | baseline tokens | treatment tokens | difference | % "
                "| elapsed difference ms |"
            )
            add("| --- | --- | --- | --- | --- | --- |")
            for pair in comparison["pairs"]:
                percent = (
                    "unavailable" if pair["percent"] is None else f"{pair['percent']:+.1f}%"
                )
                add(
                    f"| {pair['label']} | {_n(pair['baseline_tokens'])} "
                    f"| {_n(pair['treatment_tokens'])} | {_signed(pair['difference'])} "
                    f"| {percent} | {_signed(pair['elapsed_difference_ms'])} |"
                )
            total_percent = (
                "unavailable"
                if comparison["total_percent"] is None
                else f"{comparison['total_percent']:+.1f}%"
            )
            add("")
            count = len(comparison["pairs"])
            add(
                f"**Total over {count} matched pair{'' if count == 1 else 's'}: "
                f"{_signed(comparison['total_difference'])} tokens ({total_percent}).**"
            )
        else:
            add("No comparable pairs. This comparison is unavailable, not zero.")
        if comparison["skipped"]:
            add("")
            add("Skipped pairs:")
            for label, reason in comparison["skipped"]:
                add(f"- {label}: {reason}")
        add("")

    add("## Policy coverage")
    add("")
    add(
        "`estimated net context saved` is gross savings less attributed recovery, "
        "computed from components so it can be negative. It is an estimate from "
        "the hook's character-based sizing, not authoritative usage."
    )
    add("")
    if report["coverage"]:
        add(
            "| task | rep | arm | policy applied | gross saved | attributed recovery "
            "| events | legacy refetches | estimated net context saved | evidence |"
        )
        add("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
        for row in report["coverage"]:
            add(
                f"| {row['task_id']} | {row['repetition']} | {row['arm']} "
                f"| {row['policy_applied']} | {_n(row['gross'])} "
                f"| {_n(row['recovery_tokens'])} | {_n(row['recovery_events'])} "
                f"| {_n(row['legacy_refetches'])} | {_signed(row['net_context'])} "
                f"| {row['evidence'] or '—'} |"
            )
    else:
        add("No observe or safe runs recorded.")
    add("")

    add("## Limitations")
    add("")
    add(
        "- This is an initial diagnostic sample, not a test of statistical "
        "non-inferiority. It identifies regressions; it does not establish "
        "broad superiority."
    )
    add(
        "- Legacy refetch counts are diagnostic only and are never substituted "
        "for attributed recovery cost."
    )
    add(
        "- \"High confidence\" is an attribution category, not a calibrated "
        "probability that compression caused a failure."
    )
    add(
        "- Cache usage is reported as the provider exposed it. Where the provider "
        "offers no way to control caching, it was recorded rather than controlled."
    )
    add("")
    return "\n".join(out)


# --------------------------------------------------------------------------
# Self-test
# --------------------------------------------------------------------------


def _record(run_id, task_id, repetition, arm, **overrides):
    record = {
        "schema_version": SCHEMA_VERSION,
        "run_id": run_id,
        "task_id": task_id,
        "repetition": repetition,
        "arm": arm,
        "source_revision": "task-rev-1",
        "cubtoken_revision": "ctk-rev-1",
        "config_hash": "sha256:cfg",
        "checkpoint_hash": "sha256:ckpt",
        "environment": {
            "host_version": "host-1",
            "model": "model-1",
            "model_settings": "{}",
            "started_at_utc": "2026-09-22T00:00:00Z",
            "elapsed_ms": 1000,
            "time_limit_ms": 600000,
        },
        "outcome": {
            "status": "pass",
            "check_command": "true",
            "check_exit_status": 0,
            "evidence_path": "evidence.log",
            "exclusion_reason": None,
        },
        "usage": {
            "input_accounting": "components_additive",
            "input_tokens": 1000,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
            "output_tokens": 100,
            "artifact_path": "usage.json",
            "field_mapping": "test fixture",
        },
        "governor": {
            "status_before_path": "before.json",
            "status_after_path": "after.json",
            "stats_before_path": "stats-before.json",
            "stats_after_path": "stats-after.json",
            "gross_savings_tokens": 0,
            "attributed_recovery_tokens": 0,
            "attributed_recovery_events": 0,
            "legacy_refetch_count": 0,
            "policy_applied": False,
            "policy_evidence": None,
            "learning_enabled": True,
        },
        "_origin": f"<self-test>:{run_id}",
    }
    for key, value in overrides.items():
        if isinstance(value, dict) and isinstance(record.get(key), dict):
            record[key] = {**record[key], **value}
        else:
            record[key] = value
    return record


def _check(name, condition, detail=""):
    if condition:
        print(f"ok   {name}")
        return True
    print(f"FAIL {name}{(': ' + detail) if detail else ''}", file=sys.stderr)
    return False


def self_test():
    """The five failure modes the analysis must not have. No test framework."""
    passed = []

    # 1. A `total_inclusive` provider already counts cached reads in
    #    input_tokens. Adding cache_read again would double-count.
    inclusive = {
        "input_accounting": "total_inclusive",
        "input_tokens": 10000,
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 9000,
        "output_tokens": 0,
        "artifact_path": "u.json",
        "field_mapping": "test",
    }
    tokens, _ = normalized_input_tokens(inclusive)
    passed.append(
        _check(
            "cache input is not double-counted under total_inclusive",
            tokens == 10000,
            f"got {tokens}, expected 10000",
        )
    )
    additive = {**inclusive, "input_accounting": "components_additive"}
    tokens, _ = normalized_input_tokens(additive)
    passed.append(
        _check(
            "cache input is added under components_additive",
            tokens == 19000,
            f"got {tokens}, expected 19000",
        )
    )
    ambiguous = {**inclusive, "input_accounting": "unstated"}
    tokens, reason = normalized_input_tokens(ambiguous)
    passed.append(
        _check(
            "an ambiguous accounting mapping is rejected",
            tokens is None and reason is not None,
        )
    )

    # 2. A negative saving must survive to the report.
    report = analyze(
        [
            _record("d1", "t1", 1, "disabled", usage={"input_tokens": 1000, "output_tokens": 0}),
            _record("s1", "t1", 1, "static", usage={"input_tokens": 1500, "output_tokens": 0}),
        ]
    )
    comparison = report["comparisons"]["compression_savings"]
    passed.append(
        _check(
            "a negative savings result is reported, not clamped",
            comparison["total_difference"] == -500,
            f"got {comparison['total_difference']}, expected -500",
        )
    )
    rendered = render_markdown(report)
    passed.append(
        _check("the negative result is visible in the report", "-500" in rendered)
    )
    negative_net = estimated_net_context_saved(
        _record(
            "n1",
            "t1",
            1,
            "safe",
            governor={"gross_savings_tokens": 100, "attributed_recovery_tokens": 400},
        )
    )
    passed.append(
        _check(
            "estimated net context saved can go negative",
            negative_net == -300,
            f"got {negative_net}, expected -300",
        )
    )

    # 3. Missing usage is unavailable, never zero.
    report = analyze(
        [
            _record("d2", "t1", 1, "disabled", usage={"input_tokens": 1000, "output_tokens": 0}),
            _record("s2", "t1", 1, "static", usage={"input_tokens": None}),
        ]
    )
    comparison = report["comparisons"]["compression_savings"]
    static_total = report["totals"]["static"]["task_tokens"]
    passed.append(
        _check(
            "a run with missing usage is excluded from matched pairs",
            comparison["pairs"] == [] and len(comparison["skipped"]) == 1,
        )
    )
    passed.append(
        _check(
            "missing usage does not become a zero-token total",
            static_total is None,
            f"got {static_total}, expected None",
        )
    )

    # 4. Duplicate runs are a data error, not silently pooled.
    duplicated = False
    try:
        analyze([_record("dup", "t1", 1, "static"), _record("dup", "t1", 2, "static")])
    except DataError as error:
        duplicated = "duplicate run_id" in str(error)
    passed.append(_check("duplicate run IDs are rejected", duplicated))

    collided = False
    try:
        analyze([_record("a", "t1", 1, "static"), _record("b", "t1", 1, "static")])
    except DataError as error:
        collided = "expected exactly one" in str(error)
    passed.append(_check("two records for one matrix cell are rejected", collided))

    mixed = False
    try:
        analyze(
            [
                _record("a", "t1", 1, "static"),
                _record("b", "t1", 1, "disabled", cubtoken_revision="ctk-rev-2"),
            ]
        )
    except DataError as error:
        mixed = "multiple cubtoken revisions" in str(error)
    passed.append(_check("mixed cubtoken revisions are rejected", mixed))

    # 5. A failed task is never a token win.
    report = analyze(
        [
            _record("d3", "t1", 1, "disabled", usage={"input_tokens": 9000, "output_tokens": 0}),
            _record(
                "s3",
                "t1",
                1,
                "static",
                usage={"input_tokens": 10, "output_tokens": 0},
                outcome={"status": "fail", "check_exit_status": 1},
            ),
        ]
    )
    comparison = report["comparisons"]["compression_savings"]
    passed.append(
        _check(
            "a failed run is not counted as a successful matched comparison",
            comparison["pairs"] == [] and comparison["total_difference"] is None,
        )
    )
    failed_visible = report["totals"]["static"]["statuses"].get("fail") == 1
    passed.append(
        _check("the failed run still appears in total consumption", failed_visible)
    )

    timed_out = analyze(
        [
            _record("d4", "t1", 1, "disabled", usage={"input_tokens": 9000, "output_tokens": 0}),
            _record(
                "s4",
                "t1",
                1,
                "static",
                usage={"input_tokens": 10, "output_tokens": 0},
                outcome={"status": "timeout", "check_exit_status": 124},
            ),
        ]
    )["comparisons"]["compression_savings"]
    passed.append(
        _check("a timed-out run is not a token win", timed_out["pairs"] == [])
    )

    # An inconclusive policy comparison says so.
    report = analyze([_record("s5", "t1", 1, "safe")])
    passed.append(
        _check(
            "a safe arm that never applied the policy is called inconclusive",
            any("inconclusive" in warning for warning in report["warnings"]),
        )
    )

    # An arm whose configuration could not adapt is flagged.
    report = analyze(
        [_record("s6", "t1", 1, "safe", governor={"learning_enabled": False})]
    )
    passed.append(
        _check(
            "a safe arm with statistics off is flagged as having run static",
            any("ran as static" in warning for warning in report["warnings"]),
        )
    )

    # A record missing its field mapping is rejected.
    unmapped = validate_record(_record("u1", "t1", 1, "static", usage={"field_mapping": ""}))
    passed.append(
        _check("a record without a documented field mapping is rejected", bool(unmapped))
    )

    # Regressions for holes an adversarial review walked through. Each of these
    # passed silently before.
    mixed = False
    try:
        analyze(
            [
                _record("m1", "t1", 1, "disabled"),
                _record(
                    "m2", "t1", 1, "static",
                    usage={"input_accounting": "total_inclusive"},
                ),
            ]
        )
    except DataError as error:
        mixed = "mix usage accounting modes" in str(error)
    passed.append(
        _check("a cohort mixing usage accounting modes is rejected", mixed)
    )

    report = analyze(
        [
            _record("e1", "t1", 1, "disabled", environment={"elapsed_ms": None}),
            _record("e2", "t1", 1, "static"),
        ]
    )
    pairs = report["comparisons"]["compression_savings"]["pairs"]
    passed.append(
        _check(
            "a null elapsed_ms yields an unavailable difference, not a crash",
            len(pairs) == 1 and pairs[0]["elapsed_difference_ms"] is None,
        )
    )
    passed.append(
        _check(
            "a report with a null elapsed_ms still renders",
            "unavailable" in render_markdown(report),
        )
    )

    passed.append(
        _check(
            "a negative token count is rejected",
            any(
                "negative" in error
                for error in validate_record(
                    _record("n1", "t1", 1, "static", usage={"input_tokens": -5000})
                )
            ),
        )
    )
    passed.append(
        _check(
            "a string token count is rejected",
            bool(
                validate_record(
                    _record("n2", "t1", 1, "static", usage={"input_tokens": "1000"})
                )
            ),
        )
    )

    stripped = _record("l1", "t1", 1, "safe")
    del stripped["governor"]["learning_enabled"]
    passed.append(
        _check(
            "a record omitting learning_enabled is rejected",
            any("learning_enabled" in error for error in validate_record(stripped)),
        )
    )

    matrix = {"tasks": ["t1"], "repetitions": [1, 2], "arms": ARMS}
    report = analyze([_record(f"r{a}", "t1", 1, a) for a in ARMS], matrix)
    passed.append(
        _check(
            "a wholly missing repetition is reported against the manifest",
            sum("repetition 2" in warning for warning in report["warnings"]) == len(ARMS),
        )
    )
    report = analyze([_record(f"q{a}", "t1", 1, a) for a in ARMS])
    passed.append(
        _check(
            "analysis without an expected matrix says so",
            any("no expected matrix" in warning for warning in report["warnings"]),
        )
    )

    total, failures = len(passed), passed.count(False)
    print(f"\n{total - failures}/{total} self-test checks passed")
    return 0 if failures == 0 else 1


# --------------------------------------------------------------------------
# Entry point
# --------------------------------------------------------------------------


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Validate governor evaluation runs and render a Markdown report."
    )
    parser.add_argument(
        "--runs",
        nargs="+",
        metavar="PATH",
        help="JSONL run-record files (one record per line).",
    )
    parser.add_argument("--out", metavar="PATH", help="Markdown report to write.")
    parser.add_argument(
        "--expect-matrix",
        metavar="PATH",
        help="tests/evaluation/tasks.json — the tasks, repetitions, and arms that "
        "should exist. Without it, a wholly missing task or repetition cannot be "
        "detected.",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="Run the built-in analysis checks and exit.",
    )
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()
    if not args.runs or not args.out:
        parser.error("--runs and --out are required unless --self-test is given")

    records, load_errors = load_records(args.runs)
    if load_errors:
        for error in load_errors:
            print(f"data error: {error}", file=sys.stderr)
        return 2
    if not records:
        print("data error: no run records found", file=sys.stderr)
        return 2
    expected = None
    if args.expect_matrix:
        try:
            expected = load_expected_matrix(args.expect_matrix)
        except (OSError, ValueError, KeyError) as error:
            print(f"data error: cannot read --expect-matrix: {error}", file=sys.stderr)
            return 2
    try:
        report = analyze(records, expected)
    except DataError as error:
        print("data errors — the evaluation must be repaired before the", file=sys.stderr)
        print("results can be interpreted:", file=sys.stderr)
        for line in str(error).splitlines():
            print(f"  {line}", file=sys.stderr)
        return 2

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(render_markdown(report), encoding="utf-8")
    print(f"wrote {out_path} from {len(records)} run records")
    for warning in report["warnings"]:
        print(f"warning: {warning}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
