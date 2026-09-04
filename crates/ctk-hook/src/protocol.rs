//! Tolerant types for the Claude Code PostToolUse hook protocol.
//! Only fields we actually use are typed; everything else stays `Value`
//! so schema drift in the host can't break parsing (fail-open invariant).

#[derive(Debug, serde::Deserialize)]
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
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, serde::Serialize)]
pub struct HookOutput {
    #[serde(rename = "hookSpecificOutput")]
    pub hook_specific_output: HookSpecificOutput,
}

#[derive(Debug, serde::Serialize)]
pub struct HookSpecificOutput {
    #[serde(rename = "hookEventName")]
    pub hook_event_name: &'static str,
    #[serde(rename = "updatedToolOutput")]
    pub updated_tool_output: serde_json::Value,
}

impl HookOutput {
    pub fn updated(tool_output: serde_json::Value) -> Self {
        Self {
            hook_specific_output: HookSpecificOutput {
                hook_event_name: "PostToolUse",
                updated_tool_output: tool_output,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{run_hook, Config};

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/../../tests/fixtures/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    #[test]
    fn parses_all_recorded_fixtures() {
        for f in [
            "read_large",
            "read_offset",
            "grep_many",
            "glob_many",
            "bash_simple",
        ] {
            let p: HookPayload = serde_json::from_str(&fixture(f)).expect(f);
            assert!(!p.tool_name.is_empty(), "{f}");
            assert!(!p.session_id.is_empty(), "{f}");
        }
    }

    #[test]
    fn malformed_input_yields_none_decision() {
        assert!(run_hook("not json", &Config::default()).is_none());
    }

    #[test]
    fn unknown_tool_passes_through() {
        let payload = r#"{"tool_name":"SomeFutureTool","tool_response":{"x":1}}"#;
        assert!(run_hook(payload, &Config::default()).is_none());
    }

    #[test]
    fn output_serializes_with_camel_case_keys() {
        let out = HookOutput::updated(serde_json::json!({"stdout": "hi"}));
        let s = serde_json::to_string(&out).unwrap();
        assert!(s.contains("hookSpecificOutput"));
        assert!(s.contains("hookEventName"));
        assert!(s.contains("updatedToolOutput"));
        assert!(s.contains("PostToolUse"));
    }
}
