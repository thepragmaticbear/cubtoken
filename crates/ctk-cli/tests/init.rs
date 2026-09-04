use assert_cmd::Command;

const MATCHER: &str = "Read|Grep|Glob|Bash|Edit|Write|NotebookEdit";

fn settings(dir: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join(".claude/settings.json")).unwrap())
        .unwrap()
}

#[test]
fn init_installs_posttooluse_hook() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("ctk")
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
        cmd.ends_with("ctk hook") || cmd.contains("ctk\" hook"),
        "command was: {cmd}"
    );
    assert_eq!(hook["hooks"][0]["type"], "command");
    // starter project config written
    assert!(dir.path().join(".cubtoken.toml").exists());
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
        Command::cargo_bin("ctk")
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
fn init_does_not_overwrite_existing_cubtoken_toml() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".cubtoken.toml"),
        "[read]\nthreshold_tokens = 99\n",
    )
    .unwrap();
    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    let kept = std::fs::read_to_string(dir.path().join(".cubtoken.toml")).unwrap();
    assert!(kept.contains("99"));
}

#[test]
fn doctor_passes_after_init_and_fails_before() {
    let dir = tempfile::tempdir().unwrap();
    // Hermetic HOME: doctor also checks `$HOME/.claude/settings.json`, so an
    // empty HOME keeps a real global install from leaking into the "before" check.
    let home = tempfile::tempdir().unwrap();
    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .env("HOME", home.path())
        .arg("doctor")
        .assert()
        .failure();
    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .env("HOME", home.path())
        .arg("init")
        .assert()
        .success();
    let assert = Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .env("HOME", home.path())
        .arg("doctor")
        .assert()
        .success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.contains("PASS"), "doctor output: {out}");
}

#[test]
fn doctor_finds_a_hook_in_settings_local_json() {
    // `init` writes an absolute binary path, so users installing per-machine
    // land in settings.local.json. Doctor used to ignore that file and report
    // FAIL on a working install.
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let bin = dir.path().join("ctk");
    std::fs::write(&bin, "#!/bin/sh\n").unwrap();
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::write(
        dir.path().join(".claude/settings.local.json"),
        serde_json::json!({"hooks": {"PostToolUse": [{
            "matcher": MATCHER,
            "hooks": [{"type": "command", "command": format!("{} hook", bin.display())}]
        }]}})
        .to_string(),
    )
    .unwrap();

    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .env("HOME", home.path())
        .arg("doctor")
        .assert()
        .success();
}

#[test]
fn doctor_fails_when_the_installed_binary_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::write(
        dir.path().join(".claude/settings.json"),
        serde_json::json!({"hooks": {"PostToolUse": [{
            "matcher": MATCHER,
            "hooks": [{"type": "command", "command": "/nonexistent/build/dir/ctk hook"}]
        }]}})
        .to_string(),
    )
    .unwrap();

    let assert = Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .env("HOME", home.path())
        .arg("doctor")
        .assert()
        .failure();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.contains("FAIL  hook binary exists"), "got: {out}");
}
