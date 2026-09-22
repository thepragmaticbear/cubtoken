# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`cubtoken` is a single Rust binary (`ctk`) that installs as a Claude Code **PostToolUse hook** and rewrites native tool outputs (`Read`/`Grep`/`Glob`/`Bash`) *before they reach the model*, via the `updatedToolOutput` field. Large file Reads become tree-sitter signature skeletons; Grep folds per file; Glob folds into a directory tree. It targets the gap stream compressors like rtk can't reach — Claude Code's built-in Read/Grep/Glob bypass the Bash tool entirely.

**Claude Code is the only supported host.** An OpenCode adapter existed and was removed; don't reintroduce multi-host plumbing (a `CUBTOKEN_HARNESS` env var, per-host tool-name spellings, JS shims) without a deliberate decision to reverse that.

Codex CLI and Antigravity are **blocked, not deferred** — recorded here so it isn't re-researched. Codex's `PostToolUse` returns only `systemMessage` / `continue` / `stopReason`, so it can add text but never replace a result; its shell-first tool inventory is the second problem, not the first. Antigravity's `PostToolUse` receives `stepIdx` + `error` and must print `{}` — it is never told which tool ran. Antigravity's `PreToolUse` can rewrite args via `overwrite`, so clamping `view_file` to a line range is the only lever there.

## Commands

```sh
cargo build --locked --release        # binary at target/release/ctk
cargo install --locked --path crates/ctk-cli # install ctk onto PATH
cargo test --locked --workspace       # all tests
cargo test --locked -p ctk-compress   # one crate
cargo test --locked -p ctk-hook --test end_to_end # recorded hook payloads
cargo test --locked verbatim          # tests matching a name substring
cargo insta review                    # review/accept snapshot changes (ctk-sitter uses insta)
```

**Verification gate (must pass before claiming done — mirrors CI):**

```sh
cargo fmt --check && cargo clippy --locked --workspace --all-targets --all-features -- -D warnings && cargo test --locked --workspace --all-features && cargo audit --deny warnings
```

CI runs this matrix on Ubuntu, macOS, and Windows. `clippy -- -D warnings` means warnings are build failures; keep it clean.

### Compression test plan

1. **Fast loop — representation.** Run `cargo test --locked -p ctk-compress -p ctk-sitter`. Cover the changed compressor plus threshold/no-benefit pass-through, response shape, parser fallback, verbatim source lines, and advertised elision ranges. Review `insta` diffs line by line; accept them only when the representation change is intentional.
2. **Hook behavior — safety and recovery.** Run `cargo test --locked -p ctk-hook`. `tests/end_to_end.rs` exercises recorded Claude payloads; `tests/ledger.rs` and `tests/adaptive.rs` cover targeted and edited-file pass-through, lock contention, one full-repeat escape per decision, adaptive bucket selection, and concurrent snapshots.
3. **Host contract — real payloads.** If Claude changes a tool response, capture it with `ctk record`, redact it, add the fixture under `tests/fixtures/`, write the failing contract test, then change the parser. Never invent a payload shape from documentation.
4. **Release confidence — full gate and one smoke session.** Run the verification gate above, install the release binary, restart Claude Code, then verify: a large untouched Read shows one cubtoken banner; its advertised targeted Read passes through; an Edit followed by Read passes through; and `ctk stats --json` records the decision. Record before/after token estimates for the same Rust, TSX, Python, Go, and unparseable samples when changing compression quality.

Acceptance is invariant-first: no lost source lines outside advertised ranges, no rewritten displayed lines, no compression when safety state is unavailable, and no substituted output unless it is at least 30% smaller. Savings percentages are evidence, not golden tests.

Runtime commands of the binary itself: `ctk init [--global]` (install hook + write starter `.cubtoken.toml`), `ctk uninstall [--global]` (remove the hook; never touches `.cubtoken.toml`/`.cubtoken/`), `ctk doctor` (health check), `ctk stats [--json]` (read savings from the ledger), `ctk adaptive status|explain|reset` (inspect/reset the Read governor), `ctk output status` (inspect output mode), `ctk hook` (the hook handler — stdin JSON → stdout decision JSON), `ctk record <path>` (append stdin to a JSONL file, used to capture real fixtures).

## Architecture

Four-crate workspace, strictly layered (each depends only on the next):

- **`ctk-cli`** — the `ctk` binary. Subcommands in `main.rs`; `init.rs`/`doctor.rs`/`stats.rs`/`adaptive.rs` are the user-facing commands. `hook()` wraps the handler in `catch_unwind` and **always exits 0**, printing nothing on error; diagnostic commands return nonzero on failure.
- **`ctk-hook`** — the hook brain. `protocol.rs` types the PostToolUse payload tolerantly (unknown fields stay `Value` so host schema drift can't break parsing). `lib.rs::dispatch` branches on `tool_name` and calls the right compressor. `ledger.rs` is the session store.
- **`ctk-compress`** — per-tool compressors (`read.rs`, `grep.rs`, `glob_fold.rs`, `bash.rs`), plus `config.rs` and `estimate.rs` (cheap ~3.5 chars/token estimator).
- **`ctk-sitter`** — leaf crate: file text → signature skeleton via tree-sitter. Languages: Rust, TypeScript/TSX/JS (one TSX grammar covers ts/tsx/js/jsx/mjs/cjs), Python, Go. Unknown/unparseable files fall back to head+tail elision in `read.rs`.

One non-Rust install path sits beside the crates:

- **`plugins/cubtoken/`** — the same PostToolUse hook packaged as a Claude Code plugin (`.claude-plugin/plugin.json` + `hooks/hooks.json` + a `bin/ctk-hook` wrapper that resolves `ctk` at call time). Listed from the repo-root `.claude-plugin/marketplace.json`. The matcher is duplicated in `crates/ctk-cli/tests/plugin.rs` — keep it equal to `init::MATCHER`.

**Data flow:** Claude runs a matched tool → PostToolUse fires `ctk hook` → `run_hook` parses stdin → `dispatch` returns either a replacement `tool_response` value (wrapped as `updatedToolOutput`) or `None` (pass through). `dispatch` also intercepts `Edit`/`Write`/`NotebookEdit` to record the edited path in the ledger, then returns `None`.

**Ledger** (`<cwd>/.cubtoken/session-<session_id>.jsonl`): locked, append-only JSONL recording edits, compression decisions, recoveries, turn/batch boundaries, and visible output totals. Replayed on open. It protects edited files, supplies `ctk stats`, and feeds the adaptive Read governor. If the session lock is unavailable, a Read passes through rather than acting on stale state. Writes are best-effort and hook failures remain silent — **except edit records, which `append_durable` writes whether or not the lock is held.** That is deliberate: every hook call is a fresh process, so an edit dropped for want of the lock would leave an edited file compressible for the rest of the session. Don't route edits back through the locked `append`; `tests/defects.rs` fails if you do. It also refuses to append through a symlinked session file.

## Invariants

1. **Fail open** — any error, panic, or unparseable input passes the original output through untouched. Every error path in `run_hook`/`dispatch` returns `None`; the CLI catches panics and exits 0. Never let the hook break a session.
2. **Escape hatch** — every compressed view names the exact tool call that retrieves the elided content (e.g. `Read(file_path=…, offset=…, limit=…)`, `[La-Lb]` ranges), spelled exactly as Claude Code spells it.
3. **Verbatim lines** — every source line shown in a skeleton is the exact file text at the stated 1-based line number (the model may quote it in an Edit). Targeted `Read(offset/limit)` calls are never compressed.
4. **Deterministic** — no LLM calls, no network. Same input → same output.

## Non-obvious gotchas

- **Config is strict and loaded per tool call.** `main.rs::hook()` calls `run_hook_auto`, which runs `Config::try_load_for(payload.cwd)` — layered `~/.config/cubtoken/config.toml` ← nearest-ancestor `.cubtoken.toml`. Unknown keys and bad TOML are a hard error there (`deny_unknown_fields`), and the hook passes through rather than compressing with defaults the user tried to disable; `ctk doctor`, adaptive diagnostics, and `ctk output status` surface the parse error. `Config::load_from`/`load_for` are the lenient variants, for tests only.
- **Real `tool_response` schemas are the ground truth**, captured as fixtures in `tests/fixtures/*.json` via `ctk record`. Don't guess shapes — `read.rs::extract_content` reads `/file/content`; Grep content mode is `relpath:line:text` rows; Bash response has **no exit-code field** (the "don't touch failing commands" gate uses non-empty stderr as the failure heuristic).
- **Bash compression is opt-in (`bash.enabled = false`)** to coexist with rtk, which owns Bash. Leave it off unless deliberately enabled.
- **tree-sitter row gotcha:** a node ending at a newline reports end row = next row, col 0. Comment-adjacency logic in `ctk-sitter` must normalize this.
- **Hooks snapshot at session start.** After `ctk init` or reinstalling, the user must restart their Claude Code session for changes to take effect.
- **Two places know the hook matcher.** `init::MATCHER` and `plugins/cubtoken/hooks/hooks.json`. `crates/ctk-cli/tests/plugin.rs::matcher_matches_init` fails if they drift.
- **Never put `archive: false` on `upload-artifact` in `release.yml`.** It reads as "don't double-zip an already-packaged file", but it makes the uploaded `.zip` *itself* the artifact container, so `download-artifact` extracts it — v0.1.0 shipped loose `ctk.exe`/`LICENSE`/`README.md` instead of `ctk-x86_64-pc-windows-msvc.zip`. A `.tar.gz` is not a zip container, so the Linux and macOS targets look fine and hide it. **CI never runs `release.yml`** (it triggers on `v*` tags only), so nothing catches a release bug until a real tag — the publish step now asserts it has exactly 3 archives so a partial set fails loudly instead of publishing quietly.

## Conventions

- **TDD.** Write the failing test, then implement. One commit per meaningful step, short imperative messages.
- Tests live both inline (`#[cfg(test)] mod tests`) and as integration tests under each crate's `tests/`. `ctk-sitter` uses `insta` snapshots.
