# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`cubtoken` is a single Rust binary (`ctk`) that installs as a Claude Code **PostToolUse hook** and rewrites native tool outputs (`Read`/`Grep`/`Glob`/`Bash`) *before they reach the model*, via the `updatedToolOutput` field. Large file Reads become tree-sitter signature skeletons; Grep folds per file; Glob folds into a directory tree. It targets the gap stream compressors like rtk can't reach — Claude Code's built-in Read/Grep/Glob bypass the Bash tool entirely.

## Commands

```sh
cargo build --locked --release        # binary at target/release/ctk
cargo install --locked --path crates/ctk-cli # install ctk onto PATH
cargo test --locked --workspace       # all tests
cargo test -p ctk-compress            # one crate
cargo test --test record              # one integration test file (crates/ctk-cli/tests/record.rs)
cargo test verbatim                   # tests matching a name substring
cargo insta review                    # review/accept snapshot changes (ctk-sitter uses insta)
```

**Verification gate (must pass before claiming done — mirrors CI):**

```sh
cargo fmt --check && cargo clippy --locked --workspace --all-targets --all-features -- -D warnings && cargo test --locked --workspace --all-features && cargo audit --deny warnings
```

CI runs this matrix on Ubuntu, macOS, and Windows. `clippy -- -D warnings` means warnings are build failures; keep it clean.

Runtime commands of the binary itself: `ctk init [--global]` (install hook + write starter `.cubtoken.toml`), `ctk doctor` (health check), `ctk stats` (read savings from the ledger), `ctk hook` (the hook handler — stdin JSON → stdout decision JSON), `ctk record <path>` (append stdin to a JSONL file, used to capture real fixtures).

## Architecture

Four-crate workspace, strictly layered (each depends only on the next):

- **`ctk-cli`** — the `ctk` binary. Subcommands in `main.rs`; `init.rs`/`doctor.rs`/`stats.rs` are the user-facing commands. `hook()` wraps the handler in `catch_unwind` and **always exits 0**, printing nothing on error.
- **`ctk-hook`** — the hook brain. `protocol.rs` types the PostToolUse payload tolerantly (unknown fields stay `Value` so host schema drift can't break parsing). `lib.rs::dispatch` branches on `tool_name` and calls the right compressor. `ledger.rs` is the session store.
- **`ctk-compress`** — per-tool compressors (`read.rs`, `grep.rs`, `glob_fold.rs`, `bash.rs`), plus `config.rs` and `estimate.rs` (cheap ~3.5 chars/token estimator).
- **`ctk-sitter`** — leaf crate: file text → signature skeleton via tree-sitter. Languages: Rust, TypeScript/TSX/JS (one TSX grammar covers ts/tsx/js/jsx/mjs/cjs), Python, Go. Unknown/unparseable files fall back to head+tail elision in `read.rs`.

**Data flow:** Claude runs a matched tool → PostToolUse fires `ctk hook` → `run_hook` parses stdin → `dispatch` returns either a replacement `tool_response` value (wrapped as `updatedToolOutput`) or `None` (pass through). `dispatch` also intercepts `Edit`/`Write`/`NotebookEdit` to record the edited path in the ledger, then returns `None`.

**Ledger** (`<cwd>/.cubtoken/session-<session_id>.jsonl`): append-only JSONL with two record types — `edit` (path protection) and `save` (token savings). Replayed on open. Two jobs: (1) **edit-protection** — a file the model has edited this session is never compressed again (always on, prevents the Edit-correctness hazard); (2) **savings tracking** for `ctk stats` (gated by `stats.ledger`). All ledger I/O is best-effort and degrades silently.

## Invariants

1. **Fail open** — any error, panic, or unparseable input passes the original output through untouched. Every error path in `run_hook`/`dispatch` returns `None`; the CLI catches panics and exits 0. Never let the hook break a session.
2. **Escape hatch** — every compressed view names the exact tool call that retrieves the elided content (e.g. `Read(file_path=…, offset=…, limit=…)`, `[La-Lb]` ranges).
3. **Verbatim lines** — every source line shown in a skeleton is the exact file text at the stated 1-based line number (the model may quote it in an Edit). Targeted `Read(offset/limit)` calls are never compressed.
4. **Deterministic** — no LLM calls, no network. Same input → same output.

## Non-obvious gotchas

- **Config is strict and loaded per tool call.** `main.rs::hook()` calls `run_hook_auto`, which runs `Config::try_load_for(payload.cwd)` — layered `~/.config/cubtoken/config.toml` ← nearest-ancestor `.cubtoken.toml`. Unknown keys and bad TOML are a hard error there (`deny_unknown_fields`), and the hook passes through rather than compressing with defaults the user tried to disable; `ctk doctor` surfaces the parse error. `Config::load_from`/`load_for` are the lenient variants, for tests only.
- **Real `tool_response` schemas are the ground truth**, captured as fixtures in `tests/fixtures/*.json` via `ctk record`. Don't guess shapes — `read.rs::extract_content` reads `/file/content`; Grep content mode is `relpath:line:text` rows; Bash response has **no exit-code field** (the "don't touch failing commands" gate uses non-empty stderr as the failure heuristic).
- **Bash compression is opt-in (`bash.enabled = false`)** to coexist with rtk, which owns Bash. Leave it off unless deliberately enabled.
- **tree-sitter row gotcha:** a node ending at a newline reports end row = next row, col 0. Comment-adjacency logic in `ctk-sitter` must normalize this.
- **Hooks snapshot at session start.** After `ctk init` or reinstalling, the user must restart their Claude Code session for changes to take effect.

## Conventions

- **TDD.** Write the failing test, then implement. One commit per meaningful step, short imperative messages.
- Tests live both inline (`#[cfg(test)] mod tests`) and as integration tests under each crate's `tests/`. `ctk-sitter` uses `insta` snapshots.
