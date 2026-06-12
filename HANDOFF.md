# smalltoke — Running Handoff Doc

> **Purpose:** If this session dies (rate limit, crash), a fresh session resumes from this file.
> **Last updated:** 2026-06-12 — **ALL 13 TASKS COMPLETE.** Plan fully executed; see 'Where to go next'.

## How to resume

1. Read this file top to bottom.
2. Read the spec: `docs/bearpaws/plans/2026-06-12-smalltoke-design.md` (architecture, invariants, the Edit-correctness hazard).
3. Read the plan: `docs/bearpaws/plans/2026-06-12-smalltoke-mvp.md` (13 TDD tasks with code; checkboxes track step-level progress).
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
- [x] Task 4: tree-sitter signature skeleton (stk-sitter)
- [x] Task 5: Config loading (stk-compress)
- [x] Task 6: Read compressor end-to-end — **live-session validation PASSED** (headless probe confirmed the model receives the substituted skeleton)
- [x] Task 7: Session ledger (edit protection + savings; protection is always-on, only savings recording is gated by stats.ledger)
- [x] Task 8: stk init + stk doctor
- [x] Task 9: stk stats
- [x] **Phase 1 dogfood checkpoint** — hook installed in this repo's `.claude/settings.local.json` (local because the command embeds an absolute binary path; consider an `init --local` flag later). Activates on next session start; check `stk stats` then.
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

## Blockers / manual steps pending

- ~~Task 2 fixture capture~~ **RESOLVED:** headless `claude -p` (haiku, temp project `/tmp/stk-capture`, recorder hook pre-installed in its settings) captured all five payloads. This headless-capture trick is reusable for any future schema question.
- ~~Task 6 validation decision point~~ **RESOLVED:** live headless probe — agent read big.rs through the installed hook and reported COMPRESSED. `updatedToolOutput` substitution works for Read.

## Where to go next (post-MVP)

- **Real-world dogfood:** the hook is live in this repo's `.claude/settings.local.json` from the next session. Watch `stk stats`, and especially watch for failed Edits after compressed Reads (the Edit-hazard mitigations are designed but only proven in tests + one live probe).
- **Phase 4 decision** (spec §8): SQLite AST index + single `get_context` MCP tool — only if dogfooding shows repeated structural re-reads.
- Smaller ideas parked: `init --local` flag (settings.local.json), more languages (Java/C/C++/Ruby), tuning `read.threshold_tokens` from ledger data, publishing to crates.io + GitHub.

## Current state notes

(keep this section current — what's half-done, surprising findings, anything a fresh session can't infer from git)

- **Environment:** Rust was not installed on this machine; installed via `brew install rustup` + `rustup default stable` (rustc 1.96.0). cargo lives at `~/.cargo/bin` — shells may need `export PATH="$HOME/.cargo/bin:$PATH"`.
- **Real `tool_response` schemas** (from fixtures, the ground truth for `extract_content`):
  - `Read`: `{type:"text", file:{filePath, content, numLines, startLine, totalLines}}`
  - `Grep` (content mode): `{mode, numFiles, filenames, content, numLines}` — content rows are `relpath:line:text`
  - `Glob`: `{filenames:[…], durationMs, numFiles, truncated}`
  - `Bash`: `{stdout, stderr, interrupted, isImage, noOutputExpected}` — **no exit-code field**; Task 12's "don't touch failing commands" gate must use stderr-nonempty as the heuristic instead.
- Tasks 1–4 done. Pins: tree-sitter 0.26.9, rust 0.24.2, typescript 0.23.2 (TSX grammar covers ts/js too), python 0.25.0, go 0.25.0. Gotcha discovered: tree-sitter nodes ending at a newline report end row = next row, col 0 — comment adjacency must normalize this. Was: `run_hook` dispatch in `crates/stk-hook/src/lib.rs` returns None for every tool until compressors land. CLI `stk hook` wraps it in catch_unwind, logs to `.smalltoke/errors.log`, always exits 0. Next: Task 4 (tree-sitter skeletons).
