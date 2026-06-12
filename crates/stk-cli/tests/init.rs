use assert_cmd::Command;

const MATCHER: &str = "Read|Grep|Glob|Bash|Edit|Write|NotebookEdit";

fn settings(dir: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join(".claude/settings.json")).unwrap())
        .unwrap()
}

#[test]
fn init_installs_posttooluse_hook() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("stk")
        .unwrap()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    let s = settings(dir.path());
    let hook = &s["hooks"]["PostToolUse"][0];
    assert_eq!(hook["matcher"], MATCHER);
    let cmd = hook["hooks"][0]["command"].as_str().unwrap();
    assert!(
        cmd.ends_with("stk hook") || cmd.contains("stk\" hook"),
        "command was: {cmd}"
    );
    assert_eq!(hook["hooks"][0]["type"], "command");
    // starter project config written
    assert!(dir.path().join(".smalltoke.toml").exists());
}

#[test]
fn init_is_idempotent_and_preserves_existing_settings() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::write(
        dir.path().join(".claude/settings.json"),
        r#"{"permissions":{"allow":["Bash(ls:*)"]},"hooks":{"PostToolUse":[{"matcher":"Other","hooks":[]}]}}"#,
    )
    .unwrap();
    for _ in 0..2 {
        Command::cargo_bin("stk")
            .unwrap()
            .current_dir(dir.path())
            .arg("init")
            .assert()
            .success();
    }
    let s = settings(dir.path());
    assert_eq!(s["permissions"]["allow"][0], "Bash(ls:*)"); // untouched
    let hooks = s["hooks"]["PostToolUse"].as_array().unwrap();
    assert_eq!(hooks.len(), 2, "theirs + ours exactly once: {hooks:?}");
    assert_eq!(hooks[0]["matcher"], "Other");
}

#[test]
fn init_does_not_overwrite_existing_smalltoke_toml() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".smalltoke.toml"),
        "[read]\nthreshold_tokens = 99\n",
    )
    .unwrap();
    Command::cargo_bin("stk")
        .unwrap()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    let kept = std::fs::read_to_string(dir.path().join(".smalltoke.toml")).unwrap();
    assert!(kept.contains("99"));
}

#[test]
fn doctor_passes_after_init_and_fails_before() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("stk")
        .unwrap()
        .current_dir(dir.path())
        .arg("doctor")
        .assert()
        .failure();
    Command::cargo_bin("stk")
        .unwrap()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    let assert = Command::cargo_bin("stk")
        .unwrap()
        .current_dir(dir.path())
        .arg("doctor")
        .assert()
        .success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.contains("PASS"), "doctor output: {out}");
}
