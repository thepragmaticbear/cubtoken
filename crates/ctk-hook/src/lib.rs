pub mod adaptive;
pub mod adaptive_state;
pub mod attribution;
pub mod ledger;
pub mod protocol;

pub use ctk_compress::config::project_root;
pub use ctk_compress::Config;
pub use protocol::{HookEnvelope, HookEvent, HookOutput, HookPayload};

use ledger::Ledger;

/// Core hook entry point: raw stdin JSON in, optional decision JSON out.
/// `None` means pass through (emit nothing, keep the original tool output).
/// Every error path returns `None` — the fail-open invariant.
pub fn run_hook(stdin: &str, cfg: &Config) -> Option<String> {
    let payload: HookPayload = serde_json::from_str(stdin).ok()?;
    emit_for_event(&payload, cfg)
}

/// Production entry point: like [`run_hook`] but loads layered config from the
/// payload's `cwd` (global `~/.config/cubtoken/config.toml`, then that
/// project's `.cubtoken.toml`) rather than taking a `Config`. Fail-open: a
/// parse failure or absent config falls back to defaults / pass-through.
pub fn run_hook_auto(stdin: &str) -> Option<String> {
    let payload: HookPayload = serde_json::from_str(stdin).ok()?;
    let cfg = Config::try_load_for(std::path::Path::new(&payload.cwd)).ok()?;
    emit_for_event(&payload, &cfg)
}

const CONCISE_OUTPUT_INSTRUCTION: &str = "Do not narrate routine tool use or restate tool output.\nAfter successful work, report only material changes, failures,\nand required next actions unless the user asks for more detail.";

fn emit_for_event(payload: &HookPayload, cfg: &Config) -> Option<String> {
    let output = match payload.event() {
        HookEvent::SessionStart if cfg.output.mode == ctk_compress::config::OutputMode::Concise => {
            HookOutput::additional_context(CONCISE_OUTPUT_INSTRUCTION)
        }
        HookEvent::PostToolUse => HookOutput::updated(dispatch(payload, cfg)?),
        HookEvent::UserPromptSubmit => {
            if cfg.stats.ledger {
                ledger_for(payload).note_turn_start();
            }
            return None;
        }
        HookEvent::PostToolBatch => {
            if cfg.stats.ledger {
                ledger_for(payload).note_batch_end();
            }
            return None;
        }
        HookEvent::Stop => {
            if cfg.stats.ledger {
                if let Some(message) = payload.last_assistant_message.as_deref() {
                    ledger_for(payload)
                        .note_visible_output(ctk_compress::estimate::est_tokens(message));
                }
            }
            refresh_adaptive(payload, cfg);
            return None;
        }
        // These events only finalize state in later adaptive phases. They must
        // remain silent even when the host adds fields we do not understand.
        HookEvent::SessionEnd => {
            refresh_adaptive(payload, cfg);
            return None;
        }
        HookEvent::StopFailure | HookEvent::SessionStart | HookEvent::Unknown => {
            return None;
        }
    };
    serde_json::to_string(&output).ok()
}

const EDIT_TOOLS: &[&str] = &["Edit", "Write", "NotebookEdit"];

fn ledger_for(payload: &HookPayload) -> Ledger {
    Ledger::open(&data_dir_for(payload), &payload.session_id)
}

fn data_dir_for(payload: &HookPayload) -> std::path::PathBuf {
    ctk_compress::config::project_root(std::path::Path::new(&payload.cwd)).join(".cubtoken")
}

fn refresh_adaptive(payload: &HookPayload, cfg: &Config) {
    if cfg.adaptive.mode != ctk_compress::config::AdaptiveMode::Off {
        let _ = adaptive_state::refresh(&data_dir_for(payload), unix_millis());
    }
}

fn effective_read_threshold(payload: &HookPayload, cfg: &Config) -> usize {
    if cfg.adaptive.mode != ctk_compress::config::AdaptiveMode::Safe {
        return cfg.read.threshold_tokens;
    }
    let Some(preview) =
        ctk_compress::read::preview_read(&payload.tool_input, &payload.tool_response)
    else {
        return cfg.read.threshold_tokens;
    };
    let state = adaptive_state::load(&data_dir_for(payload), unix_millis());
    let key = adaptive::bucket_key(&preview.language, preview.tokens_in, &preview.strategy);
    adaptive_state::recommendation(&state, &key).effective_threshold(cfg.read.threshold_tokens)
}

fn record_targeted_recovery(ledger: &mut Ledger, payload: &HookPayload, identity: &str) {
    let Some(decision) = ledger.latest_decision(identity).cloned() else {
        return;
    };
    let Some(returned) = attribution::returned_range(&payload.tool_input, &payload.tool_response)
    else {
        return;
    };
    let confidence = attribution::classify_targeted(
        &decision,
        ledger.current_turn(),
        ledger.current_batch(),
        fingerprint_matches(identity, &decision),
        ledger.has_edit_after(&decision),
        &returned,
    );
    if let Some(confidence) = confidence {
        ledger.note_recovery(ledger::Recovery {
            path: identity.to_string(),
            tokens: ctk_compress::read::response_tokens(&payload.tool_response),
            duration_ms: payload.duration_ms.unwrap_or(0),
            decision_id: Some(decision.decision_id),
            kind: Some(ledger::RecoveryKind::Targeted),
            confidence: Some(confidence),
            turn: Some(ledger.current_turn()),
            batch: Some(ledger.current_batch()),
        });
    }
}

fn record_full_repeat_escape(ledger: &mut Ledger, payload: &HookPayload, identity: &str) -> bool {
    let Some(decision) = ledger.latest_decision(identity).cloned() else {
        return false;
    };
    if ledger.recoveries().iter().any(|recovery| {
        recovery.decision_id.as_deref() == Some(decision.decision_id.as_str())
            && recovery.kind == Some(ledger::RecoveryKind::FullRepeat)
    }) {
        return false;
    }
    let confidence = attribution::classify_full_repeat(
        &decision,
        ledger.current_turn(),
        ledger.current_batch(),
        fingerprint_matches(identity, &decision),
        ledger.has_edit_after(&decision),
    );
    let Some(confidence) = confidence else {
        return false;
    };
    ledger.note_recovery(ledger::Recovery {
        path: identity.to_string(),
        tokens: ctk_compress::read::response_tokens(&payload.tool_response),
        duration_ms: payload.duration_ms.unwrap_or(0),
        decision_id: Some(decision.decision_id),
        kind: Some(ledger::RecoveryKind::FullRepeat),
        confidence: Some(confidence),
        turn: Some(ledger.current_turn()),
        batch: Some(ledger.current_batch()),
    });
    confidence == ledger::Confidence::High
}

fn fingerprint_matches(identity: &str, decision: &ledger::CompressionDecision) -> bool {
    std::fs::read_to_string(identity)
        .ok()
        .is_some_and(|content| {
            ctk_compress::read::content_fingerprint(&content) == decision.content_fingerprint
        })
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

/// Per-tool compressor dispatch. Returns the replacement `tool_response`
/// value, or `None` to pass through. (Grep: Task 10, Glob: Task 11,
/// Bash: Task 12.)
fn dispatch(payload: &HookPayload, cfg: &Config) -> Option<serde_json::Value> {
    if EDIT_TOOLS.contains(&payload.tool_name.as_str()) {
        // edit protection always on: a session-edited file is never compressed
        let path_key = if payload.tool_name == "NotebookEdit" {
            "notebook_path"
        } else {
            "file_path"
        };
        if let Some(path) = payload.tool_input.get(path_key).and_then(|v| v.as_str()) {
            let identity =
                ledger::normalize_file_identity(std::path::Path::new(&payload.cwd), path);
            ledger_for(payload).note_edit_identity(&identity);
        }
        return None;
    }
    match payload.tool_name.as_str() {
        "Read" => {
            let mut ledger = ledger_for(payload);
            if !ledger.lock_acquired() {
                return None;
            }
            let path = payload
                .tool_input
                .get("file_path")
                .and_then(|v| v.as_str())?;
            let response_path = payload
                .tool_response
                .pointer("/file/filePath")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(path);
            let identity =
                ledger::normalize_file_identity(std::path::Path::new(&payload.cwd), response_path);
            if ledger.is_protected(&identity) || ledger.is_protected(path) {
                return None;
            }
            let targeted = payload.tool_input.get("offset").is_some()
                || payload.tool_input.get("limit").is_some();
            if targeted {
                if cfg.stats.ledger {
                    let before = ledger.recoveries().len();
                    record_targeted_recovery(&mut ledger, payload, &identity);
                    // Preserve the original broad counter for historical stats.
                    // Its unlabelled records are intentionally excluded from
                    // adaptive learning and attributed recovery overhead.
                    if before == ledger.recoveries().len()
                        && (ledger.was_compressed(&identity) || ledger.was_compressed(path))
                    {
                        ledger.note_refetch(
                            &identity,
                            ctk_compress::read::response_tokens(&payload.tool_response),
                            payload.duration_ms.unwrap_or(0),
                        );
                    }
                }
                return None;
            }
            if cfg.stats.ledger && record_full_repeat_escape(&mut ledger, payload, &identity) {
                return None;
            }
            let outcome = ctk_compress::read::compress_read_with_threshold(
                &payload.tool_input,
                &payload.tool_response,
                cfg,
                effective_read_threshold(payload, cfg),
            )?;
            if cfg.stats.ledger {
                let sequence = ledger.next_sequence();
                let decision_id = payload
                    .tool_use_id
                    .clone()
                    .unwrap_or_else(|| format!("{}:{sequence}", payload.session_id));
                ledger.note_decision(ledger::CompressionDecision {
                    decision_id,
                    turn: ledger.current_turn(),
                    batch: ledger.current_batch(),
                    sequence,
                    recorded_at_ms: unix_millis(),
                    file_identity: identity,
                    content_fingerprint: outcome.metadata.content_fingerprint,
                    language: outcome.metadata.language,
                    strategy: outcome.metadata.strategy,
                    profile: "default".to_string(),
                    tokens_in: outcome.tokens_in,
                    tokens_out: outcome.tokens_out,
                    elided_ranges: outcome
                        .metadata
                        .elided_ranges
                        .into_iter()
                        .map(|range| ledger::ElidedRange {
                            start_line: range.start_line,
                            end_line: range.end_line,
                        })
                        .collect(),
                });
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
