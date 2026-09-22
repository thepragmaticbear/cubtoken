# Adaptive governor validation receipt

**Verdict: not ready to ship safe mode.** Of five acceptance gates, one passes
(measurement integrity, tooling only), two are partial, and two have not run:

- **Correctness** is partial. The live event contract is now established on
  Claude Code 2.1.280, including a high-confidence recovery recorded in a real
  session. What's left is the live edit-protection smoke test and raw payload
  fixtures for the new events.
- **Performance** is partial. Per-Read overhead passes. The per-turn rebuild
  misses its target at 10,000 ledger records.
- **Task outcomes** and **token benefit** have not run. Token benefit is the
  question this whole evaluation exists to answer.

An earlier version of this paragraph said "three of five gates are met". The
gate table never supported that, and it has been corrected.

Nothing here authorizes a push, a merge, or a default change. Adaptive mode
stays `off` by default regardless of outcome.

| Field | Value |
| --- | --- |
| Governor as first written | `d1a78fe299f34591e8d83f9c06bc65e1ebbe2bc2` |
| Commit under test (runtime) | `acce8955956946edc73b96ca9377746106ee8ea8` |
| Branches | `feat/adaptive-read-governor` (runtime) and `feat/adaptive-governor-evaluation` (this evaluation, stacked on it) |
| Execution date | 2026-09-22 (UTC) |
| Machine | Apple M5, arm64, macOS 27.0 |
| Toolchain | rustc 1.96.0, cargo 1.96.0, Python 3.14.7 |
| `ctk` version | 0.1.3 |

These SHAs are from the pull requests' branches. If the PRs are squash-merged they won't be on `main`, but GitHub keeps them reachable through each PR's `refs/pull/<n>/head`.

The baseline commit is the in-progress governor imported verbatim from the
dirty `main` checkout, so scored runs can name an immutable implementation.
Later commits add acceptance coverage, the analysis contract, the performance
drivers, the task matrix, and fixes for four defects found by adversarial
review. **The comparison baseline is therefore `acce895`, not `d1a78fe`** —
runtime behaviour changed. No run had been scored against the older one, so
nothing is invalidated, but any further runtime change moves it again.

---

## Gate 1 — Correctness and compatibility: PARTIAL

### Repository verification: PASS

Run at `acce895` (runtime) and at the head of the evaluation branch, on the machine above.

| Command | Result |
| --- | --- |
| `cargo fmt --check` | exit 0 |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | exit 0 |
| `cargo test --locked --workspace --all-features` | exit 0 — **151 passed, 0 failed, 2 ignored** |
| `cargo audit --deny warnings` | exit 0 |
| `python3 scripts/evaluate_governor.py --self-test` | exit 0 — **25/25 checks** |

The two ignored tests are the performance drivers, run explicitly in gate 5.
Test count rose from 134 at the frozen baseline to 151.

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

### Live host contract: ESTABLISHED for attribution (Claude Code 2.1.280)

Observed on 2026-09-22 in real sessions on Claude Code **2.1.280**, macOS, with
the plugin at `78c4434`. The evidence is ledger records written by the
installed hook as it responded to events Claude Code actually sent:

| Requirement | Evidence |
| --- | --- |
| `UserPromptSubmit` is delivered | `turn` records in three separate sessions |
| `PostToolBatch` is delivered | 11 `batch` records in one session's first turn; 2 in another |
| `Stop` is delivered | `output` records |
| The replacement output is accepted | A full Read of `scripts/build-dashboard-data.ts` (8,524 estimated tokens) reached the model as a 3,614-token skeleton, and the model quoted its elided range `L357–581` back |
| Attribution works live | A targeted `Read(offset=431, limit=16)` into that elided range, in batch 1 of the same turn, was recorded as a **high-confidence** targeted recovery linked to the decision |

The attribution sequence, exactly as recorded:

```text
turn t1
save      Read 8524 -> 3614 tokens, decision at t1 b0, typescript|skeleton
batch     t1 b1
recovery  targeted, confidence high, t1 b1, 131 tokens
batch     t1 b2
output
```

Not yet demonstrated:

- `SessionStart`, `SessionEnd`, and `StopFailure` specifically. None of them
  leaves an event-specific ledger record, and one adaptive-state write came
  from either `Stop` or `SessionEnd` without showing which.
- The edit-protection half of the smoke procedure in a live session: an Edit
  followed by a Read that passes through.
- Raw, redacted payload fixtures of the new events captured with `ctk record`.
  The evidence above is the hook's own reaction to those events, which proves
  delivery but isn't a payload contract test.

### Findings from live sessions

**Per-session worktrees fragment the governor.** The Claude Code desktop app
ran each session in its own git worktree under `.claude/worktrees/`. A worktree
has its own `.git`, so cubtoken treats it as a separate project, and two things
follow:

- An **untracked** `.cubtoken.toml` at the repository root isn't visible in the
  worktree. The root reported `mode = "observe"`, while the worktree the test
  ran in reported `mode = "off"`. Decisions and recoveries were recorded only
  because statistics were on.
- Each worktree keeps its own `.cubtoken/`. Observations never pool across
  sessions, a bucket has to reach its 8-observation minimum inside one session,
  and discarding the worktree deletes what was learned.

The configuration half is solved by a global `~/.config/cubtoken/config.toml`
or a committed `.cubtoken.toml`. The pooling half is an open design question.

**Agents in auto mode read through Bash (n = 1).** An over-engineering audit of
the same repository made **no** Read calls. It read about 68 KB through 13 Bash
calls instead: one multi-file `cat`, hand-built outlines from `grep -nE
"^export|^function"`, and targeted `sed -n a,bp` ranges. Those `sed` ranges are
recoveries in all but name, but they go through a tool cubtoken leaves to rtk,
so the governor sees none of them. Auto mode's instructions tell agents to read
with `cat`, `head`, and `sed`, which likely explains this. It's one session, not
a measured trend, but if it holds, it limits how much Read traffic cubtoken, and
the governor with it, ever gets to act on.

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

Measured on source byte-identical to the head of the evaluation branch, release build. Fixture sizes **100 and 1,000 records
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
| 100 | typical | replay (Ledger::open) | 0.111 | 0.129 | 0.156 | 200 |
| 100 | typical | policy load (adaptive load) | 0.001 | 0.001 | 0.001 | 200 |
| 100 | typical | explain preview (tree-sitter) | 1.445 | 1.493 | 1.579 | 200 |
| 100 | typical | lookup (is_protected) | 0.000 | 0.000 | 0.000 | 2000 |
| 100 | typical | attribution (classify) | 0.000 | 0.000 | 0.000 | 2000 |
| 100 | typical | append (note_saving) | 0.078 | 0.103 | 0.145 | 200 |
| 100 | typical | rebuild (adaptive refresh) | 0.271 | 0.283 | 0.343 | 200 |
| 1000 | typical | replay (Ledger::open) | 0.407 | 0.423 | 0.458 | 200 |
| 1000 | typical | policy load (adaptive load) | 0.001 | 0.001 | 0.001 | 200 |
| 1000 | typical | explain preview (tree-sitter) | 1.439 | 1.493 | 1.552 | 200 |
| 1000 | typical | lookup (is_protected) | 0.000 | 0.000 | 0.000 | 2000 |
| 1000 | typical | attribution (classify) | 0.000 | 0.000 | 0.000 | 2000 |
| 1000 | typical | append (note_saving) | 0.080 | 0.108 | 0.127 | 200 |
| 1000 | typical | rebuild (adaptive refresh) | 0.776 | 0.805 | 0.847 | 200 |
| 5000 | large | replay (Ledger::open) | 1.824 | 1.871 | 1.911 | 100 |
| 5000 | large | policy load (adaptive load) | 0.001 | 0.001 | 0.001 | 100 |
| 5000 | large | explain preview (tree-sitter) | 1.442 | 1.494 | 1.528 | 100 |
| 5000 | large | lookup (is_protected) | 0.000 | 0.000 | 0.000 | 1000 |
| 5000 | large | attribution (classify) | 0.000 | 0.000 | 0.000 | 1000 |
| 5000 | large | append (note_saving) | 0.074 | 0.091 | 0.111 | 100 |
| 5000 | large | rebuild (adaptive refresh) | 3.080 | 3.122 | 3.192 | 100 |
| 10000 | large | replay (Ledger::open) | 3.629 | 3.686 | 3.742 | 100 |
| 10000 | large | policy load (adaptive load) | 0.001 | 0.001 | 0.001 | 100 |
| 10000 | large | explain preview (tree-sitter) | 1.439 | 1.503 | 1.553 | 100 |
| 10000 | large | lookup (is_protected) | 0.000 | 0.000 | 0.000 | 1000 |
| 10000 | large | attribution (classify) | 0.000 | 0.000 | 0.000 | 1000 |
| 10000 | large | append (note_saving) | 0.077 | 0.099 | 0.120 | 100 |
| 10000 | large | rebuild (adaptive refresh) | 5.973 | 6.060 | 6.110 | 100 |
| 0 | per-Read | under-threshold Read, static | 0.080 | 0.087 | 0.124 | 200 |
| 0 | per-Read | under-threshold Read, safe | 0.080 | 0.086 | 0.119 | 200 |
| 0 | per-Read | whole Read through run_hook, static | 1.666 | 1.780 | 1.886 | 200 |
| 0 | per-Read | whole Read through run_hook, safe | 1.836 | 1.978 | 2.057 | 200 |

The `explain preview` row is no longer on the Read path. Only
`ctk adaptive explain` calls it now; the hook parses once, inside
`ReadCandidate::parse`, and gives that parse to the compressor.

### Verdict by what the target covers

The spec's targets are for **governor overhead**, meaning what adaptive mode
adds over static. The compressor's own work runs whether or not this feature
exists, so a whole-Read total is not the figure to judge. Per-Read overhead is
measured directly as safe minus static on the same compressed Read.

| Governor cost | Frequency | Typical p95 | Large p95 | Target | Verdict |
| --- | --- | --- | --- | --- | --- |
| Per Read (safe minus static) | every matched Read | 0.171 ms | same | 2 / 5 ms | **PASS** |
| Per turn (rebuild) | `Stop`, `SessionEnd` | 0.847 ms | 6.11 ms at 10,000 | 2 / 5 ms | **FAIL at 10,000** |

For context, the whole compressed Read is 1.89 ms p95 static and
2.06 ms safe. A large Read below the threshold is 0.124 ms static
and 0.119 ms safe, so it carries no penalty.

### Process cost, excluded from the governor targets

| operation | min ms | median ms | p95 ms | iterations |
| --- | --- | --- | --- | --- |
| startup (ctk --version) | 1.257 | 1.376 | 2.231 | 100 |
| end-to-end (ctk hook), 1,000 ledger records | 3.704 | 3.900 | 4.021 | 100 |

Startup is 1.38 ms median of the 3.9 ms end-to-end figure,
so at 1,000 records the hook's work beyond spawning a process is about
2.5 ms. This is reported separately so that
process-spawn cost is not counted as governor cost.

### A retracted claim, and both costs behind it fixed

An earlier version of this receipt said "`policy load` — the per-Read cost safe
mode adds — is 0.001 ms at every size". **That was wrong.** It measured the
JSON state snapshot and missed the tree-sitter parse next to it:
`effective_read_threshold` called `preview_read`, which parsed with no size
gate. Cloud review caught it. That one number hid two costs, and both are now
fixed:

1. **Reads under the threshold paid for a parse they could never use.** Every
   recommendation only raises the threshold, so a Read under the configured
   one cannot compress no matter what the policy says. The parse is now
   skipped.
2. **Compressed Reads were parsed twice**: once for the preview and again in
   the compressor. `read.rs` now separates the cheap guards
   (`read_candidate`) from the parse (`ReadCandidate::parse`). The hook picks
   its threshold from that single parse and compresses with it.

| Per-Read p95 | static | safe | governor overhead |
| --- | --- | --- | --- |
| Large file under a raised threshold, before fix 1 | 0.124 ms | 1.643 ms | 1.519 ms |
| Compressed Read, before fix 2 | 1.967 ms | 3.725 ms | 1.758 ms |
| Compressed Read, after both | 1.89 ms | 2.06 ms | **0.171 ms** |

### The remaining miss

`rebuild` at 10,000 records is **6.11 ms p95 against a 5 ms target**:
min 5.97, median 6.06. It is consistently over, not a tail
artifact. At 5,000 records it is 3.19 ms and passes.

The cost is JSON parsing, not the algorithm. `replay` alone takes
3.69 ms median at the same size. `refresh` walks **every**
`session-*.jsonl` in the project through `Ledger::load_all`, so it scales with
the whole `.cubtoken/` directory rather than the current session, and nothing
prunes that directory. A long-lived project will grow past this. It runs once
per turn, not once per tool call, and only when adaptive mode is on and
statistics are enabled.

No index or snapshot was added, because the spec says not to add one unless a
measurement requires it. This measurement does require one, but the fix is a
design choice: bounding what `load_all` reads needs a retention policy or a
per-session summary, and outcomes would need decision ids so that a re-parsed
session file replaces its old contribution instead of duplicating it. That is a
state schema change. It belongs in its own reviewed commit, so this number is
reported against the stated boundary and the boundary has not been moved to
make it pass.

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
| Correctness and compatibility | **PARTIAL** | Live event contract established on Claude Code 2.1.280. Remaining: live edit-protection smoke test; raw payload fixtures for the new events. |
| Measurement integrity | **PASS** (tooling) | Host usage mapping must be verified and recorded when runs happen. |
| Task outcomes | **NOT RUN** | 40 scored runs. |
| Token benefit and coverage | **NOT RUN** | Depends on the above. |
| Performance | **PARTIAL** | Per-Read overhead passes (0.171 ms). Per-turn `rebuild` is 6.11 ms against 5 ms at 10,000 records and needs a retention design. Cross-platform confirmation is still outstanding. |

## Recommendation

**Don't treat `safe` mode as a supported feature yet.** It is merged, off by
default, and documented as experimental. Two paths remain, as the spec frames
them:

1. **Keep it experimental** until the live edit-protection smoke test passes, the 40 runs are
   executed, and the rebuild bottleneck is either closed or the large-session
   fixture boundary is re-argued on evidence rather than convenience.
2. **Narrow the release to observation-only**: withdraw `safe` and keep `off`
   and `observe`, and review that change as its own commit. This does **not** avoid the rebuild miss:
   `refresh` runs on every `Stop` whenever the mode is not `off`, so `observe`
   pays the same per-turn cost as `safe`. It avoids only the correctness risk
   of acting on the policy.

Whichever path is chosen, `185f0ca` (in #24) should be reviewed on its own. It
contains the two Critical ledger fixes, which change how edit protection
records its state. They landed partway through the code review, so no one
other than their author has examined them.

The existing safe-mode shipping criteria are not optional merely because the
feature is opt-in.

The evaluation tooling, task matrix, and acceptance coverage are merged (#24,
#25). Merging them was not a decision to ship `safe`: that decision still rests
on the evidence this document tracks.

## Reproducing the local artifacts

The commands above are the whole receipt; there are no hidden inputs. Raw
transcripts and usage exports are deliberately **not** in version control: they
can contain source, command output, paths, and secrets. When runs are executed,
keep them outside the repository and cite their paths in each record's
`evidence_path`, `artifact_path`, and `status_*_path` fields, as
[README.md](README.md) section 6 describes.
