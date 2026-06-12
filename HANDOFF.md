# smalltoke — Running Handoff Doc

> **Purpose:** If this session dies (rate limit, crash), a fresh session resumes from this file.
> **Last updated:** 2026-06-12 (update this line + the checklist on every task completion)

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
- [ ] Task 2: Payload recorder + real fixtures  ← *has a manual/external step, see below*
- [ ] Task 3: Hook protocol types and fail-open runner
- [ ] Task 4: tree-sitter signature skeleton (stk-sitter)
- [ ] Task 5: Config loading (stk-compress)
- [ ] Task 6: Read compressor end-to-end  ← *includes live-session validation decision point*
- [ ] Task 7: Session ledger (edit protection + savings)
- [ ] Task 8: stk init + stk doctor
- [ ] Task 9: stk stats
- [ ] **Phase 1 dogfood checkpoint** (install in this repo, watch `stk stats`)
- [ ] Task 10: Grep match folding
- [ ] Task 11: Glob tree folding
- [ ] Task 12: Bash noise strip + rtk detection
- [ ] Task 13: Release hardening (CI, README)

## Decision log

| Date | Decision | Why |
|---|---|---|
| 2026-06-12 | PostToolUse `updatedToolOutput`, not PreToolUse substitution | PreToolUse can only modify input — verified against official hook docs |
| 2026-06-12 | Coexist with rtk; Bash compression opt-in only | rtk (~62k stars) owns Bash compression; our gap is Read/Grep/Glob |
| 2026-06-12 | v1 never emits `permissionDecision` | Security scope cut from original proposal |
| 2026-06-12 | tree-sitter deps deferred from Task 1 to Task 4 | Resolve grammar/core version pins once, when actually implementing skeletons; keeps scaffold build trivially green |
| 2026-06-12 | Work directly on `main` | Greenfield, solo repo |

## Blockers / manual steps pending

- **Task 2 Step 5 (fixture capture):** hooks are snapshotted at Claude Code session startup, so a recorder hook added to `.claude/settings.json` mid-session won't fire in the *current* session. Resolution paths, in order of preference:
  1. Drive a **headless** `claude -p` run in a temp project whose settings pre-contain the recorder hook (current session will attempt this).
  2. If headless capture fails: Brandon restarts a session in this repo with the recorder hook installed, performs one large Read, one offset Read, one many-match Grep, one big Glob, one Bash command, then splits `tests/fixtures/raw.jsonl` into the five named fixture files (see plan Task 2 Step 5) and sanitizes paths.
- **Task 6 validation decision point:** after wiring, confirm in a live session that Read output substitution actually reaches the model (docs example shows Bash). If unsupported → STOP, reassess with Brandon (fallback sketch in plan self-review notes).

## Current state notes

(keep this section current — what's half-done, surprising findings, anything a fresh session can't infer from git)

- **Environment:** Rust was not installed on this machine; installed via `brew install rustup` + `rustup default stable` (rustc 1.96.0). cargo lives at `~/.cargo/bin` — shells may need `export PATH="$HOME/.cargo/bin:$PATH"`.
- Task 1 done; starting Task 2 (recorder + fixtures).
