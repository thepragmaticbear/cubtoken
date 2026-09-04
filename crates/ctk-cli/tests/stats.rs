use assert_cmd::Command;

#[test]
fn stats_reports_totals_and_percent() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join(".cubtoken");
    std::fs::create_dir_all(&data).unwrap();
    // two sessions: Read in=14000 out=2400 combined
    std::fs::write(
        data.join("session-a.jsonl"),
        "{\"e\":\"save\",\"tool\":\"Read\",\"in\":10000,\"out\":1500}\n",
    )
    .unwrap();
    std::fs::write(
        data.join("session-b.jsonl"),
        "{\"e\":\"save\",\"tool\":\"Read\",\"in\":4000,\"out\":900}\n{\"e\":\"edit\",\"path\":\"/x.rs\"}\n{\"e\":\"refetch\",\"path\":\"/x.rs\",\"tokens\":600,\"duration_ms\":12}\n",
    )
    .unwrap();

    let assert = Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .arg("stats")
        .assert()
        .success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.contains("Read"), "per-tool row: {out}");
    assert!(out.contains("14000"), "tokens in: {out}");
    assert!(out.contains("2400"), "tokens out: {out}");
    assert!(out.contains("83%"), "savings percent: {out}");
    assert!(out.contains("11000 tokens"), "net savings: {out}");
    assert!(out.contains("600 tokens, 12ms"), "refetch costs: {out}");
}

#[test]
fn stats_with_no_ledger_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let assert = Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .arg("stats")
        .assert()
        .success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.contains("no savings recorded"), "got: {out}");
}

#[test]
fn stats_handles_untrusted_large_counts() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join(".cubtoken");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
        data.join("session-large.jsonl"),
        format!(
            "{{\"e\":\"save\",\"tool\":\"Read\",\"in\":{},\"out\":0}}\n{{\"e\":\"save\",\"tool\":\"Read\",\"in\":1,\"out\":0}}\n",
            usize::MAX
        ),
    )
    .unwrap();

    Command::cargo_bin("ctk")
        .unwrap()
        .current_dir(dir.path())
        .arg("stats")
        .assert()
        .success();
}
