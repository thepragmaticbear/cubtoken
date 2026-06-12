pub mod ledger;
pub mod protocol;

pub use protocol::{HookOutput, HookPayload};
pub use stk_compress::Config;

use ledger::Ledger;

/// Core hook entry point: raw stdin JSON in, optional decision JSON out.
/// `None` means pass through (emit nothing, keep the original tool output).
/// Every error path returns `None` — the fail-open invariant.
pub fn run_hook(stdin: &str, cfg: &Config) -> Option<String> {
    let payload: HookPayload = serde_json::from_str(stdin).ok()?;
    let updated = dispatch(&payload, cfg)?;
    serde_json::to_string(&HookOutput::updated(updated)).ok()
}

const EDIT_TOOLS: &[&str] = &["Edit", "Write", "NotebookEdit"];

fn ledger_for(payload: &HookPayload) -> Ledger {
    let dir = std::path::Path::new(&payload.cwd).join(".smalltoke");
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
            let outcome = stk_compress::read::compress_read(
                &payload.tool_input,
                &payload.tool_response,
                cfg,
            )?;
            if cfg.stats.ledger {
                ledger.note_saving("Read", outcome.tokens_in, outcome.tokens_out);
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
            let folded = stk_compress::grep::fold(content, &cfg.grep)?;
            if cfg.stats.ledger {
                use stk_compress::estimate::est_tokens;
                ledger_for(payload).note_saving("Grep", est_tokens(content), est_tokens(&folded));
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
            let folded = stk_compress::glob_fold::fold_paths(&filenames, cfg.glob.max_paths)?;
            if cfg.stats.ledger {
                use stk_compress::estimate::est_tokens;
                let orig = filenames.join("\n");
                ledger_for(payload).note_saving("Glob", est_tokens(&orig), est_tokens(&folded));
            }
            let mut updated = payload.tool_response.clone();
            updated["filenames"] = serde_json::json!([folded]);
            updated["numFiles"] = serde_json::Value::from(1);
            Some(updated)
        }
        _ => None,
    }
}
