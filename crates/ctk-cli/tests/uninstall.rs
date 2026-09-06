//! `ctk uninstall` is the exact inverse of `ctk init`: it removes the hook
//! entry from the same settings file `init` wrote, and touches nothing else.

use assert_cmd::Command;

fn settings(dir: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join(".claude/settings.local.json")).unwrap())
        .unwrap()
}

fn init(dir: &std::path::Path) {
    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir)
        .arg("init")
        .assert()
        .success();
}

fn uninstall(dir: &std::path::Path) -> assert_cmd::assert::Assert {
    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir)
        .arg("uninstall")
        .assert()
        .success()
}

#[test]
fn uninstall_removes_the_hook_that_init_added() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path());
    uninstall(dir.path());

    // The entry is gone, and so is the empty scaffolding around it: a
    // leftover `"hooks": {"PostToolUse": []}` is litter we created.
    let s = settings(dir.path());
    assert_eq!(s.pointer("/hooks/PostToolUse"), None, "settings were: {s}");
    assert_eq!(s.get("hooks"), None, "empty hooks object left behind: {s}");
}

#[test]
fn uninstall_preserves_unrelated_hooks_and_settings() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::write(
        dir.path().join(".claude/settings.local.json"),
        r#"{"permissions":{"allow":["Bash(ls:*)"]},"hooks":{"PostToolUse":[{"matcher":"Other","hooks":[]}],"PreToolUse":[{"matcher":"Bash","hooks":[]}]}}"#,
    )
    .unwrap();
    init(dir.path());
    uninstall(dir.path());

    let s = settings(dir.path());
    assert_eq!(s["permissions"]["allow"][0], "Bash(ls:*)");
    let post = s["hooks"]["PostToolUse"].as_array().unwrap();
    assert_eq!(post.len(), 1, "only ours should go: {post:?}");
    assert_eq!(post[0]["matcher"], "Other");
    assert!(s["hooks"]["PreToolUse"].is_array(), "settings were: {s}");
}

#[test]
fn uninstall_does_not_remove_a_lookalike_command() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::write(
        dir.path().join(".claude/settings.local.json"),
        r#"{"hooks":{"PostToolUse":[{"matcher":"Other","hooks":[{"type":"command","command":"/usr/local/bin/protectk hook"}]}]}}"#,
    )
    .unwrap();
    uninstall(dir.path());

    let post = settings(dir.path())["hooks"]["PostToolUse"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(post, 1, "protectk is not ours to remove");
}

#[test]
fn uninstall_succeeds_when_nothing_is_installed() {
    let dir = tempfile::tempdir().unwrap();
    uninstall(dir.path());
    // Nothing installed means nothing to write: don't create a settings file
    // just to record that we removed nothing from it.
    assert!(!dir.path().join(".claude/settings.local.json").exists());
}

#[test]
fn uninstall_global_honors_claude_config_dir() {
    let dir = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    for args in [["init", "--global"], ["uninstall", "--global"]] {
        Command::cargo_bin("ctk")
            .unwrap()
            .current_dir(dir.path())
            .env("CLAUDE_CONFIG_DIR", config.path())
            .args(args)
            .assert()
            .success();
    }
    let s: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(config.path().join("settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(s.pointer("/hooks/PostToolUse"), None, "settings were: {s}");
}

#[test]
fn uninstall_keeps_user_config_and_ledger() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path());
    std::fs::create_dir_all(dir.path().join(".cubtoken")).unwrap();
    std::fs::write(dir.path().join(".cubtoken/session-x.jsonl"), "{}\n").unwrap();
    uninstall(dir.path());

    // Config and recorded savings are the user's data, not install state.
    assert!(dir.path().join(".cubtoken.toml").exists());
    assert!(dir.path().join(".cubtoken/session-x.jsonl").exists());
}
