# Adaptive governor validation receipt

**Verdict: not ready to ship safe mode.** Three of five acceptance gates are
met. One is blocked on evidence that cannot be produced without a live host
session, and one — the task-token comparison, the gate the whole evaluation
exists to answer — has not been run. One performance target is missed at the
top of the large-session band.

Nothing here authorizes a push, a merge, or a default change. Adaptive mode
stays `off` by default regardless of outcome.

| Field | Value |
| --- | --- |
| Governor baseline (frozen) | `3d4a3d5fda3e34302a3d2704d6b019da06f45f77` |
| Commit under test | `de1fe7ed8f465b1bd5d87d0dce4cf8bd0640be3c` |
| Branch | `feat/adaptive-governor-eval-spec-4b3ba0` |
| Execution date | 2026-09-22 (UTC) |
| Machine | Apple M5, arm64, macOS 27.0 |
| Toolchain | rustc 1.96.0, cargo 1.96.0, Python 3.14.7 |
| `ctk` version | 0.1.3 |

The baseline commit is the in-progress governor imported verbatim from the
dirty `main` checkout, so scored runs can name an immutable implementation.
Four later commits add acceptance coverage, the analysis contract, the
performance drivers with one runtime fix, and the task matrix. This receipt
itself adds documentation only. **Any further runtime change creates a new
comparison baseline and invalidates runs scored against this one.**

---

## Gate 1 — Correctness and compatibility: PARTIAL

### Repository verification: PASS

Run at `de1fe7e` on the machine above.

| Command | Result |
| --- | --- |
| `cargo fmt --check` | exit 0 |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | exit 0 |
| `cargo test --locked --workspace --all-features` | exit 0 — **143 passed, 0 failed, 2 ignored** |
| `cargo audit --deny warnings` | exit 0 |
| `python3 scripts/evaluate_governor.py --self-test` | exit 0 — 17/17 checks |

The two ignored tests are the performance drivers, run explicitly in gate 5.
Test count rose from 134 at the frozen baseline to 143.

### Acceptance coverage added: PASS

The runtime design required checks the suite did not have. Seven were added and
one defect was found.

| Check | What it pins |
| --- | --- |
| `statistics_disabled_keeps_edit_and_targeted_protection` | Edit protection and targeted pass-through live outside the `cfg.stats.ledger` guards and must stay there. |
| `reset_survives_a_later_refresh` | The `reset_at_ms` filter; without it the next hook call undoes an `adaptive reset`. |
| `unavailable_or_malformed_state_falls_back_to_static` | Truncated, absent, non-JSON, future-schema, and future-policy state all fall back to the configured threshold. |
| `too_few_observations_leave_the_threshold_unchanged` | The eight-observation minimum, through `refresh` rather than the pure policy. |
| `refresh_sums_only_high_confidence_recoveries_for_the_matching_decision` | The aggregation semantics, pinned before optimising them. |
| `plugin_events_match_init` | Installer/plugin event parity, checked against what `ctk init` actually writes. |
| `status_json_reports_whether_adaptation_is_possible` | An arm can prove which policy it ran. |

`plugin_events_match_init` was verified to catch drift, not merely to pass:
deleting `PostToolBatch` from `plugins/cubtoken/hooks/hooks.json` makes it fail.
That matters because attribution uses `PostToolBatch` as its batch boundary, so
a plugin install missing it silently downgrades every recovery below the
confidence the policy trains on.

### Defect found and fixed: configuration trap

`[adaptive] mode = "safe"` with `[stats] ledger = false` records no decisions,
so no bucket ever reaches the eight-observation minimum. The arm runs as
`static` while still reporting `mode = "safe"`. An evaluation whose operator had
`stats.ledger = false` in a global config would have produced a confident,
meaningless "safe mode changed nothing" result.

Statistics are deliberately optional, so the fix is diagnostic rather than
behavioural: `ctk adaptive status` now reports `stats_ledger`,
`read_threshold_tokens`, and `learning_enabled`, and warns in the human output.
`evaluate_governor.py` flags any `observe` or `safe` arm whose record says
`learning_enabled: false`. Behaviour is pinned unchanged by
`safe_mode_without_statistics_cannot_learn`.

### Live host contract: NOT ESTABLISHED — blocking

Per the spec, "a synthetic `PostToolBatch` test does not prove live delivery."
This has **not** been demonstrated:

- that the installed host actually emits `PostToolBatch` and `UserPromptSubmit`
  for the batch and turn boundaries attribution depends on;
- that the host accepts the `updatedToolOutput` replacement schema in a live
  session;
- the restart-and-smoke procedure against a real session.

It requires installing the hook, restarting a Claude Code session, and capturing
redacted real payloads with `ctk record` — none of which can be done from within
a single automated session. The procedure is in
[README.md](README.md) section 1.

**This is unresolved, not a pass.** Until it is closed, inferred ordering must
not be counted as high confidence and safe mode is not ready to ship.

---

## Gate 2 — Measurement integrity: PASS (tooling only)

`scripts/evaluate_governor.py` is read-only by contract: it launches nothing,
installs nothing, changes no configuration, and never writes to its inputs.

It refuses rather than guesses. Data errors — duplicate run IDs, two records for
one matrix cell, mixed cubtoken revisions, one task against multiple source
revisions, an ambiguous `input_accounting` — exit non-zero and are reported
instead of being pooled.

The self-test's 17 checks were **mutation-verified**: each of the five failure
modes the spec names was introduced deliberately and confirmed to make the
self-test fail.

| Mutation | Detected by |
| --- | --- |
| Add `cache_read` under `total_inclusive` | `cache input is not double-counted` (got 19000, expected 10000) |
| `max(0, difference)` | `a negative savings result is reported, not clamped`; `the negative result is visible in the report` |
| Missing `input_tokens` becomes `0` | `a run with missing usage is excluded`; `missing usage does not become a zero-token total` |
| Drop the duplicate-run check | `duplicate run IDs are rejected` |
| Compare pairs regardless of status | `a failed run is not counted as a successful matched comparison`; `a timed-out run is not a token win` |

The prompt-token normalization is the part most able to manufacture a saving
that is not there, so the record must declare provider semantics explicitly and
an unrecognised value is an error. The mapping is documented in
[README.md](README.md) section 5.

This gate covers the **tooling**. It cannot be fully discharged until real runs
exist, because the host usage mapping must be verified against the host version
actually used and recorded here.

---

## Gate 3 — Task outcomes: NOT RUN

The task matrix is defined ([tasks.json](tasks.json)): five tasks, one per
required family, 2 repetitions x 4 arms = **40 scored runs**. None have been
executed. Live execution is operator-driven and needs a real host and budget.

Every success check was validated in both directions — it must fail before the
task is done and pass after — rather than assumed:

| Task | Family | Check validated |
| --- | --- | --- |
| `t1-large-file-one-function` | Skeleton navigation + small recovery | Fails at `fc6d7d5` (1 of 2 tests), passes after the change. |
| `t2-multi-file-trace` | Repeated reads, broad context | Fails with no `TRACE.md`, fails on an out-of-order chain, passes on a correct one. |
| `t3-repair-omitted-detail` | Insufficient context as a correctness failure | Fails at the buggy parent `2268a77` (both `doctor` and `stats` subdirectory tests), passes at the fix `d6ccb0f` (32 tests). |
| `t4-edit-then-read` | Edit protection | Fails before, fails on a definition-only rename, passes on a complete one. |
| `t5-unparseable-source` | Fallback elision | Fails with no audit, fails on a wrong default, passes on a correct one. |

Validating rather than assuming caught a broken task. `t5` originally asked for
a README configuration section that `fc6d7d5`'s README **already contains**, so
its check passed on untouched code and the task would have measured nothing
while appearing to succeed in all four arms. It was redesigned around an artifact
derived from the configuration table, which sits in the elided middle of a 16 KB
file with no tree-sitter grammar — so the task now genuinely requires the escape
hatch, which is the fallback behaviour that family exists to exercise.

`t3` is the strongest correctness signal: the check is the upstream fix's own
test suite, written before this evaluation existed and never shown to the agent.

---

## Gate 4 — Token benefit and coverage: NOT RUN

Depends entirely on gate 3. No token claim is made in either direction.

Two outcomes must not be misread as successes when the runs happen:

- A run without authoritative usage makes its token comparison **unavailable**,
  not zero.
- If safe mode never changes a threshold or a compression decision, the policy
  comparison is **inconclusive**, not a zero-cost result. The analysis emits
  that warning automatically when no safe run reports `policy_applied`.

---

## Gate 5 — Performance: PARTIAL

Measured at `de1fe7e`, release build, p95 over repeated samples. Fixture sizes
**100 and 1,000 records are "typical" (2 ms target); 5,000 and 10,000 are
"large" (5 ms target)**, as the spec requires be stated explicitly.

### Bottleneck found and fixed

The first run measured `adaptive_state::refresh` at **17.891 ms p95 at 10,000
records** against a 5 ms target. It scanned every recovery once per decision —
O(decisions x recoveries). Grouping recoveries by decision id in a single pass
brings it to 5.796 ms. The aggregation it had to preserve was pinned by a test
first.

### Governor-only cost (p95)

| records | category | operation | p95 ms | iterations |
| --- | --- | --- | --- | --- |
| 100 | typical | replay (Ledger::open) | 0.124 | 200 |
| 100 | typical | policy load (adaptive load) | 0.001 | 200 |
| 100 | typical | lookup (is_protected) | 0.000 | 2000 |
| 100 | typical | attribution (classify) | 0.000 | 2000 |
| 100 | typical | append (note_saving) | 0.142 | 200 |
| 100 | typical | rebuild (adaptive refresh) | 0.309 | 40 |
| 1000 | typical | replay (Ledger::open) | 0.468 | 200 |
| 1000 | typical | policy load (adaptive load) | 0.001 | 200 |
| 1000 | typical | lookup (is_protected) | 0.000 | 2000 |
| 1000 | typical | attribution (classify) | 0.000 | 2000 |
| 1000 | typical | append (note_saving) | 0.125 | 200 |
| 1000 | typical | rebuild (adaptive refresh) | 0.932 | 40 |
| 5000 | large | replay (Ledger::open) | 1.904 | 40 |
| 5000 | large | policy load (adaptive load) | 0.001 | 40 |
| 5000 | large | lookup (is_protected) | 0.000 | 400 |
| 5000 | large | attribution (classify) | 0.000 | 400 |
| 5000 | large | append (note_saving) | 0.106 | 40 |
| 5000 | large | rebuild (adaptive refresh) | 3.130 | 10 |
| 10000 | large | replay (Ledger::open) | 3.722 | 40 |
| 10000 | large | policy load (adaptive load) | 0.001 | 40 |
| 10000 | large | lookup (is_protected) | 0.000 | 400 |
| 10000 | large | attribution (classify) | 0.000 | 400 |
| 10000 | large | append (note_saving) | 0.099 | 40 |
| 10000 | large | rebuild (adaptive refresh) | 5.796 | 10 |

Reported per frequency, because `refresh` fires only on `Stop` and `SessionEnd`
— once per turn — while the Read path pays `Ledger::open` plus a small state
load on every call. One combined worst-case number would misrepresent both.

| Path | Frequency | Typical p95 | Large p95 | Target | Verdict |
| --- | --- | --- | --- | --- | --- |
| Per tool call | every matched tool call | 0.468 ms | 3.722 ms | 2 / 5 ms | **PASS** |
| Per turn (rebuild) | `Stop`, `SessionEnd` | 0.932 ms | 5.796 ms | 2 / 5 ms | **FAIL at 10,000** |

`policy load` — the per-Read cost safe mode adds — is 0.001 ms at every size.
It reads the small state snapshot, not the ledgers.

### Process cost (p95), excluded from the governor targets

| operation | p95 ms | iterations |
| --- | --- | --- |
| startup (ctk --version) | 2.086 | 30 |
| end-to-end (ctk hook), 1,000 ledger records | 4.176 | 30 |

Process startup is ~2.1 ms of the ~4.2 ms end-to-end figure, so the governor's
own share at 1,000 records is roughly 2 ms. These are reported separately so
spawn cost is not attributed to the governor.

### The remaining miss

`rebuild` at 10,000 records is **5.796 ms against a 5 ms target**. At 5,000
records it is 3.130 ms and passes. The remaining cost is JSON parsing, not
algorithm: `refresh` walks **every** `session-*.jsonl` in the project via
`Ledger::load_all`, so it scales with the whole `.cubtoken/` directory rather
than the current session, and that directory is never pruned. A long-lived
project accumulates past it.

No index or snapshot was added, per the spec's instruction not to add one unless
a measurement requires it. The one-pass fix brought a 3.6x improvement and
cleared everything except the top of the band. Closing the last 0.8 ms means
either bounding what `load_all` reads — retention, or a per-session summary —
which is a design change belonging in its own commit, not a quiet addition here.

Caveat: these are single-machine numbers on an Apple M5. CI runs Ubuntu, macOS,
and Windows; the targets should be confirmed on the slowest of them before this
gate is called closed.

---

## Acceptance summary

| Gate | Verdict | What remains |
| --- | --- | --- |
| Correctness and compatibility | **PARTIAL** | Live host event/output contract not demonstrated. Blocking. |
| Measurement integrity | **PASS** (tooling) | Host usage mapping must be verified and recorded when runs happen. |
| Task outcomes | **NOT RUN** | 40 scored runs. |
| Token benefit and coverage | **NOT RUN** | Depends on the above. |
| Performance | **PARTIAL** | `rebuild` 5.796 ms vs 5 ms at 10,000 records; cross-platform confirmation. |

## Recommendation

**Defer the merge of safe mode as a shipping feature.** Two options remain open,
exactly as the spec frames them:

1. **Defer** until the live host contract is demonstrated, the 40 runs are
   executed, and the rebuild bottleneck is either closed or the large-session
   fixture boundary is re-argued on evidence rather than convenience.
2. **Revise the release scope to observation-only** — ship `off` and `observe`,
   hold `safe` back — and review the corresponding code and documentation
   changes as their own commit.

The existing safe-mode shipping criteria are not optional merely because the
feature is opt-in.

The evaluation tooling, task matrix, and acceptance coverage in this branch are
independently useful and can be reviewed and merged on their own. Pushing this
branch for review is not a shipping decision, provided the missing evidence is
stated — which is what this document is for.

## Reproducing the local artifacts

The commands above are the whole receipt; there are no hidden inputs. Raw
transcripts and usage exports are deliberately **not** in version control: they
can contain source, command output, paths, and secrets. When runs are executed,
keep them outside the repository and cite their paths in each record's
`evidence_path`, `artifact_path`, and `status_*_path` fields, as
[README.md](README.md) section 6 describes.
