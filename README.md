# cubtoken

[![CI](https://github.com/thepragmaticbear/cubtoken/actions/workflows/ci.yml/badge.svg)](https://github.com/thepragmaticbear/cubtoken/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A single Rust binary (`ctk`) that compresses Claude Code's **native tool outputs** — `Read`, `Grep`, `Glob`, and optionally `Bash` — before they enter the model's context window. Large file reads become tree-sitter signature skeletons with exact line ranges; noisy grep results fold per file; huge glob listings become directory trees. Zero workflow change.

Host: **Claude Code** (all four tools). See [Other hosts](#other-hosts) for why nothing else is supported.

**Where the evidence actually is:** large `Read` compression is the proven case — 76–88% estimated savings on the reads that trigger it. `Grep`, `Glob` and `Bash` folding are implemented and tested but fired rarely in dogfooding, so treat their value as unproven rather than typical. `ctk stats` reports what your own sessions did.

Stream compressors like [rtk](https://github.com/rtk-ai/rtk) only see commands run through the `Bash` tool — by rtk's own docs, Claude Code's built-in `Read`/`Grep`/`Glob` bypass it entirely, and those native reads are usually the biggest token sink in a session. cubtoken covers exactly that gap and coexists with rtk (Bash handling is off by default and defers to rtk when detected).

## How it works

cubtoken installs as a **post-tool hook**. The tool runs normally (a local file read costs nothing); the hook then replaces the result *before it reaches the model* — which is where tokens are actually spent. That replacement is Claude Code's `PostToolUse` + `updatedToolOutput`. Verified live: a session reading a 26KB source file received a ~70%-smaller skeleton view.

```
   42  pub struct Registry {
        … [L43-L46]
   48  impl Registry {
   49      /// Create an empty registry.
   50      pub fn new() -> Self {
        … [L51-L56]
```

## Getting started

0. **Requirements.** Claude Code with exec-form hooks (`command` + `args`) and `PostToolUse.updatedToolOutput`. Verified against Claude Code **2.1.258**; if `ctk doctor` passes but nothing ever compresses, update Claude Code first.

1. **Get the binary.** Prebuilt archives are on the [Releases](https://github.com/thepragmaticbear/cubtoken/releases) page for three targets:

   | Platform | Archive |
   |---|---|
   | Linux x86_64 | `ctk-x86_64-unknown-linux-gnu.tar.gz` |
   | macOS Apple Silicon | `ctk-aarch64-apple-darwin.tar.gz` |
   | Windows x86_64 | `ctk-x86_64-pc-windows-msvc.zip` |

   Anything else — Intel Macs and Linux ARM included — builds from source. Every release also ships `SHA256SUMS` and a build attestation:

   ```sh
   sha256sum -c SHA256SUMS --ignore-missing
   gh attestation verify ctk-*.tar.gz --repo thepragmaticbear/cubtoken
   ```

   To build from source instead (stable Rust, no minimum version pinned):

   ```sh
   git clone https://github.com/thepragmaticbear/cubtoken.git
   cd cubtoken
   cargo install --locked --path crates/ctk-cli   # puts `ctk` on your PATH
   ```

   Install it somewhere permanent. `init` embeds the absolute path of whichever binary you ran it with, so installing the hook straight out of `./target/release` breaks the moment you `cargo clean` — `init` warns when it sees a `target/` path.

2. **Install the hook.** Run inside the project you want to compress:

   ```sh
   ctk init            # writes .claude/settings.local.json + a starter .cubtoken.toml
   ```

   The hook matches `Read|Grep|Glob|Bash|Edit|Write|NotebookEdit`. Use `ctk init --global` to install once for every project (`~/.claude/settings.json`, or `$CLAUDE_CONFIG_DIR/settings.json` when set); the global install does not write a `.cubtoken.toml`. `init` is idempotent — re-running it just refreshes the hook entry, and an existing `.cubtoken.toml` is never overwritten.

   **Or install the plugin instead.** `plugins/cubtoken/` ships the same hook as a Claude Code plugin, which resolves `ctk` at call time rather than baking in an absolute path — so `cargo clean` or moving the binary can't silently break it:

   ```sh
   /plugin marketplace add thepragmaticbear/cubtoken
   /plugin install cubtoken@cubtoken
   ```

   Both commands run inside Claude Code, not in a shell. The plugin doesn't write a starter `.cubtoken.toml`; defaults apply until you add one. If the install summary says to run `/reload-plugins`, run it.

3. **Restart Claude Code.** Hooks are snapshotted at session start, so the hook only takes effect in a session opened *after* `init` (plugin installs may instead prompt for `/reload-plugins`).

4. **Verify the install:**

   ```sh
   ctk doctor
   ```

   `doctor` prints where it found the hook and checks it can actually run, for either install route:

   - **`ctk init` installs** — searches `.claude/settings.json`, `.claude/settings.local.json`, and the active global settings location, then checks the binary path in that entry still exists. `init` embeds an absolute path, so a `cargo clean` or a moved binary otherwise breaks every hook invocation silently.
   - **Plugin installs** — reads Claude Code's plugin index, since a plugin registers its hook in the plugin's own `hooks.json` and no settings file mentions it. It then checks the wrapper is executable and that `ctk` is resolvable, because the wrapper resolves `ctk` at call time and *fails open* when it can't — meaning a missing `ctk` shows up as silence, not an error.

   The `INFO  rtk …` line reports whether rtk is on your PATH — if it is, leave `bash.enabled = false` and let rtk handle Bash. The exit code is non-zero if any check fails.

5. **Work normally.** Nothing changes in how you use Claude Code. When the model runs `Read`, `Grep`, or `Glob` and the output is large, the hook swaps in the compressed view before it reaches the context window. Targeted `Read(offset, limit)` calls and files you have edited this session are left untouched.

6. **Watch the savings:**

   ```sh
   ctk stats
   ```

   Prints a per-tool table (`tokens in / out / saved / saved%`) plus a lifetime `TOTAL`, aggregated across every session ledger in `.cubtoken/` (which self-ignores via its own `.gitignore`, so it never shows up in `git status`). `no savings recorded yet` means no compressible tool calls have run in a post-`init` session — re-check step 3.

   Counts are estimates (~3.5 chars/token), not tokenizer output. The cost side is reported too: **follow-up `Read(offset/limit)` calls into compressed files** — their count, their estimated tokens, and their tool time — followed by an **estimated net saved** line (gross savings minus refetched tokens). That net number is the one to trust; if it stalls, raise `read.threshold_tokens` so fewer files get skeletonized.

7. **Tune (optional).** Edit `.cubtoken.toml` to compress more or less — raise `read.threshold_tokens`, add globs to `read.never_compress`, or set `bash.enabled = true` if you do not run rtk. See [Configuration](#configuration-cubtokentoml-overlaid-on-configcubtokenconfigtoml) below. Config is re-read on every tool call, so edits apply to the next one — no restart needed (only installing the hook with `ctk init` requires a restart).

## Updating

New versions are published to [Releases](https://github.com/thepragmaticbear/cubtoken/releases). Update the same way you installed:

| Installed with | Update |
|---|---|
| Prebuilt archive | Download the new archive and replace the `ctk` binary **at the path it already lives at** |
| From source | `git pull && cargo install --locked --path crates/ctk-cli` |
| Plugin | Update it through Claude Code's `/plugin` management |

**Replacing the binary in place needs no restart.** `ctk init` stores an absolute path in your settings, and Claude Code executes that path fresh on every tool call — so a new binary at the same path takes effect on the next call. What gets snapshotted at session start is the hook *entry*, not the binary.

You only need to re-run `ctk init` (and then restart) if the binary lands somewhere new — for example moving from a downloaded archive to `cargo install`, which puts `ctk` in `~/.cargo/bin`. The old entry would still point at the previous path.

Either way, confirm afterwards:

```sh
ctk --version
ctk doctor       # checks the path in your settings still resolves
```

`doctor` is the one that catches a stale install: it re-checks that the binary named in your hook entry still exists and is executable, which is exactly what breaks when a version update moves it.

## Other hosts

cubtoken is Claude Code only. An OpenCode adapter shipped once and was removed to keep a single host to support.

Codex CLI and Antigravity can't host cubtoken today: neither one's post-tool hook can replace a tool result (Codex offers `systemMessage`/`continue`/`stopReason` only; Antigravity's `PostToolUse` isn't even told which tool ran).

## Design invariants

1. **Fail open** — any internal error means the original output passes through untouched. The hook never breaks a session.
2. **Escape hatch** — every compressed view names the exact tool call (`Read(offset, limit)`, `Grep(path=…)`, `Glob(pattern=…)`) that retrieves the elided content, and *every* elided line falls inside an advertised `[La-Lb]` range. Nothing is dropped silently — attributes, decorators and closing braces included.
3. **Verbatim lines** — every source line shown in a skeleton is the exact file text at the stated line number, so quoted edits stay valid. Targeted `Read(offset/limit)` calls are never compressed, and a file the model has edited this session is never compressed again (Edit-protection ledger). Claude Code renders its own sequential numbering around the substituted block, so the banner tells the model to read the *inner* gutter for real line numbers.
4. **Deterministic** — same input, same output. No LLM calls, no network, fully local. Glob keeps Claude Code's newest-first path ordering rather than sorting.
5. **Never pay to compress** — Read, Grep, Glob and Bash each pass through unless the compressed form is at least 30% smaller. A wide, flat directory tree folds to roughly itself, so it is left alone.

## Configuration (`.cubtoken.toml`, overlaid on `~/.config/cubtoken/config.toml`)

| Key | Default | Meaning |
|---|---|---|
| `read.enabled` | `true` | Compress large full-file Reads into signature skeletons |
| `read.threshold_tokens` | `2000` | Reads estimated under this size pass through |
| `read.never_compress` | `["**/*.md", "**/.env*"]` | Glob patterns never compressed |
| `grep.enabled` | `true` | Fold content-mode Grep results |
| `grep.max_matches_per_file` | `5` | Per-file match cap before folding |
| `grep.max_total_matches` | `100` | Global match cap before a per-file summary |
| `glob.enabled` | `true` | Fold long Glob listings into a tree |
| `glob.max_paths` | `50` | Listings at or under this pass through |
| `bash.enabled` | `false` | Minimal ANSI/progress strip; leave off if you use rtk |
| `stats.ledger` | `true` | Record savings to `.cubtoken/` for `ctk stats` |

Config is layered: built-in defaults, then the global `~/.config/cubtoken/config.toml`, then the project `.cubtoken.toml` in the directory the agent is running in (project wins on conflicts). The hook reads these per tool call against the session's working directory, so a single global install still honors each project's own `.cubtoken.toml` — drop one in any repo to tune it there.

Languages with skeleton support: Rust, TypeScript/TSX/JS, Python, Go (tree-sitter). Skeletons keep the context attached to a signature — Rust attributes (`#[derive]`, `#[cfg]`), Python decorators, doc comments — alongside the declaration itself. Other files fall back to head+tail elision with line numbers, which is truncation rather than summary; consider adding those extensions to `read.never_compress` if the head/tail view is not useful for them.

## Security and privacy

cubtoken is local and deterministic: it makes no network requests and never executes text from tool output. The `.cubtoken/` ledger contains file paths and usage totals, not file contents. `ctk record` is different: it deliberately saves raw hook payloads for debugging, which can contain source code, command output, paths, and secrets. Do not commit recordings.

Tool output and repository content remain untrusted input; compression is not a security filter. See [SECURITY.md](SECURITY.md) for the threat model and private vulnerability reporting.

## Uninstall

1. Remove the hook — the inverse of `init`, run from the same place:

   ```sh
   ctk uninstall            # or: ctk uninstall --global
   ```

   It strips only cubtoken's `PostToolUse` entry, leaves every other hook and setting untouched, and cleans up the empty `hooks` scaffolding it created. If you installed globally but run a bare `ctk uninstall`, it tells you where the hook actually lives instead of reporting nothing found. Restart Claude Code afterwards — hooks are snapshotted at session start.

   `uninstall` only touches settings files. If you installed the **plugin** instead of running `ctk init`, remove it through Claude Code's own `/plugin` management rather than here — the plugin's hook lives in the plugin, not in your settings.

2. Delete `.cubtoken.toml` and `.cubtoken/` if you do not want to keep configuration or statistics. `uninstall` deliberately leaves both — they are your config and recorded savings, not install state.
3. Run `cargo uninstall ctk-cli` if you installed from source, or delete the downloaded `ctk` binary.

## Development

TDD throughout; fixtures in `tests/fixtures/` are real recorded hook payloads (see `ctk record`). Gate: `cargo fmt --check && cargo clippy --locked --workspace --all-targets --all-features -- -D warnings && cargo test --locked --workspace --all-features && cargo audit --deny warnings`.

Layout beyond the Rust workspace:

| Path | What |
|---|---|
| `crates/` | `ctk-cli` → `ctk-hook` → `ctk-compress` → `ctk-sitter`, strictly layered |
| `plugins/cubtoken/` | The Claude Code plugin (manifest, `hooks/hooks.json`, `bin/ctk-hook` wrapper) |
| `.claude-plugin/marketplace.json` | Makes this repo installable as a plugin marketplace |

Contributions are welcome — the gate above is what CI enforces, so run it before opening a PR.

## License

MIT — see [LICENSE](LICENSE).
