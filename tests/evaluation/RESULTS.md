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
| Commit under test | `320b7a7d18cf734712b68e09ea24805a32c9bcda` |
| Branch | `feat/adaptive-governor-eval-spec-4b3ba0` |
| Execution date | 2026-09-22 (UTC) |
| Machine | Apple M5, arm64, macOS 27.0 |
| Toolchain | rustc 1.96.0, cargo 1.96.0, Python 3.14.7 |
| `ctk` version | 0.1.3 |

The baseline commit is the in-progress governor imported verbatim from the
dirty `main` checkout, so scored runs can name an immutable implementation.
Later commits add acceptance coverage, the analysis contract, the performance
drivers, the task matrix, and fixes for four defects found by adversarial
review. **The comparison baseline is therefore `320b7a7`, not `3d4a3d5`** —
runtime behaviour changed. No run had been scored against the older one, so
nothing is invalidated, but any further runtime change moves it again.

---

## Gate 1 — Correctness and compatibility: PARTIAL

### Repository verification: PASS

Run at `320b7a7` on the machine above.

| Command | Result |
| --- | --- |
| `cargo fmt --check` | exit 0 |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | exit 0 |
| `cargo test --locked --workspace --all-features` | exit 0 — **148 passed, 0 failed, 2 ignored** |
| `cargo audit --deny warnings` | exit 0 |
| `python3 scripts/evaluate_governor.py --self-test` | exit 0 — **25/25 checks** |

The two ignored tests are the performance drivers, run explicitly in gate 5.
Test count rose from 134 at the frozen baseline to 148.

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

### Four defects found by adversarial review: FIXED

A reproduction script exercising concurrency and filesystem edge cases found
four defects, all in the frozen baseline rather than in the evaluation work
layered on it. Each now has a test that fails before its fix.

| # | Defect | Severity | Fix |
| --- | --- | --- | --- |
| 1 | Edit protection lost whenever the ledger's advisory lock was unavailable. Every hook call is a fresh process, so the in-memory record died and the next Read compressed an already-edited file. | **Critical** | Edit records append whenever the directory is safe, lock or not; `writable` tracked apart from `lock`. |
| 2 | The data directory was checked for being a symlink, the session file inside it was not — a `session-*.jsonl` symlink redirected appends into its target. | **Critical** | Refuse to append through a symlinked session file. |
| 3 | `refresh` wrote back the reset epoch it had loaded, silently undoing a concurrent `adaptive reset`. | **Important** | Re-read the epoch before publishing; abandon the refresh if a reset moved it. |
| 4 | `adaptive explain` reported a recommendation as the decision in every mode, though only `safe` acts on one — so `observe` announced "disabled" for a file the next Read would compress. | **Important** | Report the mode and the threshold the hook will actually use. |

Defect 1 is worse than a race. The lock is a `create_new` lockfile removed on
`Drop`, so a single stale lockfile from a killed process disables edit
protection for the rest of the session — silently, because the hook always
exits 0. This is the Edit-correctness hazard the protection exists to prevent.

Two lessons are recorded rather than smoothed over:

- My own `reset_survives_a_later_refresh` covers only the **sequential** case
  and passed throughout. It gave false confidence about defect 3, which is
  concurrent. The replacement holds the window open deterministically with a
  FIFO instead of relying on timing.
- My first attempt at a reproduction for defect 3 **passed against the broken
  code** — it was vacuous, because `refresh`'s `now_ms` argument is only used
  when no state file exists. It was rewritten rather than kept. This is the
  same failure mode as the t5 task check, and it is why every check here is
  validated in both directions.

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
| `t1-large-file-one-function` | Skeleton navigation + small recovery | Fails at `fc6d7d5`, passes after the change. Its second test was **tautological** (`is_some()` vs `is_some()`) until review; now compares the `Lang` variant and catches a mutated mapping. |
| `t2-multi-file-trace` | Repeated reads, broad context | **Was unpassable** — the chain named two identifiers that do not exist at the pinned revision. Corrected and re-validated against a real export of `fc6d7d5`. |
| `t3-repair-omitted-detail` | Insufficient context as a correctness failure | Fails at the buggy parent `2268a77` (both `doctor` and `stats` subdirectory tests), passes at the fix `d6ccb0f` (32 tests). |
| `t4-edit-then-read` | Edit protection | Fails before, fails on a definition-only rename, passes on a complete one. |
| `t5-unparseable-source` | Fallback elision | Fails with no audit, fails on a wrong default, passes on a correct one. Wrong-default detection used **containment** until review, so `12000` satisfied a documented `2000`; now compares for equality. |

Validating rather than assuming caught a broken task. `t5` originally asked for
a README configuration section that `fc6d7d5`'s README **already contains**, so
its check passed on untouched code and the task would have measured nothing
while appearing to succeed in all four arms. It was redesigned around an artifact
derived from the configuration table, which sits in the elided middle of a 16 KB
file with no tree-sitter grammar — so the task now genuinely requires the escape
hatch, which is the fallback behaviour that family exists to exercise.

`t3` is the strongest correctness signal: the check is the upstream fix's own
test suite, written before this evaluation existed and never shown to the agent.
Review found it was nonetheless **solvable without reading the source** — task
checkouts were full clones, and a clone at `2268a77` still carries the fix
commit `d6ccb0f` in its object store, so the answer sat in `git log`. Checkouts
are now history-stripped exports and checks needing history read from a separate
reference clone.

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

Measured at `96ab8f3`, release build. Fixture sizes **100 and 1,000 records
are "typical" (2 ms target); 5,000 and 10,000 are "large" (5 ms target)**, as
the spec requires be stated explicitly.

### Two methodology defects, both corrected

The first run measured `adaptive_state::refresh` at **17.891 ms p95 at 10,000
records**. It scanned every recovery once per decision — O(decisions x
recoveries). Grouping recoveries by decision id in a single pass fixed it. The
aggregation it had to preserve was pinned by a test first.

The second defect was in the measurement itself, found by review: sample counts
were 10 for the large sizes, and nearest-rank gives `ceil(10 * 0.95) = 10`, so
the reported "p95" **was the maximum of ten samples**. An independent run of
the same driver on the same machine produced 7.06 to 28.64 ms for the figure
first recorded here as 5.796 — a 4x spread straddling the pass/fail line. The
earlier table was one lucky quiet run wearing a percentile's name.

Sample counts are now >= 100 everywhere, and min and median are reported beside
p95 so a noisy measurement is visible rather than silently becoming the result.
The distributions below are tight, which is what makes them usable.

### Governor-only cost

| records | category | operation | min ms | median ms | p95 ms | iterations |
| --- | --- | --- | --- | --- | --- | --- |
| 100 | typical | replay (Ledger::open) | 0.110 | 0.130 | 0.153 | 200 |
| 100 | typical | policy load (adaptive load) | 0.001 | 0.001 | 0.001 | 200 |
| 100 | typical | safe-mode threshold preview (tree-sitter) | 1.454 | 1.494 | 1.570 | 200 |
| 100 | typical | lookup (is_protected) | 0.000 | 0.000 | 0.000 | 2000 |
| 100 | typical | attribution (classify) | 0.000 | 0.000 | 0.000 | 2000 |
| 100 | typical | append (note_saving) | 0.079 | 0.102 | 0.154 | 200 |
| 100 | typical | rebuild (adaptive refresh) | 0.274 | 0.282 | 0.363 | 200 |
| 1000 | typical | replay (Ledger::open) | 0.411 | 0.419 | 0.446 | 200 |
| 1000 | typical | policy load (adaptive load) | 0.001 | 0.001 | 0.001 | 200 |
| 1000 | typical | safe-mode threshold preview (tree-sitter) | 1.443 | 1.499 | 1.565 | 200 |
| 1000 | typical | lookup (is_protected) | 0.000 | 0.000 | 0.000 | 2000 |
| 1000 | typical | attribution (classify) | 0.000 | 0.000 | 0.000 | 2000 |
| 1000 | typical | append (note_saving) | 0.079 | 0.104 | 0.154 | 200 |
| 1000 | typical | rebuild (adaptive refresh) | 0.789 | 0.805 | 0.905 | 200 |
| 5000 | large | replay (Ledger::open) | 1.809 | 1.842 | 1.924 | 100 |
| 5000 | large | policy load (adaptive load) | 0.001 | 0.001 | 0.001 | 100 |
| 5000 | large | safe-mode threshold preview (tree-sitter) | 1.473 | 1.500 | 1.598 | 100 |
| 5000 | large | lookup (is_protected) | 0.000 | 0.000 | 0.000 | 1000 |
| 5000 | large | attribution (classify) | 0.000 | 0.000 | 0.000 | 1000 |
| 5000 | large | append (note_saving) | 0.082 | 0.101 | 0.129 | 100 |
| 5000 | large | rebuild (adaptive refresh) | 3.030 | 3.089 | 3.194 | 100 |
| 10000 | large | replay (Ledger::open) | 3.575 | 3.617 | 3.687 | 100 |
| 10000 | large | policy load (adaptive load) | 0.001 | 0.001 | 0.001 | 100 |
| 10000 | large | safe-mode threshold preview (tree-sitter) | 1.458 | 1.502 | 1.551 | 100 |
| 10000 | large | lookup (is_protected) | 0.000 | 0.000 | 0.000 | 1000 |
| 10000 | large | attribution (classify) | 0.000 | 0.000 | 0.000 | 1000 |
| 10000 | large | append (note_saving) | 0.076 | 0.090 | 0.126 | 100 |
| 10000 | large | rebuild (adaptive refresh) | 5.876 | 5.995 | 6.072 | 100 |
| 0 | per-Read | under-threshold Read, static | 0.081 | 0.089 | 0.125 | 200 |
| 0 | per-Read | under-threshold Read, safe | 0.083 | 0.089 | 0.125 | 200 |
| 0 | per-Read | whole Read through run_hook, static | 1.644 | 1.794 | 1.898 | 200 |
| 0 | per-Read | whole Read through run_hook, safe | 3.301 | 3.489 | 3.609 | 200 |

Per-tool-call p95: typical 1.570 ms (target 2 ms), large 3.687 ms (target 5 ms).
Per-turn p95 (Stop/SessionEnd only): typical 0.905 ms (target 2 ms), large 6.072 ms (target 5 ms).

Worst governor-only p95: typical (100-1,000 records) 1.570 ms against a 2 ms target; large (5,000-10,000 records) 6.072 ms against a 5 ms target.

Reported per frequency, because `refresh` fires only on `Stop` and `SessionEnd`
— once per turn — while the Read path pays its cost on every call.

| Path | Frequency | Typical p95 | Large p95 | Target | Verdict |
| --- | --- | --- | --- | --- | --- |
| Per tool call, static | every matched tool call | 1.967 ms | 3.741 ms | 2 / 5 ms | **PASS**, narrowly |
| Per tool call, safe | every matched tool call | 3.725 ms | — | 2 ms | **FAIL** |
| Per turn (rebuild) | `Stop`, `SessionEnd` | 0.915 ms | 6.072 ms | 2 / 5 ms | **FAIL at 10,000** |

### A retracted claim

An earlier version of this receipt said "`policy load` — the per-Read cost safe
mode adds — is 0.001 ms at every size". **That was wrong.** It measured only
the JSON state snapshot and missed the tree-sitter parse sitting beside it:
`effective_read_threshold` called `preview_read`, which parses with no size
gate. Cloud review caught it.

Two costs were hiding behind that number. The first is fixed: a Read under the
threshold can never compress, whatever the policy recommends, so the parse is
now skipped. Measured on a large file under a raised threshold, p95 per Read:

| | static | safe |
| --- | --- | --- |
| before the fix | 0.124 ms | **1.643 ms** |
| after | 0.132 ms | 0.137 ms |

The second is not fixed: a Read that **does** compress parses the same content
twice, once for the preview and once in the compressor. End to end that is
**3.725 ms p95 in safe mode against static's 1.967 ms** — over the 2 ms
typical-session target. Threading the preview through to the compressor changes
a signature across two crates, so it belongs in its own commit.

### The remaining miss

`rebuild` at 10,000 records is **6.072 ms p95 against a 5 ms target**, with a
median of 5.968 and a minimum of 5.799 — consistently over, not a tail artifact.
At 5,000 records it is 3.173 ms and passes.

The cause is JSON parsing, not algorithm: `refresh` walks **every**
`session-*.jsonl` in the project via `Ledger::load_all`, so it scales with the
whole `.cubtoken/` directory rather than the current session, and that
directory is never pruned. A long-lived project accumulates past it.

No index or snapshot was added, per the spec's instruction not to add one
unless a measurement requires it. Bounding what `load_all` reads — retention,
or a per-session summary — is a design change belonging in its own commit, so
the number is reported against the stated boundary rather than the boundary
being moved to clear it.

There are now **two** distinct misses, and safe mode is implicated in both:
the per-turn rebuild at 10,000 records, and the per-Read double parse. Neither
is a tail artifact; both reproduce with tight distributions.

Caveat: single-machine numbers on an Apple M5. CI runs Ubuntu, macOS, and
Windows; confirm on the slowest before calling this gate closed.

---

## What review caught that self-review did not

Three rounds of checking preceded the code review: a verification gate,
both-directions validation of every task check, and mutation testing of the
analysis self-test. The review still found three miscalibrated instruments,
and the pattern in all three is the same — **I validated the instrument against
something I wrote, rather than against the thing it would measure.**

| Found | How my own checking missed it |
| --- | --- |
| `t2` unpassable at its pinned revision | I validated the checker with a `TRACE.md` I wrote to contain the identifiers. I never ran it against `fc6d7d5`, where two of them do not exist. |
| "p95" was the maximum of ten samples | I read the p95 helper and the numbers looked plausible. I never ran the driver twice and compared. |
| `t1`'s guard compared `is_some()` to `is_some()` | I mutation-tested the *analysis* self-test and the `refresh` rewrite, but not the task checks. |

Two earlier instances of the same class are recorded above: the `t5` check that
passed on untouched code, and a defect-3 reproduction that passed against broken
code. Five in total, all caught, none by the same method twice.

The concrete process changes: task checks are validated against a real export
of the pinned revision rather than a fixture; percentile figures state their
sample count and report median and minimum beside p95; and every check the
harness relies on now has a recorded break attempt.

A fourth round — cloud review — then caught a fifth instance of the same class:
the receipt's "per-Read cost safe mode adds is 0.001 ms" measured one of the
two things that happen per Read and missed the expensive one. The lesson
generalises past instruments to claims: **a number is only as good as the scope
it actually covers**, and I had not checked what `effective_read_threshold`
does beyond the line I measured.

---

## Acceptance summary

| Gate | Verdict | What remains |
| --- | --- | --- |
| Correctness and compatibility | **PARTIAL** | Live host event/output contract not demonstrated. Blocking. |
| Measurement integrity | **PASS** (tooling) | Host usage mapping must be verified and recorded when runs happen. |
| Task outcomes | **NOT RUN** | 40 scored runs. |
| Token benefit and coverage | **NOT RUN** | Depends on the above. |
| Performance | **PARTIAL** | Per-turn `rebuild` 6.072 ms vs 5 ms at 10,000 records; per-Read safe mode 3.725 ms vs a 2 ms typical target (double parse); cross-platform confirmation. |

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
