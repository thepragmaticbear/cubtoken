# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`cubtoken` is a single Rust binary (`ctk`) that installs as a Claude Code **PostToolUse hook** and rewrites native tool outputs (`Read`/`Grep`/`Glob`/`Bash`) *before they reach the model*, via the `updatedToolOutput` field. Large file Reads become tree-sitter signature skeletons; Grep folds per file; Glob folds into a directory tree. It targets the gap stream compressors like rtk can't reach — Claude Code's built-in Read/Grep/Glob bypass the Bash tool entirely.

A second host is wired up: **OpenCode**, through `packages/opencode/` (its `tool.execute.after` plugin hook can replace a tool result the same way). Codex CLI and Antigravity cannot host cubtoken — neither one's post-tool hook can replace a tool result at all.

## Commands

```sh
cargo build --release                 # binary at target/release/ctk
cargo install --path crates/ctk-cli   # install ctk onto PATH
cargo test                            # all tests
cargo test -p ctk-compress            # one crate
cargo test --test record              # one integration test file (crates/ctk-cli/tests/record.rs)
cargo test verbatim                   # tests matching a name substring
cargo insta review                    # review/accept snapshot changes (ctk-sitter uses insta)
```

**Verification gate (must pass before claiming done — mirrors CI):**

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

CI runs this matrix on ubuntu + macos. `clippy -- -D warnings` means warnings are build failures; keep it clean.

Runtime commands of the binary itself: `ctk init [--global]` (install hook + write starter `.cubtoken.toml`), `ctk doctor` (health check), `ctk stats` (read savings from the ledger), `ctk hook` (the hook handler — stdin JSON → stdout decision JSON), `ctk record <path>` (append stdin to a JSONL file, used to capture real fixtures).

## Architecture

Four-crate workspace, strictly layered (each depends only on the next):

- **`ctk-cli`** — the `ctk` binary. Subcommands in `main.rs`; `init.rs`/`doctor.rs`/`stats.rs` are the user-facing commands. `hook()` wraps the handler in `catch_unwind` and **always exits 0**, printing nothing on error.
- **`ctk-hook`** — the hook brain. `protocol.rs` types the PostToolUse payload tolerantly (unknown fields stay `Value` so host schema drift can't break parsing). `lib.rs::dispatch` branches on `tool_name` and calls the right compressor. `ledger.rs` is the session store.
- **`ctk-compress`** — per-tool compressors (`read.rs`, `grep.rs`, `glob_fold.rs`, `bash.rs`), plus `config.rs` and `estimate.rs` (cheap ~3.5 chars/token estimator).
- **`ctk-sitter`** — leaf crate: file text → signature skeleton via tree-sitter. Languages: Rust, TypeScript/TSX/JS (one TSX grammar covers ts/tsx/js/jsx/mjs/cjs), Python, Go. Unknown/unparseable files fall back to head+tail elision in `read.rs`.

Two non-Rust install paths sit beside the crates:

- **`plugins/cubtoken/`** — the same PostToolUse hook packaged as a Claude Code plugin (`.claude-plugin/plugin.json` + `hooks/hooks.json` + a `bin/ctk-hook` wrapper that resolves `ctk` at call time). Listed from the repo-root `.claude-plugin/marketplace.json`. The matcher is duplicated in `crates/ctk-cli/tests/plugin.rs` — keep it equal to `init::MATCHER`.
- **`packages/opencode/`** — `@cubtoken/opencode`, an OpenCode plugin that translates OpenCode's `tool.execute.after` payload into the Claude Code hook shape and shells out to `ctk hook`. Covers `read` plus `edit`/`write` protection only. `cargo test --test opencode` runs its self-check.

**Data flow:** Claude runs a matched tool → PostToolUse fires `ctk hook` → `run_hook` parses stdin → `dispatch` returns either a replacement `tool_response` value (wrapped as `updatedToolOutput`) or `None` (pass through). `dispatch` also intercepts `Edit`/`Write`/`NotebookEdit` to record the edited path in the ledger, then returns `None`.

**Ledger** (`<cwd>/.cubtoken/session-<session_id>.jsonl`): append-only JSONL with two record types — `edit` (path protection) and `save` (token savings). Replayed on open. Two jobs: (1) **edit-protection** — a file the model has edited this session is never compressed again (always on, prevents the Edit-correctness hazard); (2) **savings tracking** for `ctk stats` (gated by `stats.ledger`). All ledger I/O is best-effort and degrades silently.

## Invariants (these override convenience — see `docs/bearpaws/plans/2026-06-12-cubtoken-design.md`)

1. **Fail open** — any error, panic, or unparseable input passes the original output through untouched. Every error path in `run_hook`/`dispatch` returns `None`; the CLI catches panics and exits 0. Never let the hook break a session.
2. **Escape hatch** — every compressed view names the exact tool call that retrieves the elided content (e.g. `Read(file_path=…, offset=…, limit=…)`, `[La-Lb]` ranges). The call differs per host, so `CUBTOKEN_HARNESS` (`read.rs::HarnessNames`) switches the spelling; the adapter sets it, not the user.
3. **Verbatim lines** — every source line shown in a skeleton is the exact file text at the stated 1-based line number (the model may quote it in an Edit). Targeted `Read(offset/limit)` calls are never compressed.
4. **Deterministic** — no LLM calls, no network. Same input → same output.

## Non-obvious gotchas

- **Config is strict and loaded per tool call.** `main.rs::hook()` calls `run_hook_auto`, which runs `Config::try_load_for(payload.cwd)` — layered `~/.config/cubtoken/config.toml` ← nearest-ancestor `.cubtoken.toml`. Unknown keys and bad TOML are a hard error there (`deny_unknown_fields`), and the hook passes through rather than compressing with defaults the user tried to disable; `ctk doctor` surfaces the parse error. `Config::load_from`/`load_for` are the lenient variants, for tests only.
- **Real `tool_response` schemas are the ground truth**, captured as fixtures in `tests/fixtures/*.json` via `ctk record`. Don't guess shapes — `read.rs::extract_content` reads `/file/content`; Grep content mode is `relpath:line:text` rows; Bash response has **no exit-code field** (the "don't touch failing commands" gate uses non-empty stderr as the failure heuristic).
- **Bash compression is opt-in (`bash.enabled = false`)** to coexist with rtk, which owns Bash. Leave it off unless deliberately enabled.
- **tree-sitter row gotcha:** a node ending at a newline reports end row = next row, col 0. Comment-adjacency logic in `ctk-sitter` must normalize this.
- **Hooks snapshot at session start.** After `ctk init` or reinstalling, the user must restart their Claude Code session for changes to take effect.
- **OpenCode's read output is shaped differently from Claude Code's.** Claude passes raw source in `/file/content`; OpenCode passes `<path>…</path>\n<type>file</type>\n<content>\n1: line\n…\n\n(note)\n</content>` with the line numbers **inside the string**. `packages/opencode/index.js` strips those prefixes before handing text to `ctk` and re-adds sequential ones afterwards — don't feed numbered text to `ctk-sitter`, it won't parse. Its `parseRead` bails on any unexpected shape rather than guessing, because a wrong guess breaks invariant 3.
- **Two places know the hook matcher.** `init::MATCHER` and `plugins/cubtoken/hooks/hooks.json`. `crates/ctk-cli/tests/plugin.rs::matcher_matches_init` fails if they drift.
- **The on-disk directory is still `~/repos/smalltoke`** (project/binary/crates were renamed smalltoke→cubtoken, stk→ctk, but the dir was intentionally left to preserve the session/memory path). Fixture sample paths still mention `stk-capture` — that's opaque recorded data, leave it.

## Conventions

- **TDD, plan-driven.** Work follows `docs/bearpaws/plans/2026-06-12-cubtoken-mvp.md` (13 tasks, test-first). Write the failing test, then implement. One commit per task/meaningful step, short imperative messages.
- `HANDOFF.md` is a running resume-from-here doc; keep its "Current state notes" and decision log current when state changes in non-obvious ways.
- Tests live both inline (`#[cfg(test)] mod tests`) and as integration tests under each crate's `tests/`. `ctk-sitter` uses `insta` snapshots.
