# cubtoken

A single Rust binary (`ctk`) that compresses Claude Code's **native tool outputs** — `Read`, `Grep`, `Glob`, and optionally `Bash` — before they enter the model's context window. Large file reads become tree-sitter signature skeletons with exact line ranges; noisy grep results fold per file; huge glob listings become directory trees. Typical savings: 50–90% on large outputs, with zero workflow change.

Stream compressors like [rtk](https://github.com/rtk-ai/rtk) only see commands run through the `Bash` tool — by rtk's own docs, Claude Code's built-in `Read`/`Grep`/`Glob` bypass it entirely, and those native reads are usually the biggest token sink in a session. cubtoken covers exactly that gap and coexists with rtk (Bash handling is off by default and defers to rtk when detected).

## How it works

cubtoken installs as a **PostToolUse hook**. The tool runs normally (a local file read costs nothing); the hook then replaces the result via `updatedToolOutput` *before it reaches the model* — which is where tokens are actually spent. Verified live: a session reading a 26KB source file received a ~70%-smaller skeleton view.

```
   42  pub struct Registry {
        … [L43-L46]
   48  impl Registry {
   49      /// Create an empty registry.
   50      pub fn new() -> Self {
        … [L51-L56]
```

## Getting started

1. **Build the binary.** From the repo root:

   ```sh
   cargo install --path crates/ctk-cli   # puts `ctk` on your PATH
   # or, without installing: cargo build --release  →  ./target/release/ctk
   ```

2. **Install the hook.** Run inside the project you want to compress:

   ```sh
   ctk init            # writes .claude/settings.json + a starter .cubtoken.toml
   ```

   The hook matches `Read|Grep|Glob|Bash|Edit|Write|NotebookEdit`. Use `ctk init --global` to install once for every project (`~/.claude/settings.json`); the global install does not write a `.cubtoken.toml`. `init` is idempotent — re-running it just refreshes the hook entry, and an existing `.cubtoken.toml` is never overwritten.

3. **Restart Claude Code.** Hooks are snapshotted at session start, so the hook only takes effect in a session opened *after* `init`.

4. **Verify the install:**

   ```sh
   ctk doctor
   ```

   Expect `PASS  PostToolUse hook installed` and `PASS  .cubtoken/ writable`. The `INFO  rtk …` line reports whether rtk is on your PATH — if it is, leave `bash.enabled = false` and let rtk handle Bash. The exit code is non-zero if any check fails.

5. **Work normally.** Nothing changes in how you use Claude Code. When the model runs `Read`, `Grep`, or `Glob` and the output is large, the hook swaps in the compressed view before it reaches the context window. Targeted `Read(offset, limit)` calls and files you have edited this session are left untouched.

6. **Watch the savings:**

   ```sh
   ctk stats
   ```

   Prints a per-tool table (`tokens in / out / saved / saved%`) plus a lifetime `TOTAL`, aggregated across every session ledger in `.cubtoken/`. `no savings recorded yet` means no compressible tool calls have run in a post-`init` session — re-check step 3.

7. **Tune (optional).** Edit `.cubtoken.toml` to compress more or less — raise `read.threshold_tokens`, add globs to `read.never_compress`, or set `bash.enabled = true` if you do not run rtk. See [Configuration](#configuration-cubtokentoml-overlaid-on-configcubtokenconfigtoml) below. Restart the session for changes to take effect.

## Design invariants

1. **Fail open** — any internal error means the original output passes through untouched. The hook never breaks a session.
2. **Escape hatch** — every compressed view names the exact tool call (`Read(offset, limit)`, `Grep(path=…)`, `Glob(pattern=…)`) that retrieves the elided content.
3. **Verbatim lines** — every source line shown in a skeleton is the exact file text at the stated line number, so quoted edits stay valid. Targeted `Read(offset/limit)` calls are never compressed, and a file the model has edited this session is never compressed again (Edit-protection ledger).
4. **Deterministic** — same input, same output. No LLM calls, no network, fully local.

## Configuration (`.cubtoken.toml`, overlaid on `~/.config/cubtoken/config.toml`)

| Key | Default | Meaning |
|---|---|---|
| `read.enabled` | `true` | Compress large full-file Reads into signature skeletons |
| `read.threshold_tokens` | `2000` | Reads estimated under this size pass through |
| `read.never_compress` | `["**/*.md", "**/.env*"]` | Glob patterns never compressed |
| `grep.enabled` | `true` | Fold content-mode Grep results |
| `grep.max_matches_per_file` | `5` | Per-file match cap before folding |
| `glob.enabled` | `true` | Fold long Glob listings into a tree |
| `glob.max_paths` | `50` | Listings at or under this pass through |
| `bash.enabled` | `false` | Minimal ANSI/progress strip; leave off if you use rtk |
| `stats.ledger` | `true` | Record savings to `.cubtoken/` for `ctk stats` |

Languages with skeleton support: Rust, TypeScript/TSX/JS, Python, Go (tree-sitter). Other files fall back to head+tail elision with line numbers.

## Development

TDD throughout; fixtures in `tests/fixtures/` are real recorded hook payloads (see `ctk record`). Gate: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`. Design docs live in `docs/bearpaws/plans/`.
