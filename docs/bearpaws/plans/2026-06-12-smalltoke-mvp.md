# smalltoke MVP Implementation Plan (Phases 1–3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use bp:subagent-driven-development (recommended) or bp:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Ship `stk`, a single Rust binary installed as a Claude Code PostToolUse hook that compresses `Read`/`Grep`/`Glob` (and optionally `Bash`) tool outputs 50–90% before they enter model context, failing open on any error.

**Architecture:** Cargo workspace with four crates: `stk-cli` (clap entrypoint), `stk-hook` (hook protocol + dispatch, host-agnostic), `stk-compress` (pure compressor functions), `stk-sitter` (tree-sitter → signature skeleton). The hook reads PostToolUse JSON on stdin and either emits `{hookSpecificOutput: {updatedToolOutput: …}}` on stdout or emits nothing (pass-through). See companion spec: `2026-06-12-smalltoke-design.md`.

**Tech Stack:** Rust (stable), clap, serde/serde_json, tree-sitter (+ rust/typescript/python/go grammars to start), globset, insta (snapshot tests), assert_cmd (CLI integration tests).

**Design invariants every task must respect:**
1. **Fail open** — any error in `stk hook` → print nothing, exit 0.
2. **Escape hatch** — every compressed output contains exact tool+args to retrieve elided content.
3. **Verbatim lines** — any source line shown in a skeleton is an exact substring of the original at the stated line number.
4. **Deterministic** — no clocks/randomness in compressors.

---

### Task 1: Workspace scaffold

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `rust-toolchain.toml`
- Create: `crates/stk-cli/Cargo.toml`, `crates/stk-cli/src/main.rs`
- Create: `crates/stk-hook/Cargo.toml`, `crates/stk-hook/src/lib.rs`
- Create: `crates/stk-compress/Cargo.toml`, `crates/stk-compress/src/lib.rs`
- Create: `crates/stk-sitter/Cargo.toml`, `crates/stk-sitter/src/lib.rs`

- [x] **Step 1: Root workspace manifest**

```toml
# Cargo.toml
[workspace]
resolver = "2"
members = ["crates/stk-cli", "crates/stk-hook", "crates/stk-compress", "crates/stk-sitter"]

[workspace.package]
edition = "2021"
license = "MIT"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
```

```toml
# rust-toolchain.toml
[toolchain]
channel = "stable"
```

```
# .gitignore
target/
.smalltoke/
```

- [x] **Step 2: Crate manifests + empty lib/main** *(deviation: tree-sitter deps deferred to Task 4 — see HANDOFF decision log)*

Each `crates/*/Cargo.toml` declares `name`, inherits `edition.workspace = true`, `license.workspace = true`. `stk-cli` is `[[bin]] name = "stk"` depending on `clap = { version = "4", features = ["derive"] }`, `stk-hook` (path dep). `stk-hook` depends on `stk-compress` (path), `serde`, `serde_json`. `stk-compress` depends on `stk-sitter` (path), `globset = "0.4"`. `stk-sitter` depends on `tree-sitter = "0.25"` plus grammar crates (`tree-sitter-rust`, `tree-sitter-typescript`, `tree-sitter-python`, `tree-sitter-go` — pin to versions compatible with the tree-sitter core version; check crates.io at implementation time and record the pins in this file's commit).

`crates/stk-cli/src/main.rs`:

```rust
fn main() {
    println!("stk");
}
```

Lib crates: empty `lib.rs` files.

- [x] **Step 3: Verify build — `cargo build && cargo test`**

Expected: builds clean, zero tests pass (no tests yet). ✅ rustc 1.96.0, clean build.

- [x] **Step 4: Commit** — `git add -A && git commit -m "Scaffold cargo workspace"`

---

### Task 2: Payload recorder + real fixtures

The exact `tool_response` JSON shapes for Read/Grep/Glob are not fully documented; we record real ones rather than guessing. This recorder is also the permanent debug facility.

**Files:**
- Modify: `crates/stk-cli/src/main.rs`
- Create: `tests/fixtures/` (recorded payloads, sanitized)

- [x] **Step 1: Write failing CLI test** in `crates/stk-cli/tests/record.rs` (add `assert_cmd = "2"`, `tempfile = "3"` as dev-deps):

```rust
use assert_cmd::Command;

#[test]
fn record_appends_stdin_json_to_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("payloads.jsonl");
    Command::cargo_bin("stk").unwrap()
        .args(["record", out.to_str().unwrap()])
        .write_stdin(r#"{"tool_name":"Read"}"#)
        .assert().success().stdout("");
    let content = std::fs::read_to_string(&out).unwrap();
    assert_eq!(content.trim(), r#"{"tool_name":"Read"}"#);
}
```

- [x] **Step 2: Run — verify FAIL** (`cargo test -p stk-cli`): no `record` subcommand. ✅ failed as expected.

- [x] **Step 3: Implement** clap derive in `main.rs`: `enum Cmd { Record { path: PathBuf }, Hook, Init { #[arg(long)] global: bool }, Stats, Doctor }`. `Record` reads all of stdin, appends one line to `path` (create if missing), prints nothing, always exits 0. `Hook`/`Init`/`Stats`/`Doctor` are stubs that exit 0 silently for now.

- [x] **Step 4: Run — verify PASS.** ✅ 2 passed.

- [x] **Step 5: Capture real payloads.** *(Done autonomously: headless `claude -p` haiku run in /tmp/stk-capture with the recorder hook pre-installed — no session restart needed. Real `tool_response` schemas now in `tests/fixtures/`. Finding: Bash response has NO exit-code field — only stdout/stderr/interrupted/isImage/noOutputExpected.)* Manually (this step is for the human/agent driving a live Claude Code session): add to this repo's `.claude/settings.json` a PostToolUse hook `{"matcher": "Read|Grep|Glob|Bash", "hooks": [{"type": "command", "command": "cargo run -q -p stk-cli -- record tests/fixtures/raw.jsonl"}]}`; in a Claude Code session run one Read of a large file, one offset/limit Read, one Grep with many matches, one Glob with many results, one Bash command. Split the resulting lines into `tests/fixtures/read_large.json`, `read_offset.json`, `grep_many.json`, `glob_many.json`, `bash_simple.json`. Strip any private absolute-path or content data; keep structure. Remove the temporary hook.

- [x] **Step 6: Commit** — `git commit -m "Add record subcommand and real hook fixtures"`

---

### Task 3: Hook protocol types and fail-open runner

**Files:**
- Create: `crates/stk-hook/src/protocol.rs`
- Modify: `crates/stk-hook/src/lib.rs`, `crates/stk-cli/src/main.rs`

- [x] **Step 1: Write failing tests** in `crates/stk-hook/src/protocol.rs` (unit tests parse every fixture):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_all_recorded_fixtures() {
        for f in ["read_large", "read_offset", "grep_many", "glob_many", "bash_simple"] {
            let raw = std::fs::read_to_string(
                format!("{}/../../tests/fixtures/{f}.json", env!("CARGO_MANIFEST_DIR"))).unwrap();
            let p: HookPayload = serde_json::from_str(&raw).expect(f);
            assert!(!p.tool_name.is_empty());
        }
    }
    #[test]
    fn malformed_input_yields_none_decision() {
        assert!(run_hook("not json", &Config::default()).is_none());
    }
}
```

- [x] **Step 2: Run — verify FAIL** (types don't exist).

- [x] **Step 3: Implement.** Tolerant types — only pin down what we use, keep the rest as `Value`:

```rust
#[derive(serde::Deserialize)]
pub struct HookPayload {
    pub tool_name: String,
    #[serde(default)]
    pub tool_input: serde_json::Value,
    #[serde(default)]
    pub tool_response: serde_json::Value,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub cwd: String,
}

#[derive(serde::Serialize)]
pub struct HookOutput {
    #[serde(rename = "hookSpecificOutput")]
    pub hook_specific_output: HookSpecificOutput,
}

#[derive(serde::Serialize)]
pub struct HookSpecificOutput {
    #[serde(rename = "hookEventName")]
    pub hook_event_name: &'static str, // always "PostToolUse"
    #[serde(rename = "updatedToolOutput")]
    pub updated_tool_output: serde_json::Value,
}
```

`lib.rs` exposes `pub fn run_hook(stdin: &str, cfg: &Config) -> Option<String>`: parse → dispatch (returns `None` for every tool for now) → serialize. `Config::default()` for now is an empty struct (fleshed out in Task 5). In `stk-cli`, `Hook` reads stdin, wraps the call in `std::panic::catch_unwind`, prints `Some(json)` or nothing, **always exits 0**; on error, appends a line to `.smalltoke/errors.log` if writable, never to stdout/stderr.

- [x] **Step 4: Run — verify PASS** (`cargo test -p stk-hook`).

- [x] **Step 5: Commit** — `git commit -m "Hook protocol types and fail-open runner"`

---

### Task 4: tree-sitter signature skeleton (`stk-sitter`)

**Files:**
- Modify: `crates/stk-sitter/src/lib.rs`
- Create: `crates/stk-sitter/tests/skeleton.rs`, `crates/stk-sitter/tests/corpus/sample.rs` (a ~200-line realistic Rust file with structs, impls, doc comments, private fns)

- [x] **Step 1: Write failing tests** (`insta = "1"` as dev-dep):

```rust
use stk_sitter::{skeleton, Lang};

#[test]
fn rust_skeleton_snapshot() {
    let src = include_str!("corpus/sample.rs");
    let sk = skeleton(src, Lang::Rust).unwrap();
    insta::assert_snapshot!(sk.rendered);
}

#[test]
fn skeleton_lines_are_verbatim() {
    let src = include_str!("corpus/sample.rs");
    let lines: Vec<&str> = src.lines().collect();
    for shown in skeleton(src, Lang::Rust).unwrap().shown_lines {
        // shown.0 is 1-based line number, shown.1 the text we rendered
        assert_eq!(lines[shown.0 - 1], shown.1, "line {} not verbatim", shown.0);
    }
}

#[test]
fn unknown_language_returns_none() {
    assert!(stk_sitter::lang_for_path("file.xyz").is_none());
}
```

- [x] **Step 2: Run — verify FAIL.**

- [x] **Step 3: Implement.** Public API:

```rust
pub enum Lang { Rust, TypeScript, Python, Go }
pub fn lang_for_path(path: &str) -> Option<Lang>;   // by extension
pub struct Skeleton {
    pub rendered: String,                  // the full skeleton text with line-number gutter
    pub shown_lines: Vec<(usize, String)>, // (1-based line, verbatim text) for invariant testing
}
pub fn skeleton(src: &str, lang: Lang) -> Option<Skeleton>;
```

Algorithm: parse with tree-sitter; walk top-level + one nesting level of named nodes. For declaration kinds per language (Rust: `function_item`, `struct_item`, `enum_item`, `impl_item`, `trait_item`, `use_declaration`, `mod_item`; TS: `function_declaration`, `class_declaration`, `interface_declaration`, `import_statement`, method definitions; Python: `function_definition`, `class_definition`, `import.*`; Go: `function_declaration`, `method_declaration`, `type_declaration`, `import.*`): render the node's **first line verbatim** with its line number, then `{ … }   [L<start>-L<end>]` if the body spans >2 lines. Keep contiguous doc-comment lines immediately above public items. Imports rendered verbatim; runs of >10 collapsed to first 3 + `… +N more imports [L<a>-L<b>]`. Track every emitted verbatim line in `shown_lines`. Parse failure or zero declarations → return `None` (caller falls back).

- [x] **Step 4: Run — verify PASS**, review and `cargo insta accept` the snapshot, re-run green.

- [x] **Step 5: Commit** — `git commit -m "tree-sitter signature skeletons for Rust/TS/Python/Go"`

---

### Task 5: Config loading (`stk-compress`)

**Files:**
- Create: `crates/stk-compress/src/config.rs`
- Modify: `crates/stk-compress/src/lib.rs` (add `toml = "0.8"` dep)

- [x] **Step 1: Write failing unit tests** in `config.rs`:

```rust
#[test]
fn defaults_when_no_file() {
    let c = Config::load_from(None, None);
    assert_eq!(c.read.threshold_tokens, 2000);
    assert!(c.read.enabled && c.grep.enabled && c.glob.enabled);
    assert!(!c.bash.enabled);
}

#[test]
fn project_overrides_global() {
    let global = r#"[read]
threshold_tokens = 5000"#;
    let project = r#"[read]
threshold_tokens = 1000"#;
    let c = Config::load_from(Some(global), Some(project));
    assert_eq!(c.read.threshold_tokens, 1000);
}

#[test]
fn never_compress_glob_matches() {
    let c = Config::load_from(None, Some(r#"[read]
never_compress = ["**/*.md"]"#));
    assert!(c.read.is_excluded("docs/notes.md"));
    assert!(!c.read.is_excluded("src/main.rs"));
}
```

- [x] **Step 2: Run — verify FAIL.**

- [x] **Step 3: Implement** `Config` with serde-default structs exactly mirroring the spec §5 TOML (`read.{enabled, threshold_tokens, never_compress}`, `grep.{enabled, max_matches_per_file}`, `glob.{enabled, max_paths}`, `bash.enabled`, `stats.ledger`). `load_from(global: Option<&str>, project: Option<&str>)` parses each TOML and field-wise overlays project over global over defaults (each field is `Option` in the raw deserialization layer, then resolved). `load()` reads `~/.config/smalltoke/config.toml` and `<cwd>/.smalltoke.toml`. `is_excluded` builds a `globset::GlobSet` (cached with `OnceCell`). Move the placeholder `Config` from Task 3 here; `stk-hook` re-exports it.

- [x] **Step 4: Run — verify PASS.**

- [x] **Step 5: Commit** — `git commit -m "Layered TOML config"`

---

### Task 6: Read compressor end-to-end

**Files:**
- Create: `crates/stk-compress/src/read.rs`, `crates/stk-compress/src/estimate.rs`
- Modify: `crates/stk-hook/src/lib.rs` (dispatch)
- Create: `crates/stk-hook/tests/end_to_end.rs`

- [x] **Step 1: Write failing tests.** In `estimate.rs`: `est_tokens("abcd".repeat(35)) ≈ 40` (chars/3.5, ceiling). In `read.rs` unit tests + hook end-to-end test:

```rust
// crates/stk-hook/tests/end_to_end.rs
use stk_hook::{run_hook, Config};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/../../tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn large_read_is_compressed_with_escape_hatch() {
    let out = run_hook(&fixture("read_large"), &Config::default()).expect("should compress");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let text = v["hookSpecificOutput"]["updatedToolOutput"].to_string();
    assert!(text.contains("smalltoke"));            // banner present
    assert!(text.contains("offset"));               // escape hatch present
    assert!(text.len() < fixture("read_large").len() / 2);
}

#[test]
fn offset_read_passes_through() {
    assert!(run_hook(&fixture("read_offset"), &Config::default()).is_none());
}

#[test]
fn small_read_passes_through() {
    // grep fixture reused: any payload whose content is under threshold
    let mut v: serde_json::Value = serde_json::from_str(&fixture("read_large")).unwrap();
    // shrink content below threshold, keeping real structure
    truncate_content_field(&mut v, 200); // helper in this test file: walks tool_response, truncates the content string
    assert!(run_hook(&v.to_string(), &Config::default()).is_none());
}
```

- [x] **Step 2: Run — verify FAIL.**

- [x] **Step 3: Implement.**
  - `estimate.rs`: `pub fn est_tokens(s: &str) -> usize { (s.chars().count() as f32 / 3.5).ceil() as usize }`
  - `read.rs`: `pub fn compress_read(input: &Value, response: &Value, cfg: &Config) -> Option<Value>`:
    1. Extract `file_path`, `offset`, `limit` from `tool_input`; **return `None` if `offset` or `limit` present**, or path matches `never_compress`, or `!cfg.read.enabled`.
    2. Extract the content string from `tool_response` — write `fn extract_content(&Value) -> Option<(String, ContentShape)>` handling the shapes observed in the Task 2 fixtures (string form and nested-object form); `ContentShape` remembers where it came from so we can write the replacement back into an identically-shaped `Value`.
    3. `est_tokens(content) <= cfg.read.threshold_tokens` → `None`.
    4. `lang_for_path` + `skeleton()`; fallback when `None`: head 40 lines + `… [smalltoke: lines 41-N elided] …` + tail 20 lines, all with line-number gutter.
    5. Banner: `[smalltoke: compressed view of {path} — {orig_chars} chars → skeleton. This is NOT the full file. Before quoting or editing, Read the exact region: Read(file_path={path}, offset=<line>, limit=<count>). Bracketed [La-Lb] ranges mark elided bodies.]`
    6. Re-wrap rendered text into the original `ContentShape` → `Some(updated_response_value)`. If the compressed form isn't at least 30% smaller, return `None` (not worth substituting).
  - `stk-hook` dispatch: `tool_name == "Read"` → `compress_read`; wrap result in `HookOutput`.

- [x] **Step 4: Run — verify PASS** (`cargo test`).

- [x] **Step 5: Add invariant test** in `read.rs`: feed `tests/corpus/sample.rs` content through `compress_read` with a synthetic payload; assert every gutter line shown matches the source verbatim (reuse `shown_lines`), and output contains exactly one banner. Verify PASS.

- [x] **Step 6: Commit** — `git commit -m "Read signature-view compression end-to-end"`

---

### Task 7: Session ledger — edit protection and savings stats

**Files:**
- Create: `crates/stk-hook/src/ledger.rs`
- Modify: `crates/stk-hook/src/lib.rs`, `crates/stk-cli/src/main.rs` (hook matcher will include Edit/Write)

- [x] **Step 1: Write failing tests** in `ledger.rs` (tempdir-based):

```rust
#[test]
fn edited_file_is_protected_for_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = Ledger::open(dir.path(), "sess1");
    l.note_edit("/repo/src/main.rs");
    assert!(l.is_protected("/repo/src/main.rs"));
    // fresh handle, same session id — persisted
    let l2 = Ledger::open(dir.path(), "sess1");
    assert!(l2.is_protected("/repo/src/main.rs"));
    // different session — not protected
    let l3 = Ledger::open(dir.path(), "sess2");
    assert!(!l3.is_protected("/repo/src/main.rs"));
}

#[test]
fn savings_accumulate() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = Ledger::open(dir.path(), "s");
    l.note_saving("Read", 10_000, 1_500);
    l.note_saving("Read", 4_000, 900);
    let s = Ledger::open(dir.path(), "s").totals();
    assert_eq!((s.tokens_in, s.tokens_out), (14_000, 2_400));
}
```

- [x] **Step 2: Run — verify FAIL.**

- [x] **Step 3: Implement.** JSONL file `.smalltoke/session-<session_id>.jsonl`, append-only records `{"e":"edit","path":…}` / `{"e":"save","tool":…,"in":…,"out":…}`; `open` replays the file into memory. Wire into `run_hook`: payloads with `tool_name` `Edit`/`Write`/`NotebookEdit` → `note_edit(file_path)`, return `None`; `compress_read` consults `is_protected` (skip compression) and successful compressions call `note_saving` with `est_tokens` before/after. Ledger I/O errors are swallowed (fail open). Update the future `init` matcher list to `Read|Grep|Glob|Bash|Edit|Write|NotebookEdit`.

- [x] **Step 4: Run — verify PASS.**

- [x] **Step 5: Commit** — `git commit -m "Session ledger: edit protection and savings tracking"`

---

### Task 8: `stk init` and `stk doctor`

**Files:**
- Create: `crates/stk-cli/src/init.rs`, `crates/stk-cli/src/doctor.rs`
- Modify: `crates/stk-cli/src/main.rs`
- Create: `crates/stk-cli/tests/init.rs`

- [x] **Step 1: Write failing tests** (assert_cmd + tempdir as fake project / fake `CLAUDE_CONFIG_DIR`):

```rust
#[test]
fn init_installs_posttooluse_hook() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("stk").unwrap()
        .current_dir(dir.path()).arg("init").assert().success();
    let s: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap()).unwrap();
    let hook = &s["hooks"]["PostToolUse"][0];
    assert_eq!(hook["matcher"], "Read|Grep|Glob|Bash|Edit|Write|NotebookEdit");
    assert!(hook["hooks"][0]["command"].as_str().unwrap().contains("stk hook"));
}

#[test]
fn init_is_idempotent_and_preserves_existing_settings() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::write(dir.path().join(".claude/settings.json"),
        r#"{"permissions":{"allow":["Bash(ls:*)"]},"hooks":{"PostToolUse":[{"matcher":"Other","hooks":[]}]}}"#).unwrap();
    for _ in 0..2 {
        Command::cargo_bin("stk").unwrap().current_dir(dir.path()).arg("init").assert().success();
    }
    let s: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap()).unwrap();
    assert_eq!(s["permissions"]["allow"][0], "Bash(ls:*)");          // untouched
    assert_eq!(s["hooks"]["PostToolUse"].as_array().unwrap().len(), 2); // theirs + ours, once
}
```

- [x] **Step 2: Run — verify FAIL.**

- [x] **Step 3: Implement** `init.rs`: read-modify-write `.claude/settings.json` (`--global` → `~/.claude/settings.json`): parse as `Value`, ensure `hooks.PostToolUse` array, remove any prior entry whose command contains `"stk hook"`, append ours (`command: "stk hook"` — resolved to the absolute binary path via `std::env::current_exe()`), pretty-print back. Also write a starter `.smalltoke.toml` if absent. `doctor.rs`: checks and prints PASS/FAIL lines for: hook entry present; `stk` on PATH or absolute path valid; `.smalltoke/` writable; rtk present on PATH (informational: "bash compression deferred to rtk"). Exit 1 if any FAIL.

- [x] **Step 4: Run — verify PASS.**

- [x] **Step 5: Commit** — `git commit -m "stk init and stk doctor"`

---

### Task 9: `stk stats`

**Files:**
- Create: `crates/stk-cli/src/stats.rs`
- Modify: `crates/stk-cli/src/main.rs`

- [x] **Step 1: Write failing test** (assert_cmd; seed a `.smalltoke/` with two session JSONL files via `Ledger` API or literal lines):

```rust
#[test]
fn stats_reports_totals_and_percent() {
    // seeded: Read in=14000 out=2400 across two sessions
    let assert = Command::cargo_bin("stk").unwrap()
        .current_dir(dir.path()).arg("stats").assert().success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.contains("Read"));
    assert!(out.contains("14000") || out.contains("14,000"));
    assert!(out.contains("83%")); // 1 - 2400/14000
}
```

- [x] **Step 2: Run — verify FAIL.**

- [x] **Step 3: Implement:** glob `.smalltoke/session-*.jsonl`, replay `save` records, print a per-tool table (tokens in, tokens out, saved, %) plus a lifetime total row.

- [x] **Step 4: Run — verify PASS.**

- [x] **Step 5: Commit** — `git commit -m "stk stats savings report"`

**🏁 Phase 1 complete — dogfood checkpoint:** run `stk init` in this repo, develop the remaining tasks with smalltoke active, and check `stk stats` daily. If skeleton views confuse the agent in practice (failed Edits, repeated re-reads), fix before proceeding.

---

### Task 10: Grep match folding

**Files:**
- Create: `crates/stk-compress/src/grep.rs`
- Modify: `crates/stk-hook/src/lib.rs` (dispatch), `crates/stk-hook/tests/end_to_end.rs`

- [ ] **Step 1: Write failing tests** — unit tests in `grep.rs` against constructed match text, plus end-to-end on `grep_many.json`:

```rust
#[test]
fn folds_excess_matches_per_file() {
    let raw = lines_for_file("src/big.rs", 40);            // helper builds "path:line:text" rows
    let out = fold(&raw, &GrepCfg { max_matches_per_file: 5 }).unwrap();
    assert_eq!(count_rows_for(&out, "src/big.rs"), 5);
    assert!(out.contains("+35 more in src/big.rs"));
    assert!(out.contains("path=src/big.rs"));              // escape hatch: re-grep narrowed to that file
}

#[test]
fn dedupes_identical_lines_across_files() {
    let raw = same_line_in_files(r#""lodash": "^4.17.21""#, 30); // 30 lockfile-ish files
    let out = fold(&raw, &GrepCfg::default()).unwrap();
    assert!(out.contains("identical match in 30 files"));
}

#[test]
fn few_matches_pass_through() {
    assert!(fold(&lines_for_file("a.rs", 3), &GrepCfg::default()).is_none());
}
```

- [ ] **Step 2: Run — verify FAIL.**

- [ ] **Step 3: Implement** `fold`: parse `path:line:text` rows (and `path` rows for files_with_matches mode — passthrough that mode unless > `glob`-scale, then fold like Task 11); group by path; per file keep first `max` rows + `(+k more in <path> — rerun Grep with path=<path>)`; hash identical `text` across ≥5 files into one row + file count + first 3 paths. Return `None` when nothing folded or result not ≥30% smaller. Dispatch `Grep` in `run_hook` using the same `extract_content`/re-wrap machinery from Task 6.

- [ ] **Step 4: Run — verify PASS.**

- [ ] **Step 5: Commit** — `git commit -m "Grep match folding"`

---

### Task 11: Glob tree folding

**Files:**
- Create: `crates/stk-compress/src/glob_fold.rs`
- Modify: `crates/stk-hook/src/lib.rs`, `crates/stk-hook/tests/end_to_end.rs`

- [ ] **Step 1: Write failing tests:**

```rust
#[test]
fn folds_large_listing_into_tree() {
    let paths: Vec<String> = (0..300).map(|i| format!("src/components/C{i}.tsx")).collect();
    let out = fold_paths(&paths, 50).unwrap();
    assert!(out.contains("src/components/ (300 files, .tsx)"));
    assert!(out.contains("src/components/**"));   // escape hatch: narrower Glob pattern
    assert!(out.lines().count() < 20);
}

#[test]
fn small_listing_passes_through() {
    let paths: Vec<String> = (0..10).map(|i| format!("src/f{i}.rs")).collect();
    assert!(fold_paths(&paths, 50).is_none());
}
```

- [ ] **Step 2: Run — verify FAIL.**

- [ ] **Step 3: Implement** `fold_paths(paths, max) -> Option<String>`: build a directory trie; render directories with ≤5 entries fully, others as `dir/ (N files, <dominant extensions>)`; banner names the count and shows `Glob(pattern=<dir>/**)` as the expansion route. Dispatch `Glob` in `run_hook`.

- [ ] **Step 4: Run — verify PASS.**

- [ ] **Step 5: Commit** — `git commit -m "Glob tree folding"`

---

### Task 12: Bash minimal noise strip (opt-in) + rtk detection

**Files:**
- Create: `crates/stk-compress/src/bash.rs`
- Modify: `crates/stk-hook/src/lib.rs`, `crates/stk-cli/src/doctor.rs`

- [ ] **Step 1: Write failing tests:**

```rust
#[test]
fn strips_ansi_and_collapses_progress_lines() {
    let noisy = "\x1b[32mok\x1b[0m\n".to_string() +
        &(0..200).map(|i| format!("Downloading [{}%]\r", i / 2)).collect::<String>() +
        "\ndone\n";
    let out = strip(&noisy).unwrap();
    assert!(!out.contains('\x1b'));
    assert!(out.lines().count() < 10);
    assert!(out.contains("done"));
}

#[test]
fn failing_command_output_untouched() {
    // run_hook level: bash fixture mutated to nonzero exit / stderr content
    let payload = bash_payload_with(exit_code = 1, stderr = "assertion failed: left == right");
    assert!(run_hook(&payload, &Config { bash: BashCfg { enabled: true }, ..Default::default() }).is_none());
}

#[test]
fn disabled_by_default() {
    assert!(run_hook(&fixture("bash_simple"), &Config::default()).is_none());
}
```

- [ ] **Step 2: Run — verify FAIL.**

- [ ] **Step 3: Implement** `strip`: regex ANSI removal, split on `\r` keeping final segment per carriage-return run, collapse consecutive near-duplicate lines (same prefix before first digit) to last + `(repeated xN)`, collapse blank runs. Gate in dispatch: only when `cfg.bash.enabled` **and** exit code is 0/absent **and** stderr empty. `stk init` writes `bash.enabled = false` with a comment `# set true if you don't use rtk`; `doctor` reports rtk detection.

- [ ] **Step 4: Run — verify PASS.**

- [ ] **Step 5: Commit** — `git commit -m "Opt-in Bash noise strip with rtk coexistence"`

---

### Task 13: Release hardening

**Files:**
- Create: `README.md`, `.github/workflows/ci.yml`
- Modify: `Cargo.toml` (profile)

- [ ] **Step 1:** `[profile.release] lto = "thin", strip = true`. CI: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` on ubuntu + macos.
- [ ] **Step 2:** README: what it does (one paragraph), the verified mechanism (PostToolUse `updatedToolOutput`), install (`cargo install`, `stk init`), the four invariants from this plan's header, rtk coexistence note, config reference (spec §5 table).
- [ ] **Step 3:** Full local gate: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`. Expected: all green.
- [ ] **Step 4: Commit** — `git commit -m "CI, release profile, README"`

---

## Self-review notes

- **Spec coverage:** spec §4.1→Task 6 (+4), §4.2→Task 10, §4.3→Task 11, §4.4→Task 12, §3 subcommands→Tasks 2/8/9, §5→Task 5, §6 fail-open→Task 3, Edit hazard→Tasks 6+7, §7 testing→every task + invariant tests. Phase 4 (MCP/index) intentionally unplanned per spec §8.
- **Known risk pushed to Task 2 deliberately:** exact `tool_response`/`updatedToolOutput` shapes per tool are recorded from reality, not assumed; `extract_content`/`ContentShape` isolates the rest of the codebase from that uncertainty. **Validation step for Task 6:** after wiring, manually confirm in a live session that Claude actually receives the substituted output for Read (the docs example shows Bash; if Read substitution is unsupported by the harness, fallback strategy is PreToolUse `updatedInput` clamping `limit` + skeleton via `additionalContext` — decision point, stop and reassess with Brandon).
- **Grammar version pins** intentionally resolved at Task 1 implementation time against crates.io (tree-sitter core/grammar compatibility moves fast).
