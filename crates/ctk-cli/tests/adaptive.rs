use assert_cmd::Command;

fn ctk(dir: &std::path::Path) -> Command {
    let mut command = Command::cargo_bin("ctk").unwrap();
    command.current_dir(dir).env("HOME", dir);
    command
}

#[test]
fn diagnostics_reject_malformed_config() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".cubtoken.toml"), "not = [valid").unwrap();
    ctk(dir.path())
        .args(["adaptive", "status"])
        .assert()
        .failure();
    ctk(dir.path())
        .args(["output", "status"])
        .assert()
        .failure();
}

#[test]
fn explain_and_reset_failures_return_nonzero() {
    let dir = tempfile::tempdir().unwrap();
    ctk(dir.path())
        .args(["adaptive", "explain", "missing.rs"])
        .assert()
        .failure();

    std::fs::write(dir.path().join(".cubtoken"), "not a directory").unwrap();
    ctk(dir.path())
        .args(["adaptive", "reset"])
        .assert()
        .failure();
}

/// An evaluation arm has to prove which policy it actually ran. `mode` alone
/// cannot: `mode = "safe"` with `[stats] ledger = false` records no decisions,
/// so no bucket reaches the eight-observation minimum and the arm silently
/// behaves as static. `status --json` is the artifact a run record cites, so it
/// must carry the inputs that decide whether adaptation was even possible.
#[test]
fn status_json_reports_whether_adaptation_is_possible() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".cubtoken.toml"),
        "[read]\nthreshold_tokens = 1234\n[adaptive]\nmode = \"safe\"\n[stats]\nledger = false\n",
    )
    .unwrap();
    let out = ctk(dir.path())
        .args(["adaptive", "status", "--json"])
        .assert()
        .success();
    let json: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("status --json is not JSON");

    assert_eq!(json["mode"], "safe");
    assert_eq!(
        json["stats_ledger"], false,
        "status must report whether the ledger the policy learns from is enabled"
    );
    assert_eq!(
        json["read_threshold_tokens"], 1234,
        "status must report the configured threshold each recommendation multiplies"
    );
    assert_eq!(
        json["learning_enabled"], false,
        "safe mode without the ledger cannot learn and must not claim it can"
    );

    // With statistics on, the same mode can actually act.
    std::fs::write(
        dir.path().join(".cubtoken.toml"),
        "[adaptive]\nmode = \"safe\"\n[stats]\nledger = true\n",
    )
    .unwrap();
    let out = ctk(dir.path())
        .args(["adaptive", "status", "--json"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["learning_enabled"], true);
}

/// The same trap, surfaced to a human running the plain command.
#[test]
fn status_warns_when_statistics_disable_adaptation() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".cubtoken.toml"),
        "[adaptive]\nmode = \"safe\"\n[stats]\nledger = false\n",
    )
    .unwrap();
    let out = ctk(dir.path())
        .args(["adaptive", "status"])
        .assert()
        .success();
    let printed = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(
        printed.contains("stats.ledger"),
        "safe mode with statistics off must say so; printed: {printed}"
    );

    // No warning when the configuration is coherent.
    std::fs::write(
        dir.path().join(".cubtoken.toml"),
        "[adaptive]\nmode = \"off\"\n[stats]\nledger = false\n",
    )
    .unwrap();
    let out = ctk(dir.path())
        .args(["adaptive", "status"])
        .assert()
        .success();
    let printed = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(
        !printed.contains("stats.ledger"),
        "mode off needs no warning; printed: {printed}"
    );
}

/// `explain` is the diagnostic the runbook tells operators to use while warming
/// a checkpoint, which happens in `observe` mode. It reported the bucket's
/// recommendation as the decision regardless of mode — but the hook only
/// applies a recommendation in `safe` mode, so `explain` announced "disabled"
/// on a file the very next Read would compress.
#[test]
fn explain_reports_the_threshold_the_hook_will_actually_use() {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        "pub fn large() {{\n{}\n}}\n",
        "    let value = 42;\n".repeat(1_500)
    );
    let path = dir.path().join("large.rs");
    std::fs::write(&path, source).unwrap();
    std::fs::create_dir_all(dir.path().join(".cubtoken")).unwrap();
    std::fs::write(
        dir.path().join(".cubtoken/adaptive-v1.json"),
        r#"{"schema":1,"policy_version":1,"reset_at_ms":0,"buckets":{"rust|8k-16k|skeleton":{"outcomes":[],"recommendation":"disabled"}}}"#,
    )
    .unwrap();

    let explain = |mode: &str| {
        std::fs::write(
            dir.path().join(".cubtoken.toml"),
            format!("[adaptive]\nmode = \"{mode}\"\n"),
        )
        .unwrap();
        let out = ctk(dir.path())
            .args(["adaptive", "explain", path.to_str().unwrap()])
            .assert()
            .success();
        String::from_utf8_lossy(&out.get_output().stdout).into_owned()
    };

    // In `safe` the recommendation is applied, so reporting it is correct.
    let safe = explain("safe");
    assert!(
        safe.contains("disabled"),
        "safe mode applies the recommendation; explain should say so: {safe}"
    );

    // In `observe` and `off` it is not applied: the hook uses the configured
    // threshold and will compress this file.
    for mode in ["observe", "off"] {
        let printed = explain(mode);
        assert!(
            !printed.contains("effective threshold: 18446744073709551615"),
            "{mode} mode does not apply the recommendation, so the effective \
             threshold must not be the disabled sentinel: {printed}"
        );
        assert!(
            printed.contains("not applied") || printed.contains(&format!("mode: {mode}")),
            "{mode} mode must say the recommendation is not in force: {printed}"
        );
    }
}
