pub mod protocol;

pub use protocol::{HookOutput, HookPayload};
pub use stk_compress::Config;

/// Core hook entry point: raw stdin JSON in, optional decision JSON out.
/// `None` means pass through (emit nothing, keep the original tool output).
/// Every error path returns `None` — the fail-open invariant.
pub fn run_hook(stdin: &str, cfg: &Config) -> Option<String> {
    let payload: HookPayload = serde_json::from_str(stdin).ok()?;
    let updated = dispatch(&payload, cfg)?;
    serde_json::to_string(&HookOutput::updated(updated)).ok()
}

/// Per-tool compressor dispatch. Returns the replacement `tool_response`
/// value, or `None` to pass through. (Grep: Task 10, Glob: Task 11,
/// Bash: Task 12.)
fn dispatch(payload: &HookPayload, cfg: &Config) -> Option<serde_json::Value> {
    match payload.tool_name.as_str() {
        "Read" => {
            stk_compress::read::compress_read(&payload.tool_input, &payload.tool_response, cfg)
                .map(|o| o.updated_response)
        }
        _ => None,
    }
}
