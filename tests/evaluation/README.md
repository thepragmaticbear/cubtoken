# Adaptive governor evaluation runbook

This directory holds the controlled comparison described in
[ADAPTIVE_GOVERNOR_EVALUATION_SPEC.md](../../ADAPTIVE_GOVERNOR_EVALUATION_SPEC.md).
It answers one question: **does the governor preserve task correctness while
reducing total model token use at acceptable latency?**

| File | What it is |
| --- | --- |
| `tasks.json` | The task manifest: revision, prompt, setup, time limit, and success check per task. |
| `checks/` | The success checks. Copied in *after* the agent stops; the agent never sees them. |
| `RESULTS.md` | The validation receipt: what was measured, what passed, what is still unknown. |
| `../../scripts/evaluate_governor.py` | Record validation and report generation. Read-only. |

Live task execution is operator-driven. Nothing here launches a model.

---

## 1. Before spending live-run budget

Run these in order. Each one has caught a real defect; skipping them wastes runs.

```sh
cargo fmt --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo audit --deny warnings
python3 scripts/evaluate_governor.py --self-test
```

Then confirm the host contract on the machine that will run the comparison.
A synthetic `PostToolBatch` test does not prove live delivery:

1. Install the hook (`ctk init`) and **restart the Claude Code session** — hooks
   are snapshotted at session start.
2. Capture real payloads with `ctk record <path>` and confirm the host actually
   emits `PostToolBatch` and `UserPromptSubmit`. Attribution uses those as its
   batch and turn boundaries. Without them every recovery degrades to a lower
   confidence and safe mode never learns.
3. Confirm the host accepts `updatedToolOutput` by seeing a large Read come back
   compressed in a real session.

If the host cannot supply that evidence, **stop**. Pass-through guarantees still
hold, but inferred ordering must not be counted as high confidence and safe mode
is not ready to ship. That is an unresolved gate, not a pass.

Finally run the smoke case: a large Read compresses, its advertised targeted
recovery passes through, an edited file stays protected, and the ledger records
the decision.

---

## 2. The four arms

Each arm is one `.cubtoken.toml`, written into the **task checkout root**.

> **Trap.** Config is the nearest-ancestor `.cubtoken.toml` layered over
> `~/.config/cubtoken/config.toml`. Several task repositories ship their own
> `.cubtoken.toml` — cubtoken does. Overwrite it. Also resolve and record the
> global file, or a personal setting silently changes an arm. Record the
> effective configuration per run; do not assume it.

**Arm `disabled`** — total task cost with no compression at all.

```toml
[read]
enabled = false
[grep]
enabled = false
[glob]
enabled = false
[bash]
enabled = false
[stats]
ledger = true
[adaptive]
mode = "off"
[output]
mode = "default"
```

**Arm `static`** — the intended production configuration; the value of
compression alone.

```toml
[read]
enabled = true
threshold_tokens = 2000
[grep]
enabled = true
[glob]
enabled = true
[bash]
enabled = false
[stats]
ledger = true
[adaptive]
mode = "off"
[output]
mode = "default"
```

**Arm `observe`** — `static` with `[adaptive] mode = "observe"`.
**Arm `safe`** — `static` with `[adaptive] mode = "safe"`.

Only the `[adaptive] mode` line differs between `static`, `observe`, and `safe`.
`[output] mode` stays `"default"` in every arm: output-profile tuning is out of
scope and would confound the comparison.

> `stats.ledger = true` in every arm is not optional. With it false the policy
> records no decisions, no bucket reaches the eight-observation minimum, and a
> `safe` arm silently runs as `static` while still reporting `mode = "safe"`.
> `ctk adaptive status --json` reports `learning_enabled` for exactly this
> reason, and `evaluate_governor.py` flags any arm whose record says false.

Keep fixed across every run: model identifier, reasoning settings, host version,
available tools, task revision, dependencies, and time limits.
**Counterbalance arm order across repetitions** to reduce ordering and cache
effects — for example run `disabled, static, observe, safe` in repetition 1 and
the reverse in repetition 2.

---

## 3. Checkpoint handling

Safe mode can only change a decision in a bucket that already has eight or more
observations. Those must come from **separate, unscored warm-up tasks**.

1. Build the warm-up state in its own directory, with `mode = "observe"`, by
   running tasks that are *not* in `tasks.json`. Reading many similarly sized
   Rust files is what fills a bucket; `ctk adaptive explain <file>` shows which
   bucket a given file lands in.
2. Verify the checkpoint before spending run budget:
   - `ctk adaptive status --json` shows at least one bucket with `outcomes`
     length >= 8;
   - the state survives a refresh — run a session, then check the bucket is
     still there. A `refresh` opens its epoch at *now* when no state file
     exists, so observations recorded before the epoch are discarded by design;
   - a fresh, empty state leaves thresholds unchanged (the eight-observation
     minimum), which is worth confirming once as a negative control.
3. Record the checkpoint hash: `shasum -a 256 .cubtoken/adaptive-v1.json`.
4. **Copy** the whole `.cubtoken/` directory into each arm's isolated run
   directory. Never share writable state between runs.

Do not hand-edit `adaptive-v1.json` to simulate learning. A run whose policy was
edited by hand measures nothing. Do not count scored tasks as warm-up evidence.

---

## 4. Per-run procedure

For each (task, repetition, arm) — 5 x 2 x 4 = 40 scored runs:

1. **Fresh checkout** of the task revision into a new directory. Never reuse a
   previous task's edits.
2. **Fresh agent session.** Never reuse a conversation.
3. Write the arm's `.cubtoken.toml` into the checkout root, overwriting any that
   shipped with the repository.
4. Restore the checkpoint into the run's own `.cubtoken/`.
5. Capture **pre-run** state so warm-up totals are excluded from scored
   diagnostics:
   ```sh
   ctk adaptive status --json > run/status-before.json
   ctk stats --json          > run/stats-before.json
   ```
6. Run the task's setup steps, then give the agent the task's **exact prompt**,
   unmodified. Enforce the time limit.
7. Capture **post-run** state (`status-after.json`, `stats-after.json`) and save
   the host's authoritative usage result.
8. Run the task's `check` command **from the checkout root**, with `CHECKS`
   exported to the absolute path of this directory's `checks/`:
   ```sh
   export CHECKS=/path/to/cubtoken/tests/evaluation/checks
   ```
   Record its exit status and output. Checks are only ever introduced after the
   agent has stopped, so nothing in `checks/` is visible to it while it works.
9. Write one JSONL record (section 6).

Record every run, including failures, timeouts, and infrastructure errors.
Preserve them in the report. A failed task is never a token win.

---

## 5. Host usage mapping — verify, do not assume

**This is the single most likely way to produce a confident wrong number.**

Total model usage must come from the host's authoritative usage result. Do
**not** derive it from transcript text, and do **not** use the hook's
character-based estimates (~3.5 chars/token) — those size compression decisions,
they do not measure billing.

Before the first scored run, confirm against the host version you will use:

- What the host reports as prompt tokens, and **whether cached tokens are
  included in that number or reported alongside it**.
- Whether the figure is per-turn or per-session, and if per-turn, that summing
  across turns is correct and does not double-count.

Then set `usage.input_accounting` in every record to whichever applies:

| Value | Meaning | Total input is |
| --- | --- | --- |
| `components_additive` | `input_tokens` **excludes** cached tokens, which are reported separately. This is the Anthropic Messages API shape (`input_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`). | `input + cache_creation + cache_read` |
| `total_inclusive` | `input_tokens` **already contains** cached reads; the cache figure is a subset kept for reporting. | `input_tokens` alone |

Choosing `components_additive` for a `total_inclusive` provider double-counts
every cached token and manufactures a saving that is not there. The analysis
script refuses any other value rather than guessing, and `usage.field_mapping`
must state in prose how the host's result was mapped. Write down the host
version you verified against in `RESULTS.md`.

If a run has no authoritative usage, set the missing fields to `null`. They will
be reported as unavailable. **Do not write zero.**

Record cache usage as the provider exposes it. If the provider offers no way to
disable or control caching, say so rather than implying it was controlled.

---

## 6. Record schema

One JSON object per line. `schema_version` is `1`. Every field is required;
unknown usage values are `null`, never `0`.

```json
{
  "schema_version": 1,
  "run_id": "t1-r1-static",
  "task_id": "t1-large-file-one-function",
  "repetition": 1,
  "arm": "static",
  "source_revision": "fc6d7d5",
  "cubtoken_revision": "<SHA of the frozen governor baseline>",
  "config_hash": "sha256:<of the arm's .cubtoken.toml>",
  "checkpoint_hash": "sha256:<of adaptive-v1.json as restored>",
  "environment": {
    "host_version": "claude-code x.y.z",
    "model": "<exact model id>",
    "model_settings": "<reasoning settings, verbatim>",
    "started_at_utc": "2026-09-22T10:00:00Z",
    "elapsed_ms": 412000,
    "time_limit_ms": 900000
  },
  "outcome": {
    "status": "pass",
    "check_command": "cargo test --locked -p ctk-sitter --test t1_extensions",
    "check_exit_status": 0,
    "evidence_path": "runs/t1-r1-static/check.log",
    "exclusion_reason": null
  },
  "usage": {
    "input_accounting": "components_additive",
    "input_tokens": 18422,
    "cache_creation_input_tokens": 1200,
    "cache_read_input_tokens": 90140,
    "output_tokens": 3311,
    "artifact_path": "runs/t1-r1-static/usage.json",
    "field_mapping": "Host usage result, summed across assistant turns; input_tokens excludes cache."
  },
  "governor": {
    "status_before_path": "runs/t1-r1-static/status-before.json",
    "status_after_path": "runs/t1-r1-static/status-after.json",
    "stats_before_path": "runs/t1-r1-static/stats-before.json",
    "stats_after_path": "runs/t1-r1-static/stats-after.json",
    "gross_savings_tokens": 14000,
    "attributed_recovery_tokens": 900,
    "attributed_recovery_events": 2,
    "legacy_refetch_count": 3,
    "policy_applied": false,
    "policy_evidence": null,
    "learning_enabled": true
  }
}
```

Notes on the governor group:

- `gross_savings_tokens` and `attributed_recovery_tokens` are **run-local**:
  the after-snapshot minus the before-snapshot, not lifetime totals.
- `legacy_refetch_count` is diagnostic only. It is never substituted for
  attributed recovery cost, and refetch counts alone must not drive the
  shipping decision.
- `policy_applied` is true only when a recommendation actually changed an
  effective threshold in this run; `policy_evidence` names the bucket and
  factor. If no safe-mode run applied the policy, the policy comparison is
  **inconclusive** — not a zero-cost result.
- `learning_enabled` comes from `ctk adaptive status --json`. False in an
  `observe` or `safe` arm means that arm ran as `static`.

---

## 7. Analysis

```sh
python3 scripts/evaluate_governor.py --runs runs/*.jsonl --out tests/evaluation/report.md
```

The script is read-only: it launches nothing, installs nothing, changes no
configuration, and never writes to its inputs. It exits non-zero on data errors
— duplicate run IDs, two records for one matrix cell, mixed cubtoken revisions,
a task run against multiple source revisions, an ambiguous usage mapping — and
reports them rather than pooling the records anyway. Repair the evaluation
before interpreting any savings.

It computes signed differences, so a regression shows as a negative number:

```text
task_tokens                   = normalized_total_input_tokens + total_output_tokens
compression_savings           = disabled_task_tokens - static_task_tokens
adaptive_savings              = static_task_tokens      - safe_task_tokens
estimated_net_context_saved   = run_gross_savings       - run_attributed_recovery_tokens
```

The last is computed from components because `ctk stats`' net total saturates at
zero and cannot express a loss. It is an estimate from the hook's
character-based sizing, not authoritative usage, and the report labels it so.

Check the report's own integrity section first, then task outcomes for every
arm, then total consumption across all attempts — a comparison restricted to
successful pairs cannot conceal a regression paid for in failed runs.

Verify the script still behaves before trusting a report:

```sh
python3 scripts/evaluate_governor.py --self-test
```

Its checks were mutation-verified: breaking cache normalization, clamping a
negative saving, treating missing usage as zero, accepting duplicate runs, or
counting a failed task as a passing pair each makes the self-test fail.

---

## 8. Performance measurement

Two ignored drivers, run explicitly. They report p95 over repeated samples, not
a single timing:

```sh
cargo test --release -p ctk-hook --test performance -- --ignored --nocapture
cargo test --release -p ctk-cli  --test performance -- --ignored --nocapture
```

The first measures governor-only cost — replay, policy load, lookup,
attribution, append, and rebuild — at 100, 1,000, 5,000, and 10,000 ledger
records. **100 and 1,000 records are "typical" (2 ms p95 target); 5,000 and
10,000 are "large" (5 ms p95 target).**

Per-tool-call and per-turn work are reported separately, because they run at
very different frequencies and one combined worst-case number misrepresents
both. `adaptive_state::refresh` fires only on `Stop` and `SessionEnd`; the
Read path pays `Ledger::open` plus a small state load.

The second measures the real binary: process startup and end-to-end hook
latency. These are **not** governor cost and the targets above do not apply to
them; they are reported so the governor's share is not confused with the cost
of spawning a process.

Current results and which targets are met are in [RESULTS.md](RESULTS.md).

---

## 9. What this does not establish

Forty runs over five tasks is an **initial diagnostic sample**. It supports an
engineering release decision and will surface a regression. It does not
establish statistical non-inferiority, does not establish broad superiority, and
does not justify changing defaults. Adaptive mode stays `off` by default even
after every gate passes.

"High confidence" is an attribution category — appropriately ordered,
overlapping recovery of unchanged content — not a calibrated probability and not
proof that compression caused a task failure.

Keep raw transcripts and usage exports **outside version control**. They can
contain source, command output, paths, and secrets. Commit redacted fixtures or
compact normalized evidence only where reproducibility needs it.
