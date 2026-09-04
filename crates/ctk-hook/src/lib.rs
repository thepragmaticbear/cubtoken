pub mod ledger;
pub mod protocol;

pub use ctk_compress::Config;
pub use protocol::{HookOutput, HookPayload};

use ledger::Ledger;

/// Core hook entry point: raw stdin JSON in, optional decision JSON out.
/// `None` means pass through (emit nothing, keep the original tool output).
/// Every error path returns `None` — the fail-open invariant.
pub fn run_hook(stdin: &str, cfg: &Config) -> Option<String> {
    let payload: HookPayload = serde_json::from_str(stdin).ok()?;
    let updated = dispatch(&payload, cfg)?;
    serde_json::to_string(&HookOutput::updated(updated)).ok()
}

/// Production entry point: like [`run_hook`] but loads layered config from the
/// payload's `cwd` (global `~/.config/cubtoken/config.toml`, then that
/// project's `.cubtoken.toml`) rather than taking a `Config`. Fail-open: a
/// parse failure or absent config falls back to defaults / pass-through.
pub fn run_hook_auto(stdin: &str) -> Option<String> {
    let payload: HookPayload = serde_json::from_str(stdin).ok()?;
    let cfg = Config::load_for(std::path::Path::new(&payload.cwd));
    let updated = dispatch(&payload, &cfg)?;
    serde_json::to_string(&HookOutput::updated(updated)).ok()
}

const EDIT_TOOLS: &[&str] = &["Edit", "Write", "NotebookEdit"];

fn ledger_for(payload: &HookPayload) -> Ledger {
    let dir = std::path::Path::new(&payload.cwd).join(".cubtoken");
    Ledger::open(&dir, &payload.session_id)
}

/// Per-tool compressor dispatch. Returns the replacement `tool_response`
/// value, or `None` to pass through. (Grep: Task 10, Glob: Task 11,
/// Bash: Task 12.)
fn dispatch(payload: &HookPayload, cfg: &Config) -> Option<serde_json::Value> {
    if EDIT_TOOLS.contains(&payload.tool_name.as_str()) {
        // edit protection always on: a session-edited file is never compressed
        if let Some(path) = payload.tool_input.get("file_path").and_then(|v| v.as_str()) {
            ledger_for(payload).note_edit(path);
        }
        return None;
    }
    match payload.tool_name.as_str() {
        "Read" => {
            let mut ledger = ledger_for(payload);
            let path = payload
                .tool_input
                .get("file_path")
                .and_then(|v| v.as_str())?;
            if ledger.is_protected(path) {
                return None;
            }
            // A targeted Read into a file we already compressed is the model
            // buying back what we elided — the cost side of the savings
            // number. Record it before passing through.
            if payload.tool_input.get("offset").is_some()
                || payload.tool_input.get("limit").is_some()
            {
                if cfg.stats.ledger && ledger.was_compressed(path) {
                    ledger.note_refetch(path);
                }
                return None;
            }
            let outcome = ctk_compress::read::compress_read(
                &payload.tool_input,
                &payload.tool_response,
                cfg,
            )?;
            if cfg.stats.ledger {
                ledger.note_saving("Read", Some(path), outcome.tokens_in, outcome.tokens_out);
            }
            Some(outcome.updated_response)
        }
        "Grep" => {
            if !cfg.grep.enabled {
                return None;
            }
            // only content mode has match rows worth folding
            if payload.tool_response.get("mode").and_then(|m| m.as_str()) != Some("content") {
                return None;
            }
            let content = payload.tool_response.get("content")?.as_str()?;
            let folded = ctk_compress::grep::fold(content, &cfg.grep)?;
            if cfg.stats.ledger {
                use ctk_compress::estimate::est_tokens;
                ledger_for(payload).note_saving(
                    "Grep",
                    None,
                    est_tokens(content),
                    est_tokens(&folded),
                );
            }
            let mut updated = payload.tool_response.clone();
            updated["content"] = serde_json::Value::String(folded.clone());
            updated["numLines"] = serde_json::Value::from(folded.lines().count());
            Some(updated)
        }
        "Glob" => {
            if !cfg.glob.enabled {
                return None;
            }
            let filenames: Vec<String> = payload
                .tool_response
                .get("filenames")?
                .as_array()?
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            let folded = ctk_compress::glob_fold::fold_paths(&filenames, cfg.glob.max_paths)?;
            if cfg.stats.ledger {
                use ctk_compress::estimate::est_tokens;
                let orig = filenames.join("\n");
                ledger_for(payload).note_saving(
                    "Glob",
                    None,
                    est_tokens(&orig),
                    est_tokens(&folded),
                );
            }
            let mut updated = payload.tool_response.clone();
            updated["filenames"] = serde_json::json!([folded]);
            // filenames now holds one folded blob, but the match count is a
            // fact about the search — keep it true.
            updated["numFiles"] = serde_json::Value::from(filenames.len());
            Some(updated)
        }
        "Bash" => {
            if !cfg.bash.enabled {
                return None;
            }
            // never touch failing/interrupted commands: stderr and exit
            // context are sacred. The recorded response shape has no exit
            // code field, so non-empty stderr is the failure heuristic.
            let stderr = payload.tool_response.get("stderr").and_then(|v| v.as_str());
            if stderr.map(|s| !s.trim().is_empty()).unwrap_or(true) {
                return None;
            }
            if payload
                .tool_response
                .get("interrupted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return None;
            }
            let stdout = payload.tool_response.get("stdout")?.as_str()?;
            let stripped = ctk_compress::bash::strip(stdout)?;
            if cfg.stats.ledger {
                use ctk_compress::estimate::est_tokens;
                ledger_for(payload).note_saving(
                    "Bash",
                    None,
                    est_tokens(stdout),
                    est_tokens(&stripped),
                );
            }
            let mut updated = payload.tool_response.clone();
            updated["stdout"] = serde_json::Value::String(stripped);
            Some(updated)
        }
        _ => None,
    }
}
