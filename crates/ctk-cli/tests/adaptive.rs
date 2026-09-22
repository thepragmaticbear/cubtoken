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
