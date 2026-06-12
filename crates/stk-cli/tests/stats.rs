use assert_cmd::Command;

#[test]
fn stats_reports_totals_and_percent() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join(".smalltoke");
    std::fs::create_dir_all(&data).unwrap();
    // two sessions: Read in=14000 out=2400 combined
    std::fs::write(
        data.join("session-a.jsonl"),
        "{\"e\":\"save\",\"tool\":\"Read\",\"in\":10000,\"out\":1500}\n",
    )
    .unwrap();
    std::fs::write(
        data.join("session-b.jsonl"),
        "{\"e\":\"save\",\"tool\":\"Read\",\"in\":4000,\"out\":900}\n{\"e\":\"edit\",\"path\":\"/x.rs\"}\n",
    )
    .unwrap();

    let assert = Command::cargo_bin("stk")
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
}

#[test]
fn stats_with_no_ledger_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let assert = Command::cargo_bin("stk")
        .unwrap()
        .current_dir(dir.path())
        .arg("stats")
        .assert()
        .success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.contains("no savings recorded"), "got: {out}");
}
