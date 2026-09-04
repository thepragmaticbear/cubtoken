# cubtoken — Running Handoff Doc

> **Purpose:** If this session dies (rate limit, crash), a fresh session resumes from this file.
> **Last updated:** 2026-09-04 — post-MVP review pass: merged the config fix, fixed four correctness bugs found by probing the live hook, added refetch instrumentation. Gate green (66 tests).
>
> **Note:** work now happens in `~/repos/cubtoken`; a `~/repos/smalltoke` copy still exists on disk. The memory dir is still keyed off the `smalltoke` path, so it's unaffected. **Gotcha:** the `cubtoken` checkout inherited `smalltoke`'s `target/`, which baked the old manifest path into `insta`, so `ctk-sitter`'s `rust_skeleton_snapshot` fails in a full `cargo test` (passes in isolation) with a `/Users/brandon/repos/smalltoke/...` path in the error. Fix: `cargo clean` (or `cargo clean -p ctk-sitter`).

## How to resume

1. Read this file top to bottom.
2. Read the spec: `docs/bearpaws/plans/2026-06-12-cubtoken-design.md` (architecture, invariants, the Edit-correctness hazard).
3. Read the plan: `docs/bearpaws/plans/2026-06-12-cubtoken-mvp.md` (13 TDD tasks with code; checkboxes track step-level progress).
4. `git log --oneline` — one commit per task; the last commit tells you where work stopped.
5. Continue from the first unchecked task below, following the plan's steps exactly (test first → fail → implement → pass → commit).

## Working conventions

- **TDD, plan-driven:** every task in the plan has failing-test-first steps. Don't skip verifications.
- **Commit per task** (sometimes per meaningful step), message style: short imperative summary.
- **Working on `main`** — greenfield repo, solo, user-approved scaffold start.
- **Verification gate before claiming done:** `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.
- The four invariants (fail open, escape hatch, verbatim lines, deterministic) override convenience everywhere.

## Progress checklist (mirrors plan tasks)

- [x] Task 1: Workspace scaffold
- [x] Task 2: Payload recorder + real fixtures *(captured autonomously via headless `claude -p`; no manual step needed)*
- [x] Task 3: Hook protocol types and fail-open runner
- [x] Task 4: tree-sitter signature skeleton (ctk-sitter)
- [x] Task 5: Config loading (ctk-compress)
- [x] Task 6: Read compressor end-to-end — **live-session validation PASSED** (headless probe confirmed the model receives the substituted skeleton)
- [x] Task 7: Session ledger (edit protection + savings; protection is always-on, only savings recording is gated by stats.ledger)
- [x] Task 8: ctk init + ctk doctor
- [x] Task 9: ctk stats
- [x] **Phase 1 dogfood checkpoint** — hook installed in this repo's `.claude/settings.local.json` (local because the command embeds an absolute binary path; consider an `init --local` flag later). Activates on next session start; check `ctk stats` then.
- [x] Task 10: Grep match folding
- [x] Task 11: Glob tree folding
- [x] Task 12: Bash noise strip + rtk detection (failure heuristic = non-empty stderr, since Bash responses carry no exit code)
- [x] Task 13: Release hardening (CI, README)

## Decision log

| Date | Decision | Why |
|---|---|---|
| 2026-06-12 | PostToolUse `updatedToolOutput`, not PreToolUse substitution | PreToolUse can only modify input — verified against official hook docs |
| 2026-06-12 | Coexist with rtk; Bash compression opt-in only | rtk (~62k stars) owns Bash compression; our gap is Read/Grep/Glob |
| 2026-06-12 | v1 never emits `permissionDecision` | Security scope cut from original proposal |
| 2026-06-12 | tree-sitter deps deferred from Task 1 to Task 4 | Resolve grammar/core version pins once, when actually implementing skeletons; keeps scaffold build trivially green |
| 2026-06-12 | Work directly on `main` | Greenfield, solo repo |
| 2026-09-04 | Elision markers coalesce rather than stack | A run of closing braces produced `… [L37-L38]` / `… [L39-L40]` back to back; widening one marker is honest and quieter |
| 2026-09-04 | Glob preserves input order, does not sort | The recorded fixture is `big.rs, helper_1…helper_30` — creation order. Glob returns newest-first and that is a signal worth keeping |
| 2026-06-13 | Renamed smalltoke→cubtoken, binary stk→ctk, crates ctk-* | User rename request; `ctk` keeps the abbreviation coherent (cub-to-ken). Directory left as `smalltoke` to avoid disrupting the live session/memory path. Fixtures' `stk-capture` sample paths left untouched (opaque recorded data). |
| 2026-06-15 | Hook loads layered config from the payload `cwd` (`run_hook_auto` + `Config::load_for`) | `main.rs` called `Config::default()`, so `.cubtoken.toml` and the global file were ignored at runtime — every documented setting was inert (Task 5 built the loader but nothing called it). Reading from `payload.cwd` (not process cwd) makes a global hook install honor each project's config and matches where the ledger is written. `run_hook(cfg)` kept for test injection. |

## Blockers / manual steps pending

- ~~Task 2 fixture capture~~ **RESOLVED:** headless `claude -p` (haiku, temp project `/tmp/ctk-capture`, recorder hook pre-installed in its settings) captured all five payloads. This headless-capture trick is reusable for any future schema question.
- ~~Task 6 validation decision point~~ **RESOLVED:** live headless probe — agent read big.rs through the installed hook and reported COMPRESSED. `updatedToolOutput` substitution works for Read.

## Review findings actioned (2026-09-04)

Found by running the installed hook against real payloads, not by reading code.

| Bug | Symptom | Fix |
|---|---|---|
| Double line-number gutter | The Read tool renders its own sequential numbering around our substituted block, so the model saw two conflicting numbers per row | Banner names the outer gutter and tells the model to use the inner one |
| Python decorated defs | `@app.route(...)` shown, `def handler(req):` never shown — the signature skeleton had no signature | `emit_decl` shows every row from wrapper start through the declaration's own start row |
| Rust attributes dropped | `#[derive]`, `#[serde]`, `#[cfg(target_os)]` vanished with no elision marker | `emit_prefix_lines` accepts `attribute_item` alongside comments |
| Unadvertised elisions | Container bodies, closing braces and trailing content disappeared with no `[La-Lb]` range — a direct invariant-2 violation | `Builder.covered` tracks the high-water row; `emit_gap` advertises any non-blank skip. Adjacent markers coalesce so closing-brace runs don't stack |
| Glob expanded its input | 80 files in 80 dirs: 1110 chars → 1201 chars, substituted anyway, ledger recorded `in:318 out:344` | 30% guard (matching read/grep/bash), plus input ordering preserved and a truthful `numFiles` |

Also: `doctor` now searches `settings.local.json` and verifies the embedded binary path still exists; the grep duplicate-fold note carries an escape hatch; `ctk stats` labels its numbers as estimates.

**New instrumentation — read this before tuning anything.** The ledger now records a `refetch` whenever the model issues a targeted `Read(offset/limit)` into a file compressed earlier in the same session. `ctk stats` reports the count. That is the cost side of compression: gross savings were always measurable, net savings were not.

Baseline from ~3 months of real dogfooding (9 sessions across 3 repos), measured during this review:

| tool | firings | est. tokens in | est. tokens out |
|---|---|---|---|
| Read | 13 | 79,976 | 9,897 |
| Grep | 0 | — | — |
| Glob | 0 | — | — |
| Bash | 0 | — | — |

88% savings when it fires — but it fired 13 times in three months, and Grep/Glob/Bash have never fired once. The same ledgers hold **52 Edit records**, so edit-protection suppressed far more Reads than compression captured. Collect refetch data before widening thresholds or adding languages.

## Data-type bodies kept (2026-09-04)

Skeletons elided the one thing the model reads a file *for*. `pub struct Config { … [L3-L6] }` hid four field lines to save four lines — negative value, and a guaranteed refetch. Same for enum variants, TS interface members and Python dataclass fields. Go was already correct (its `type_declaration` has no `body` field to elide), so three of four languages disagreed with the fourth.

| Change | Where |
|---|---|
| `verbatim_body_kinds` — struct/enum/union (Rust), interface/enum (TS) render their body whole | `ctk-sitter/src/lib.rs` |
| `depth == 0` → `depth < MAX_CONTAINER_DEPTH` (3): a `mod`/`namespace`/nested class no longer swallows every member into one marker | `emit_decl` |
| Bare `namespace N {}` arrives wrapped in an `expression_statement` in the TSX grammar (only `export namespace` is an `export_statement`) — `unwrap_decl` now unwraps that one shape | `unwrap_decl` |
| Python `expression_statement` added to `decl_kinds` so dataclass/Pydantic/Django field assignments survive | `decl_kinds` |
| `MAX_VERBATIM_BODY_LINES` (40) caps both verbatim bodies and bodyless decls, so a 300-variant enum or a 200-line lookup table still elides and still advertises its range | `emit_decl` |

Measured: `tests/fixtures/read_large.json` is byte-identical before and after (26,118 → 8,105 chars, 69% saved) — that fixture is all functions, so the change costs nothing where structs are absent and only pays where they are present.

**`body_node` follow-up (same day, from ultrareview).** `body_node` searched *any* named descendant for a `body` field, so a declaration whose value merely *contained* a function reported that inner body as the end of its own signature — `HANDLERS = {"a": lambda x: f(x), ...}` rendered its first line then a misleading `… [L2-L5]`. Pre-existing for Rust `static`/`const` and TS `lexical_declaration`; adding Python `expression_statement` to `decl_kinds` widened it. Now the descent follows only the direct value chain (`value`/`right`, through a lone declarator) and stops at anything not `is_function_like`. The case the descent exists for — `export const handler = async () => {…}`, whose value *is* the function — still elides its body. Removing the recursion outright passes every test but silently regresses that very common TS shape, so it isn't the fix.

Test-fixture gotcha: `crates/ctk-sitter/tests/corpus/sample.rs` is ~3.5KB, and with struct bodies kept the fixed ~600-char banner pushes it past the 30% savings guard — `compress_read` correctly declines. Three `read.rs` unit tests now build their input with `big_sample()` (`SAMPLE.repeat(4)`) rather than loosening the guard.

## Deliberately not done

- **Loosening edit-protection.** Tempting (52 edits vs 13 compressions) but it directly weakens the project's #1 stated hazard mitigation. Needs refetch + failed-Edit data first, not a guess.
- **Grep/Glob threshold tuning.** They have never fired. Log near-misses for a week before picking new numbers; changing them blind trades one unmeasured setting for another.
- **Structural fallback for JSON/YAML/lockfiles.** Head+tail is truncation, not summary (`Cargo.lock` elided 93% and left an arbitrary window). Real feature work, and worth more than adding Java/C — but it is a feature, not a fix.

## Where to go next (post-MVP)

- **Real-world dogfood:** the hook is live in this repo's `.claude/settings.local.json` from the next session. Watch `ctk stats`, and especially watch for failed Edits after compressed Reads (the Edit-hazard mitigations are designed but only proven in tests + one live probe).
- **Phase 4 decision** (spec §8): SQLite AST index + single `get_context` MCP tool — only if dogfooding shows repeated structural re-reads.
- Smaller ideas parked: `init --local` flag (settings.local.json), more languages (Java/C/C++/Ruby), tuning `read.threshold_tokens` from ledger data, publishing to crates.io + GitHub.

## Current state notes

(keep this section current — what's half-done, surprising findings, anything a fresh session can't infer from git)

- **Harness integrations (2026-09-04):** two new install paths beside `ctk init`.
  - `plugins/cubtoken/` — the same PostToolUse hook as a Claude Code plugin, listed from the repo-root `.claude-plugin/marketplace.json`. `bin/ctk-hook` resolves `ctk` at call time (PATH, then `~/.cargo/bin`, `~/.local/bin`, `/usr/local/bin`) instead of baking an absolute path, and drains stdin + exits 0 when it finds nothing. Its matcher is duplicated in `crates/ctk-cli/tests/plugin.rs`; `matcher_matches_init` is what catches drift from `init::MATCHER`.
  - `packages/opencode/` — npm plugin for OpenCode. Verified at source (`packages/opencode/src/session/tools.ts` on `dev`): the object handed to `tool.execute.after` is the one returned to the model, so mutating `output.output` is the `updatedToolOutput` equivalent. Scope is deliberately `read` + `edit`/`write` protection only.
  - **OpenCode's read shape differs from Claude Code's.** Claude passes raw source in `/file/content`; OpenCode wraps it as `<path>…</path>\n<type>file</type>\n<content>\n1: line\n…\n\n(note)\n</content>` with the line numbers **in the string**. The adapter peels the prefixes off before handing text to `ctk` and re-adds sequential ones after, which keeps cubtoken's "ignore the tool's own numbering" header true in both hosts.
  - **`CUBTOKEN_HARNESS` env var** (`read.rs::HarnessNames`) switches the escape hatch between `Read(file_path=…)` and `read(filePath=…)`. Invariant 2 requires naming a call the host actually has; without it the OpenCode view pointed at a nonexistent tool. Env var rather than a config key because the adapter sets it, not the user. Tested in its own process (`crates/ctk-compress/tests/harness_names.rs`) since the var is process-global.
  - **Codex CLI and Antigravity are blocked, not deferred.** Codex `PostToolUse` returns only `systemMessage`/`continue`/`stopReason` — no output replacement at all, so its shell-first tool inventory is the *second* problem, not the first. Antigravity's `PostToolUse` gets `stepIdx` + `error` and must print `{}`; it isn't told which tool ran. Antigravity's `PreToolUse` *can* rewrite args via `overwrite`, so clamping `view_file` to a line range is the only lever there.
  - **Untested:** nobody has run the OpenCode plugin inside a real OpenCode session yet — only the self-check (`packages/opencode/test.js`, run by `cargo test --test opencode`). Savings there are projected from Claude Code, not measured. Note OpenCode's `read` already caps at 2000 lines / 2000 chars per line / 50 KB, so per-read headroom is smaller than Claude Code's.
  - **Possibly-dead code:** Claude Code now fires `PostToolUseFailure` for failed tool calls, so `PostToolUse` may never see a non-zero-exit Bash. If so the stderr-nonempty failure heuristic in `dispatch` is unreachable. Confirm with `ctk record` against a failing command before removing it.

- **Config wiring (2026-06-15):** the CLI hook now calls `ctk_hook::run_hook_auto`, which loads global + `<payload.cwd>/.cubtoken.toml` via `Config::load_for`. Config is read fresh per tool call, so config edits need no session restart (only `ctk init` does, since the hook entry is snapshotted at session start). Regression test: `run_hook_auto_loads_project_config_from_cwd` in `crates/ctk-hook/tests/end_to_end.rs`. Also made `doctor_passes_after_init_and_fails_before` hermetic (it sets `HOME` to a temp dir) so a developer's real global install no longer fails it.
- **Environment:** Rust was not installed on this machine; installed via Homebrew `rustup` (rustc/cargo 1.96.0). Homebrew links only `rustup` into `/opt/homebrew/bin`; the `cargo`/`rustc` proxies live in `/opt/homebrew/opt/rustup/bin`, and `cargo install` drops binaries in `~/.cargo/bin`. `~/.zshrc` now puts both on PATH: `export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/rustup/bin:$PATH"`.
- **Real `tool_response` schemas** (from fixtures, the ground truth for `extract_content`):
  - `Read`: `{type:"text", file:{filePath, content, numLines, startLine, totalLines}}`
  - `Grep` (content mode): `{mode, numFiles, filenames, content, numLines}` — content rows are `relpath:line:text`
  - `Glob`: `{filenames:[…], durationMs, numFiles, truncated}`
  - `Bash`: `{stdout, stderr, interrupted, isImage, noOutputExpected}` — **no exit-code field**; Task 12's "don't touch failing commands" gate must use stderr-nonempty as the heuristic instead.
- Hook overhead measured: <10ms warm (release binary, read_large fixture). Tasks 1–4 were: Pins: tree-sitter 0.26.9, rust 0.24.2, typescript 0.23.2 (TSX grammar covers ts/js too), python 0.25.0, go 0.25.0. Gotcha discovered: tree-sitter nodes ending at a newline report end row = next row, col 0 — comment adjacency must normalize this. Was: `run_hook` dispatch in `crates/ctk-hook/src/lib.rs` returns None for every tool until compressors land. CLI `ctk hook` wraps it in catch_unwind, logs to `.cubtoken/errors.log`, always exits 0. Next: Task 4 (tree-sitter skeletons).
