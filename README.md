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

## Install

```sh
cargo install --path crates/ctk-cli   # or: cargo build --release
ctk init          # project install (.claude/settings.json) + starter .cubtoken.toml
ctk init --global # or per-user (~/.claude/settings.json)
ctk doctor        # verify
# restart your Claude Code session (hooks snapshot at startup)
ctk stats         # watch the savings accumulate
```

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
