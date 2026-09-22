//! Tolerant types for Claude Code hook events.
//! Only fields we actually use are typed; everything else stays `Value`
//! so schema drift in the host can't break parsing (fail-open invariant).

#[derive(Debug, serde::Deserialize)]
pub struct HookEnvelope {
    #[serde(default)]
    pub hook_event_name: String,
    #[serde(default)]
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
    #[serde(default)]
    pub tool_use_id: Option<String>,
    #[serde(default)]
    pub last_assistant_message: Option<String>,
}

/// Backward-compatible name for callers that only handle PostToolUse.
pub type HookPayload = HookEnvelope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart,
    UserPromptSubmit,
    PostToolUse,
    PostToolBatch,
    Stop,
    StopFailure,
    SessionEnd,
    Unknown,
}

impl HookEnvelope {
    /// Legacy recorded fixtures did not include `hook_event_name`; a tool name
    /// is enough to unambiguously retain their PostToolUse meaning.
    pub fn event(&self) -> HookEvent {
        match self.hook_event_name.as_str() {
            "SessionStart" => HookEvent::SessionStart,
            "UserPromptSubmit" => HookEvent::UserPromptSubmit,
            "PostToolUse" => HookEvent::PostToolUse,
            "PostToolBatch" => HookEvent::PostToolBatch,
            "Stop" => HookEvent::Stop,
            "StopFailure" => HookEvent::StopFailure,
            "SessionEnd" => HookEvent::SessionEnd,
            "" if !self.tool_name.is_empty() => HookEvent::PostToolUse,
            _ => HookEvent::Unknown,
        }
    }
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
    #[serde(rename = "updatedToolOutput", skip_serializing_if = "Option::is_none")]
    pub updated_tool_output: Option<serde_json::Value>,
    #[serde(rename = "additionalContext", skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<&'static str>,
}

impl HookOutput {
    pub fn updated(tool_output: serde_json::Value) -> Self {
        Self {
            hook_specific_output: HookSpecificOutput {
                hook_event_name: "PostToolUse",
                updated_tool_output: Some(tool_output),
                additional_context: None,
            },
        }
    }

    pub fn additional_context(context: &'static str) -> Self {
        Self {
            hook_specific_output: HookSpecificOutput {
                hook_event_name: "SessionStart",
                updated_tool_output: None,
                additional_context: Some(context),
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

    #[test]
    fn legacy_tool_payload_is_post_tool_use() {
        let payload: HookPayload = serde_json::from_str(
            r#"{"tool_name":"Read","tool_response":{"file":{"content":"x"}}}"#,
        )
        .unwrap();
        assert_eq!(payload.event(), HookEvent::PostToolUse);
    }

    #[test]
    fn session_start_additional_context_has_no_tool_replacement() {
        let out = serde_json::to_string(&HookOutput::additional_context("be concise")).unwrap();
        assert!(out.contains("additionalContext"));
        assert!(!out.contains("updatedToolOutput"));
    }
}
