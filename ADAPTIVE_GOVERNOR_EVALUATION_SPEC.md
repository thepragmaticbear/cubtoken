# Adaptive governor: implementation and validation spec

Status: ready for implementation; validation results pending.  
Date: September 22, 2026.  
Baseline inspected: local `main` at `fc6d7d5`, including the uncommitted and untracked adaptive-governor changes.

## 1. Decision and branch sequencing

**Begin this work on top of the current governor changes. They do not need to be pushed or merged first. Complete the validation required below before treating safe mode as ready to ship.**

The current implementation already contains the useful part of the Jev-inspired idea: observe recovery costs and reduce compression when those costs become excessive. Do not add session-wide protection after one refetch. A targeted Read is an intended recovery mechanism and already passes through unchanged.

| Question | Decision |
| --- | --- |
| Does the current work need a new refetch-protection policy? | No. Keep the existing attribution and opt-in governor. |
| Does the current work need anything before merge? | Yes: complete the existing verification, host-contract, baseline, and performance gates; add reproducible evaluation tooling and evidence. Fix defects those checks establish. |
| Must the governor be pushed or merged before evaluation implementation starts? | No. Evaluation depends on its code, not its merge status. |
| May the work be pushed for review before live evaluation finishes? | Yes, as work in progress with the missing evidence stated. Pushing a review branch is not a shipping decision. |
| When should another adaptive policy be implemented? | Only after this evaluation identifies repeat waste that the existing governor does not address; use a separate follow-up change. |

The checkout currently has no separate feature branch: it is dirty `main`. When implementation starts, preserve this working tree on a `codex/` feature branch and commit the governor baseline before comparing runs. Do not reset, stash away, or omit the untracked governor modules. This spec does not authorize a push or merge.

This document implements the evaluation work described in [ADAPTIVE_CONTEXT_GOVERNOR.md](ADAPTIVE_CONTEXT_GOVERNOR.md), particularly its baseline, task-token benchmark, and shipping criteria. That document remains the runtime design. This spec adds an executable evaluation contract; it does not replace the governor or weaken its shipping gates.

## 2. Outcome and scope

Determine whether the current governor preserves task correctness while reducing total model token use at acceptable latency. Produce evidence that another developer can reproduce from a recorded commit, configuration, task, and host version.

The implementation has three deliverables:

1. A small task manifest and runbook for controlled coding-agent comparisons.
2. One local analysis script that validates exported run records and produces a Markdown results report.
3. A checked-in validation receipt linking test results, host fixtures, performance measurements, and the task comparison.

Use Python's standard library for analysis unless an equivalent repository utility exists when work starts. Reuse the existing Rust tests, ledger, `ctk stats --json`, and `ctk adaptive status --json`. Do not introduce a service, database, dashboard, model dependency, or general-purpose benchmark framework. Live task execution may initially be operator-driven; unattended agent orchestration is unnecessary for the first comparison.

Excluded: a Jev runtime integration, learned probabilities, automatic session-wide file pinning, new compression strategies, new adaptive thresholds, default changes, and output-profile tuning. Keep `[output].mode = "default"` in every measured arm.

## 3. Preserve the existing runtime contract

The benchmark must exercise the current policy rather than quietly alter it to obtain a favorable result:

| Area | Required behavior |
| --- | --- |
| Safety | Targeted and session-edited Reads pass through; unavailable protection state fails open; displayed lines remain verbatim. |
| Attribution | Only appropriately ordered, overlapping recoveries of unchanged content qualify for high confidence. Legacy counters and lower-confidence events remain diagnostic. |
| Recovery | Preserve the existing eligible full-repeat escape once per compression decision. Do not convert it into permanent file protection. |
| Adaptation | Preserve `off`, `observe`, and one-way `safe` behavior, the eight-observation minimum, current bucket keys, and current thresholds. |
| Configuration | Preserve default `adaptive.mode = "off"` and optional statistics. An experiment must explicitly record its effective configuration. |

“High confidence” is an attribution category, not a calibrated probability or proof that compression caused a task failure. Refetch counts alone must not determine the shipping decision.

## 4. Implementation sequence

### Step 1 — Freeze and validate the governor baseline

Create a reproducible commit containing the current governor implementation and record its SHA. Inspection and test work can precede that commit; scored comparisons must identify an immutable implementation. Any subsequent runtime fix creates a new comparison baseline.

Run the repository verification gate:

```sh
cargo fmt --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo audit --deny warnings
```

Review existing coverage before adding tests. Add only missing checks required by the runtime design: statistics-disabled targeted/edit protection; attribution ordering and changed-content exclusion; reset surviving a subsequent refresh; malformed or unavailable state; concurrent ledger behavior; and installer/plugin event parity. Fix a confirmed defect in its owning module with a reproducing test.

Validate the installed host contract using redacted, real payloads. Demonstrate that the host emits the batch boundary used for attribution and accepts the replacement-output schema. A synthetic `PostToolBatch` test does not prove live delivery. If the supported host cannot supply the required evidence, preserve pass-through guarantees and do not count inferred ordering as high confidence; resolve the compatibility issue before claiming safe-mode readiness.

Also run the existing restart-and-smoke procedure: a large Read compresses, its advertised targeted recovery passes through, an edited file remains protected, and the ledger records the decision. Confirm a known eligible recovery can reach the policy through real events.

### Step 2 — Define the task matrix

Add `tests/evaluation/tasks.json` and a short runbook at `tests/evaluation/README.md`. Each task must name an immutable repository revision, the exact prompt, setup requirements, a time limit, and an independent success check. Use disposable repositories containing no production credentials.

Start with five tasks covering these distinct behaviors:

| Task family | Why it is included |
| --- | --- |
| Find and modify one function in a large file | Exercises useful skeleton navigation followed by a small recovery. |
| Trace behavior across multiple files | Exercises repeated reads and broader context needs. |
| Repair a bug requiring omitted implementation details | Makes insufficient context observable through a correctness check. |
| Make a change followed by additional reads | Exercises edit protection. |
| Work with unsupported or unparseable source | Exercises fallback behavior. |

At least one task must exercise a bucket with enough separate training observations for safe mode to change a decision. Do not count scored evaluation tasks as warm-up evidence when selecting the starting policy. Do not choose only tasks that already favored the governor.

Run two repetitions of each task in each of four arms: 40 scored runs. This is an initial diagnostic sample, not proof of statistical non-inferiority. Preserve failures and timeouts in the report.

| Arm | Configuration and purpose |
| --- | --- |
| Compression disabled | Disable Read, Grep, Glob, and Bash compression; keep the host setup and default output profile consistent. Establish total task cost without compression. |
| Static | Enable the intended production compression configuration with adaptive mode `off`. Establish the value of compression alone. |
| Observe | Use the same compression configuration with adaptive mode `observe`. Verify collection and policy recommendations without applying adaptive thresholds. |
| Safe | Use the same compression configuration with adaptive mode `safe`. Measure the incremental effect of threshold backoff. |

Keep model identifier, reasoning settings, host version, available tools, task revision, dependencies, and time limits fixed. Counterbalance arm order across repetitions to reduce ordering and cache effects. Record all cache usage; do not pretend it was controlled if the provider does not expose a way to control it.

### Step 3 — Capture isolated runs and policy state

Use a fresh task checkout and a fresh agent session for each scored run. Never reuse the previous task's edits or conversation. Resolve and record inherited configuration so global settings do not silently change an arm.

Prepare training observations from separate, unscored warm-up tasks. Preserve the complete governor checkpoint needed to reproduce the starting policy, including its reset epoch and supporting ledger history. Restore equivalent checkpoints into isolated arm directories; never share writable state between runs. Record the checkpoint hash and capture policy status before and after each run. Do not manually edit recommendations to simulate learning.

Take pre-run and post-run statistics snapshots so warm-up totals are excluded from scored diagnostics. A checkpoint must survive a refresh and retain its intended observations; verify this before spending live-run budget. A fresh-state smoke case must also show that too few observations leave thresholds unchanged.

For each run, save the host's authoritative usage result, the task-check output, effective configuration, and the relevant cubtoken exports locally. Verify usage field semantics against the supported host version during implementation and document the mapping. Do not derive total model usage from transcript text or the hook's character-based estimates.

If a run lacks authoritative usage, mark token comparisons unavailable. If safe mode never changes a threshold or compression decision, label the policy comparison inconclusive. Neither condition is a zero-cost result or a successful validation of adaptation.

### Step 4 — Implement the analysis contract

Add `scripts/evaluate_governor.py`. It reads explicit local input paths, validates a JSONL record per run, and writes a report to an explicit output path. It must not launch models, install hooks, change configuration, or mutate the source run records.

Use a versioned record with the following required groups:

| Group | Fields |
| --- | --- |
| Identity | Schema version, run ID, task ID, repetition, arm, source revision, cubtoken revision, configuration hash, checkpoint hash. |
| Environment | Host version, exact model/settings, UTC start time, elapsed milliseconds, time limit. |
| Outcome | `pass`, `fail`, `timeout`, or `infrastructure_error`; success-check command and exit status; evidence path and any exclusion reason. |
| Usage | Normalized total input and output tokens, available cache-read/write breakdown, original usage artifact path, and the documented field mapping. Missing values are null. |
| Governor diagnostics | Before/after statistics and policy artifact paths; run-local gross savings, attributed recovery tokens/events, legacy refetch counts, and evidence of policy application. |

Do not assume cache fields are additive to an already-total input count. Normalize once using the documented provider semantics and reject ambiguous mappings. Require unique run IDs and complete matching keys. Report missing runs, duplicate records, mixed revisions, and incompatible configurations as data errors rather than silently pooling them.

Compute signed differences:

```text
task_tokens = normalized_total_input_tokens + total_output_tokens
compression_savings = disabled_task_tokens - static_task_tokens
adaptive_savings = static_task_tokens - safe_task_tokens
estimated_net_context_saved = run_gross_savings - run_attributed_recovery_tokens
```

Compute the last metric from component deltas; the existing CLI net total saturates at zero and cannot expose negative savings. Label it an estimate. Legacy refetch totals must not be substituted for attributed recovery costs.

Report task outcomes for every arm first. For matched successful runs, show absolute and percentage token differences, elapsed-time differences, and policy coverage by task. Also show total consumption and failures across all attempts so excluding failures from successful-pair comparisons cannot conceal a regression. Keep input, output, and cache breakdowns visible. Never convert a failed task into a token win.

Include one runnable `--self-test` using small in-memory records. It must fail if analysis double-counts cache input, hides a negative savings result, treats missing usage as zero, accepts duplicate runs, or counts a failed task as a successful matched comparison. Do not add a test framework.

### Step 5 — Record evidence and decide

Add a results receipt at `tests/evaluation/RESULTS.md` containing the tested commit, execution date, exact verification commands, live host evidence, performance results, and the task-comparison verdict. Keep raw transcripts and usage exports outside version control; commit redacted fixtures or compact normalized evidence only when needed for reproducibility. Document how to obtain the local artifacts used by the receipt.

Measure ledger replay, append, lookup, attribution, and rebuild at 100, 1,000, 5,000, and 10,000 records. Reuse a simple local driver and repeat measurements enough to report p95 rather than one timing. Report process startup and end-to-end hook latency separately from governor-only cost. Apply the existing targets: p95 governor overhead below 2 ms for typical sessions and below 5 ms for large sessions; explicitly state which fixture sizes represent each category. Do not add an index or snapshot optimization unless a measurement requires it.

## 5. Acceptance and merge decision

| Gate | Required evidence | If it fails or remains unknown |
| --- | --- | --- |
| Correctness and compatibility | Full repository checks; real host event/output contract; invariant and recovery smoke cases. | Fix the current governor before merge. Missing host evidence is unresolved, not a pass. |
| Measurement integrity | Reproducible revisions/checkpoints; authoritative usage mapping; analysis self-test; all run outcomes retained. | Repair the evaluation before interpreting savings. |
| Task outcomes | No observed safe-mode correctness regression against matching successful static cases; failures individually reviewed. | Investigate and correct the cause before shipping safe mode. Small sample size still limits the claim. |
| Token benefit and coverage | Positive aggregate context and task-token savings on successful representative tasks versus compression disabled; report safe-versus-static results separately and demonstrate policy application. | Do not claim adaptive benefit. If safe is worse than static, review the policy before shipping it as beneficial. If it never activates, collect additional independent observations. |
| Performance | Recorded p95 measurements meeting the existing runtime targets. | Fix the measured bottleneck and rerun affected validation. |

The initial task set supports an engineering release decision and identifies regressions; it does not establish broad superiority or justify changing defaults. Keep adaptive mode off by default even after these gates pass.

If the gates cannot be completed before the intended merge, the options are to defer merge or explicitly revise the release scope to observation-only and review the corresponding code/documentation changes. Do not silently declare the existing safe-mode shipping criteria optional because the feature is opt-in.

## 6. Expected change footprint

| Location | Planned change |
| --- | --- |
| `tests/evaluation/tasks.json` | Five task definitions and independent checks. |
| `tests/evaluation/README.md` | Run procedure, configuration matrix, host usage mapping, checkpoint handling, and performance measurement procedure. |
| `scripts/evaluate_governor.py` | Standard-library record validation, analysis, Markdown output, and one self-test entry point. |
| `tests/evaluation/RESULTS.md` | Completed validation receipt with limitations and a merge recommendation. |
| Existing runtime modules/tests and real payload fixtures | Changes only for a demonstrated defect or missing acceptance coverage. No speculative governor refactor. |

The benchmark code and evidence may be a separate review commit stacked on the governor baseline. They do not require that baseline to be merged first. A future stronger recovery policy belongs in its own change, supported by this report.

## 7. Current evidence and immediate next action

The earlier review ran `cargo test --locked -p ctk-hook`: 42 tests passed. That establishes the inspected hook behavior only. Full workspace/audit checks, real-host compatibility, performance targets, and task-token benefit have not been established by this review.

**Next implementation action:** preserve the current working tree on a feature branch, freeze the governor baseline, and run Step 1. Do not implement the blanket refetch-protection proposal.
