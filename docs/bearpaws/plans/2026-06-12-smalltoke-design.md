# smalltoke — Design Spec

**Date:** 2026-06-12
**Status:** Draft — pending Brandon's review
**Repo:** `~/repos/smalltoke` (greenfield)

---

## 1. Background: what the vetting found

The original market analysis was directionally right but wrong on the load-bearing technical detail. Summary of verified facts:

| Claim | Verdict | Reality |
|---|---|---|
| rtk exists, Rust, compresses Bash output 60–90% | ✅ True | [rtk-ai/rtk](https://github.com/rtk-ai/rtk), ~62k stars, very active (v0.42.4, June 2026) |
| rtk only covers the `Bash` tool; `Read`/`Grep`/`Glob` bypass it | ✅ True — **rtk's own README admits this** | "Claude Code built-in tools like Read, Grep, and Glob do not pass through the Bash hook, so they are not auto-rewritten" |
| rtk auto-injects `permissionDecision: allow` as a security hole | ⚠️ Overstated | Hook `allow` skips the prompt but explicit deny/ask permission rules are **still evaluated**. A real design concern, not a gaping vulnerability |
| repowise: 5 layers, 25 biomarkers, slow LLM-driven indexing | ✅ Mostly true | [repowise-dev/repowise](https://github.com/repowise-dev/repowise). ROC AUC is "up to 0.90", not 0.74. Exposes **9** MCP tools, not dozens |
| graphify: Leiden clustering, `graphify-out/`, freshness problem | ✅ True | [graphify.net](https://graphify.net/), tree-sitter + NetworkX + Leiden |
| "45 MCP tools" conversational overhead (codegraph) | ⚠️ Unverified number, but the *category problem* is real | Serena/codegraph-class servers do expose large tool surfaces; tool-choice overhead is a known cost |
| **A PreToolUse hook can substitute compressed output for Read/Grep** | ❌ **False — kills the proposal as written** | PreToolUse can only allow/deny/ask and rewrite tool *input* (`updatedInput`). It cannot fabricate output |
| — | ✅ **Corrected mechanism exists** | **PostToolUse `updatedToolOutput` replaces the tool result before the model sees it.** Tokens are only spent when content enters model context; the tool ran locally (free), so rewriting after execution captures the full savings |

**Landscape verdict:** the space is crowded on two flanks — stream compression (rtk, dominant) and deep repo intelligence (repowise, Serena, graphify, codegraph). The genuinely open gap is narrow but real: **nobody compresses the native agent tools (`Read`, `Grep`, `Glob`), and rtk has publicly documented that it doesn't.** Native-tool reads are typically the largest single token sink in a Claude Code session.

**Strategic consequence:** do **not** rebuild rtk's 100+ command filters or repowise's biomarkers. Build the thing nobody has: a PostToolUse output-rewriting engine for native tools, in one Rust binary, that coexists with rtk.

---

## 2. Product definition

**smalltoke** (binary: `stk`) — a single Rust binary that installs as a Claude Code `PostToolUse` hook and losslessly-escapable compresses native tool outputs (`Read`, `Grep`, `Glob`, optionally `Bash`) before they enter model context.

### Goals
1. Cut tokens from `Read`/`Grep`/`Glob` results 50–90% on large outputs with zero workflow change.
2. Sub-10ms hook overhead (parse stdin JSON → rewrite → stdout JSON; no network, no daemon required for v1).
3. **Never break the agent.** Every compressed output embeds an explicit escape hatch telling the model exactly how to retrieve the uncompressed detail it needs.
4. Deterministic, LLM-free, fully local.

### Non-goals (YAGNI — explicitly cut from the original proposal)
- Re-implementing rtk's Bash command filters (rtk has 62k stars; coexist, don't compete). A minimal Bash fallback ships only as an opt-in for users without rtk.
- LLM-generated docs, ONNX embeddings, defect prediction, biomarkers (repowise's turf).
- Graph visualization frontends.
- The "Rule-Based Sandbox Gate" auto-approval layer — deferred. It's a separate product with separate risk; v1 never touches `permissionDecision`.
- Cursor/Windsurf adapters — Claude Code first; the hook core is host-agnostic by design so adapters can come later.

---

## 3. Architecture

```
┌─ Claude Code ────────────────────────────────────────────┐
│  Read/Grep/Glob/Bash executes normally (local, free)     │
│        │ PostToolUse fires with tool_response            │
│        ▼                                                 │
│  stk hook  ── stdin: hook JSON ──┐                       │
└──────────────────────────────────┼───────────────────────┘
                                   ▼
                    ┌─ compressor dispatch ─┐
                    │ Read  → signature view│  tree-sitter, on-demand parse
                    │ Grep  → match folding │  pure text algorithms
                    │ Glob  → tree folding  │
                    │ Bash  → noise strip   │  opt-in only
                    └───────────┬───────────┘
                                ▼
            stdout: { hookSpecificOutput: { updatedToolOutput } }
            (or empty output = pass through untouched)
```

### Crate layout (cargo workspace)

```
smalltoke/
├── Cargo.toml              # workspace
├── crates/
│   ├── stk-cli/            # bin: clap entrypoint (hook, init, stats, doctor)
│   ├── stk-hook/           # hook protocol types + dispatch; host-agnostic core
│   ├── stk-compress/       # the compressors (read/grep/glob/bash) — pure functions
│   └── stk-sitter/         # tree-sitter wrapper: file → signature skeleton
└── docs/
```

`stk-compress` functions are `fn(&Input) -> Option<Compressed>` — pure, no I/O — so the entire value of the product is golden-file testable.

### Subcommands

| Command | Purpose |
|---|---|
| `stk hook` | The PostToolUse handler. Reads hook JSON on stdin, writes decision JSON on stdout. Exit fast, fail open (any internal error → emit nothing → original output passes through). |
| `stk init [--global]` | Installs the PostToolUse hook into `.claude/settings.json` (or `~/.claude/settings.json`). Detects rtk and disables `stk`'s Bash handling if present. |
| `stk stats` | Reads the local savings ledger (JSONL in `.smalltoke/`): tokens in vs out per tool, session and lifetime totals. |
| `stk doctor` | Verifies hook installation, tree-sitter grammars, permission-rule conflicts. |

---

## 4. Compressor designs

### 4.1 Read → signature view (the flagship)

Trigger: `tool_response` content estimated > `read_threshold` tokens (default ~2,000; chars/3.5 heuristic).

Output substituted via `updatedToolOutput`:

```
[smalltoke: compressed view of src/parser.rs — 48,210 chars → skeleton.
 Full text of any region: Read with offset/limit as shown.]

  1  use std::collections::HashMap;            ← imports kept verbatim
  …
 40  pub struct Parser { … 12 fields … }       ← types: names + field count
 88  pub fn parse(&mut self, src: &str) -> Ast  { … }   [L88-214]
215  fn advance(&mut self) -> Token             { … }   [L215-260]
     // 14 more private fns: peek L261, expect L270, …
```

Rules:
- tree-sitter parse of the raw text (grammars compiled in: Rust, TS/JS/TSX, Python, Go, Java, C/C++, Ruby, JSON, YAML, TOML, Markdown to start). Unparseable/unknown language → fall back to head+tail elision with line numbers, or pass through if small.
- Doc comments on public items kept; bodies elided with **exact line ranges** so the model can `Read(offset, limit)` precisely.
- **Targeted reads are sacred:** if the model passed an explicit `offset`/`limit`, never compress — it's drilling into detail, likely to quote text for `Edit`.

#### ⚠️ The Edit-correctness hazard (biggest design risk)

`Edit` requires `old_string` to match file text **exactly**. If the model edits based on a compressed Read, the edit fails (harness rejects a non-matching `old_string` — safe failure, but burns a turn) or worse, the model guesses.

Mitigations (all three):
1. The substituted output's header explicitly instructs: *"Before editing this file, Read the exact target region with offset/limit."*
2. Skeleton lines that **are** shown are verbatim file text with real line numbers — never paraphrased.
3. Session-scoped recency ledger (`.smalltoke/session-<id>.jsonl`, also used for stats): a file the model has Edited or Written this session is **never compressed again** for the rest of the session.

### 4.2 Grep → match folding

- Collapse >N matches in one file to first/last + `(+k more in this file)`.
- Deduplicate identical match lines across files (lockfiles, generated code) to one line + count + file list.
- Cap total output; remainder summarized as `path: count` table with the exact `Grep` invocation (path filter) to expand.

### 4.3 Glob → tree folding

- >N paths: render as directory tree with counts (`src/components/ (212 .tsx files)`), expanding only directories with few entries. Include the narrower glob pattern to expand any folded node.

### 4.4 Bash → minimal noise strip (opt-in, off when rtk detected)

- ANSI strip, progress-bar line dedup, blank-run collapse. Nothing command-specific — that's rtk's job. Never touch stderr content of failing commands (exit code ≠ 0 → pass through verbatim; test failures and stack traces are sacred per the brevity-of-noise-not-signal principle).

---

## 5. Configuration

`.smalltoke.toml` (project) merged over `~/.config/smalltoke/config.toml` (global):

```toml
[read]
enabled = true
threshold_tokens = 2000
never_compress = ["**/*.md", "**/.env*"]   # globs; docs read for prose, secrets policed by user

[grep]
enabled = true
max_matches_per_file = 5

[glob]
enabled = true
max_paths = 50

[bash]
enabled = false          # auto-false when rtk present

[stats]
ledger = true
```

---

## 6. Error handling & safety principles

1. **Fail open.** Any panic, parse error, malformed JSON, or timeout inside `stk hook` → emit nothing → Claude Code uses the original output. A compression tool must never be the reason a session breaks. Panics are caught (`catch_unwind`) and logged to `.smalltoke/errors.log`, never to stdout.
2. **Deterministic.** Same input → same output. No clocks, no randomness in compressors.
3. **No permission meddling.** v1 never emits `permissionDecision`. PostToolUse only.
4. **Escape hatch invariant** (testable): every compressed output must contain at least one concrete instruction (exact tool + args) for retrieving elided content.

---

## 7. Testing strategy

- **Golden-file tests** for every compressor: `tests/fixtures/<tool>/<case>/{input.json, expected.txt}`. The fixtures are recorded real hook payloads.
- **Invariant tests:** escape-hatch presence; shown-lines-are-verbatim (skeleton lines must be exact substrings of source at the stated line numbers); output-smaller-than-input or pass-through.
- **Integration test:** drive `stk hook` as a subprocess with recorded stdin payloads, assert stdout JSON shape matches Claude Code's PostToolUse schema.
- **Dogfood metric:** develop smalltoke with smalltoke installed; `stk stats` is both a feature and the benchmark.

---

## 8. Phased roadmap

- **Phase 1 (MVP):** workspace scaffold → hook protocol → Read signature view → `init`/`doctor`/`stats`. *Ships alone; this is the entire differentiator.*
- **Phase 2:** Grep + Glob folding.
- **Phase 3:** Bash minimal strip + rtk detection.
- **Phase 4 (re-evaluate after dogfooding):** SQLite AST index + single `get_context` MCP tool (blast radius, symbol lookup). Only if Phase 1–3 data shows the model re-reading the same structures repeatedly.
- **Deferred indefinitely:** sandbox permission gate, git analytics, embeddings.

---

## 9. Open questions for Brandon

1. Name/binary OK? (`smalltoke` / `stk`)
2. Default `read.threshold_tokens` = 2000 — too aggressive? (Claude Code itself pages reads at its own internal limit; we compress below that.)
3. Phase 4 (MCP/index) is deliberately deferred — agree, or is the index the part you care most about?
4. License/publishing intent (crates.io? GitHub public?) — affects CI scaffolding in Phase 1.
