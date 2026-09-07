use assert_cmd::Command;

const MATCHER: &str = "Read|Grep|Glob|Bash|Edit|Write|NotebookEdit";

fn settings(dir: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join(".claude/settings.local.json")).unwrap())
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
        cmd.ends_with("ctk") || cmd.ends_with("ctk.exe"),
        "command was: {cmd}"
    );
    assert_eq!(hook["hooks"][0]["args"], serde_json::json!(["hook"]));
    assert_eq!(hook["hooks"][0]["type"], "command");
    // starter project config written
    assert!(dir.path().join(".cubtoken.toml").exists());
}

#[test]
fn global_init_honors_claude_config_dir() {
    let dir = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .env("CLAUDE_CONFIG_DIR", config.path())
        .args(["init", "--global"])
        .assert()
        .success();

    assert!(config.path().join("settings.json").exists());
    assert!(!dir.path().join(".cubtoken.toml").exists());
}

#[test]
fn init_is_idempotent_and_preserves_existing_settings() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::write(
        dir.path().join(".claude/settings.local.json"),
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
fn init_does_not_remove_an_unrelated_command_with_a_ctk_suffix() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::write(
        dir.path().join(".claude/settings.local.json"),
        r#"{"hooks":{"PostToolUse":[{"matcher":"Other","hooks":[{"type":"command","command":"/usr/local/bin/protectk hook"}]}]}}"#,
    )
    .unwrap();
    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    assert_eq!(
        settings(dir.path())["hooks"]["PostToolUse"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
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
fn init_does_not_overwrite_unreadable_settings() {
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join(".claude/settings.local.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let original = [0xff, 0xfe, 0xfd];
    std::fs::write(&settings, original).unwrap();

    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .failure();
    assert_eq!(std::fs::read(settings).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn init_does_not_follow_a_broken_config_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    symlink("outside.toml", dir.path().join(".cubtoken.toml")).unwrap();

    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    assert!(!dir.path().join("outside.toml").exists());
}

#[cfg(unix)]
#[test]
fn init_rejects_a_symlinked_settings_directory() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), dir.path().join(".claude")).unwrap();

    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .failure();
    assert!(!outside.path().join("settings.json").exists());
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
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
    assert!(
        out.contains("FAIL  hook binary is executable"),
        "got: {out}"
    );
}

/// The current PATH with the binary under test prepended, so a plugin wrapper
/// (and doctor's check that it can resolve `ctk`) behaves like a real install.
fn path_with_ctk() -> std::ffi::OsString {
    let ctk = assert_cmd::cargo::cargo_bin("ctk");
    let bin_dir = ctk
        .parent()
        .expect("cargo_bin path has a parent")
        .to_owned();
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![bin_dir];
    dirs.extend(std::env::split_paths(&existing));
    std::env::join_paths(dirs).expect("PATH entries contain no separator")
}

/// Build a fake Claude Code plugin install under `home`: the
/// `installed_plugins.json` index, the plugin's own `hooks/hooks.json`, and an
/// executable wrapper. Mirrors the real layout — the index points at an
/// `installPath` and the hook command is `${CLAUDE_PLUGIN_ROOT}/bin/<wrapper>`.
fn fake_plugin(home: &std::path::Path, key: &str, wrapper: &str, enabled: bool) {
    let install = home.join("plugins/cache/cubtoken/cubtoken/0.1.2");
    std::fs::create_dir_all(install.join("hooks")).unwrap();
    std::fs::create_dir_all(install.join("bin")).unwrap();
    let bin = install.join("bin").join(wrapper);
    std::fs::write(&bin, "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(
        install.join("hooks/hooks.json"),
        serde_json::json!({"hooks": {"PostToolUse": [{
            "matcher": MATCHER,
            "hooks": [{"type": "command", "command": format!("${{CLAUDE_PLUGIN_ROOT}}/bin/{wrapper}")}]
        }]}})
        .to_string(),
    )
    .unwrap();

    std::fs::create_dir_all(home.join("plugins")).unwrap();
    std::fs::write(
        home.join("plugins/installed_plugins.json"),
        serde_json::json!({"version": 2, "plugins": {
            key: [{"scope": "user", "installPath": install.to_str().unwrap()}]
        }})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        home.join("settings.json"),
        serde_json::json!({"enabledPlugins": {key: enabled}}).to_string(),
    )
    .unwrap();
}

#[test]
fn doctor_detects_an_enabled_plugin_install() {
    // The plugin registers its hook in the plugin's own hooks.json, not in any
    // settings file — doctor reported FAIL on a working plugin install.
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    fake_plugin(
        &home.path().join(".claude"),
        "cubtoken@cubtoken",
        "ctk-hook",
        true,
    );
    let assert = Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .env("HOME", home.path())
        // The wrapper resolves `ctk` at call time, and doctor now checks that
        // it can: put the binary under test on PATH the way a real install is.
        .env("PATH", path_with_ctk())
        .arg("doctor")
        .assert()
        .success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        out.contains("plugin"),
        "doctor should name the plugin: {out}"
    );
}

#[test]
fn doctor_ignores_a_disabled_plugin() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    fake_plugin(
        &home.path().join(".claude"),
        "cubtoken@cubtoken",
        "ctk-hook",
        false,
    );
    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .env("HOME", home.path())
        .arg("doctor")
        .assert()
        .failure();
}

#[test]
fn doctor_ignores_a_plugin_that_is_not_ours() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    fake_plugin(
        &home.path().join(".claude"),
        "somethingelse@market",
        "other-hook",
        true,
    );
    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .env("HOME", home.path())
        .arg("doctor")
        .assert()
        .failure();
}

#[test]
fn doctor_finds_the_install_from_a_subdirectory() {
    // Reporting commands resolve the project root the same way the hook does,
    // so `doctor` from deep in a tree checks the real install instead of
    // reporting a missing hook.
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .env("HOME", home.path())
        .arg("init")
        .assert()
        .success();

    let deep = dir.path().join("crates/ctk-cli/src");
    std::fs::create_dir_all(&deep).unwrap();
    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(&deep)
        .env("HOME", home.path())
        .arg("doctor")
        .assert()
        .success();
    // ...and the probe directory lands at the root, not in the subdirectory.
    assert!(dir.path().join(".cubtoken").is_dir());
    assert!(!deep.join(".cubtoken").exists());
}
