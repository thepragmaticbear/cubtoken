//! The shipped Claude Code plugin (`plugins/cubtoken/`) is an alternative to
//! `ctk init`: `claude plugin install` instead of editing settings.json. These
//! tests pin the parts that silently rot — the matcher drifting away from
//! `init`'s, and the wrapper losing its fail-open behaviour.

use std::path::{Path, PathBuf};
// Only the two wrapper tests below run the shell script, and those are
// unix-only — leaving this ungated fails `-D warnings` on the Windows runner.
#[cfg(unix)]
use std::process::{Command, Stdio};

/// Mirrors `init::MATCHER`; `matcher_matches_init` is the guard that they stay
/// equal (the constant lives in a binary crate, so tests can't import it).
const MATCHER: &str = "Read|Grep|Glob|Bash|Edit|Write|NotebookEdit";

fn plugin_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/cubtoken")
}

fn json(path: &Path) -> serde_json::Value {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn manifest_declares_a_name() {
    let manifest = json(&plugin_dir().join(".claude-plugin/plugin.json"));
    assert_eq!(manifest["name"], "cubtoken");
    assert!(manifest["description"].is_string());
}

#[test]
fn matcher_matches_init() {
    let hooks = json(&plugin_dir().join("hooks/hooks.json"));
    let entry = &hooks["hooks"]["PostToolUse"][0];
    assert_eq!(
        entry["matcher"], MATCHER,
        "plugin matcher drifted from init::MATCHER"
    );
    assert_eq!(entry["hooks"][0]["type"], "command");
}

#[test]
fn hook_command_resolves_through_the_plugin_root() {
    let hooks = json(&plugin_dir().join("hooks/hooks.json"));
    let cmd = hooks["hooks"]["PostToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    // An absolute path baked at install time is what `init` does and what
    // `cargo clean` breaks; the plugin must stay relocatable instead.
    assert!(cmd.contains("${CLAUDE_PLUGIN_ROOT}"), "command was: {cmd}");
    assert!(plugin_dir().join("bin/ctk-hook").exists());
}

/// Invariant 1 (fail open): with no `ctk` anywhere on PATH the wrapper must
/// still exit 0 and print nothing, so Claude Code keeps the original output.
#[cfg(unix)]
#[test]
fn wrapper_fails_open_when_ctk_is_missing() {
    let empty = tempfile::tempdir().unwrap();
    let mut child = Command::new("/bin/sh")
        .arg(plugin_dir().join("bin/ctk-hook"))
        .env("PATH", empty.path())
        .env("HOME", empty.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Deliberately larger than a pipe buffer (64KB on Linux). A payload that
    // fits gets buffered by the kernel, so the write succeeds whether or not
    // anything reads it — which would let a wrapper that never drains stdin
    // pass here and hand Claude Code an EPIPE in real use.
    let payload = format!(
        r#"{{"tool_name":"Read","tool_response":{{"pad":"{}"}}}}"#,
        "x".repeat(256 * 1024)
    );
    use std::io::Write as _;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .expect("wrapper closed stdin without draining it (broken pipe)");
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "wrapper exited {:?}", out.status);
    assert!(
        out.stdout.is_empty(),
        "wrapper printed: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// The happy path: a `ctk` on PATH is found and its decision reaches stdout.
#[cfg(unix)]
#[test]
fn wrapper_forwards_to_ctk_on_path() {
    let bin_dir = tempfile::tempdir().unwrap();
    let real = assert_cmd::cargo::cargo_bin("ctk");
    std::os::unix::fs::symlink(&real, bin_dir.path().join("ctk")).unwrap();

    // Comfortably over the default 2000-token threshold: one signature over a
    // fat body is the shape that actually compresses.
    let content =
        "pub fn a() -> usize {\n".to_string() + &"    let x = 1;\n".repeat(1500) + "    0\n}\n";
    let lines = content.lines().count();
    let payload = serde_json::json!({
        "tool_name": "Read",
        "session_id": "plugin-test",
        "cwd": bin_dir.path(),
        "tool_input": {"file_path": "/tmp/cubtoken-plugin-test.rs"},
        "tool_response": {"type": "text", "file": {
            "filePath": "/tmp/cubtoken-plugin-test.rs",
            "content": content, "numLines": lines, "startLine": 1, "totalLines": lines
        }}
    });

    let mut child = Command::new("/bin/sh")
        .arg(plugin_dir().join("bin/ctk-hook"))
        .env("PATH", bin_dir.path())
        .current_dir(bin_dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write as _;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let decision: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("wrapper produced no decision JSON");
    assert_eq!(
        decision["hookSpecificOutput"]["hookEventName"],
        "PostToolUse"
    );
}

/// Event parity, checked against what `ctk init` actually writes rather than a
/// mirrored constant. Attribution depends on `PostToolBatch` for its batch
/// boundary and on `UserPromptSubmit` for its turn boundary: a plugin install
/// that silently lacks either one downgrades every recovery to a lower
/// confidence, so the two install paths must subscribe to the same events.
#[test]
fn plugin_events_match_init() {
    let dir = tempfile::tempdir().unwrap();
    assert_cmd::Command::cargo_bin("ctk")
        .unwrap()
        .arg("init")
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .assert()
        .success();

    let installed = json(&dir.path().join(".claude/settings.local.json"));
    let plugin = json(&plugin_dir().join("hooks/hooks.json"));

    let names = |value: &serde_json::Value| {
        let mut events: Vec<String> = value["hooks"]
            .as_object()
            .expect("hooks is not an object")
            .keys()
            .cloned()
            .collect();
        events.sort();
        events
    };
    assert_eq!(
        names(&installed),
        names(&plugin),
        "plugin hooks.json subscribes to different events than `ctk init`"
    );

    // Only PostToolUse is filtered, and both paths must filter it identically.
    for event in names(&plugin) {
        let installed_matcher = installed["hooks"][&event][0].get("matcher");
        let plugin_matcher = plugin["hooks"][&event][0].get("matcher");
        assert_eq!(
            installed_matcher, plugin_matcher,
            "matcher for {event} differs between `ctk init` and the plugin"
        );
    }
    assert_eq!(plugin["hooks"]["PostToolUse"][0]["matcher"], MATCHER);
}
