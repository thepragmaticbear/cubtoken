use assert_cmd::Command;

#[test]
fn record_appends_stdin_json_to_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("payloads.jsonl");
    Command::cargo_bin("stk")
        .unwrap()
        .args(["record", out.to_str().unwrap()])
        .write_stdin(r#"{"tool_name":"Read"}"#)
        .assert()
        .success()
        .stdout("");
    let content = std::fs::read_to_string(&out).unwrap();
    assert_eq!(content.trim(), r#"{"tool_name":"Read"}"#);
}

#[test]
fn record_appends_not_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("payloads.jsonl");
    for payload in [r#"{"a":1}"#, r#"{"b":2}"#] {
        Command::cargo_bin("stk")
            .unwrap()
            .args(["record", out.to_str().unwrap()])
            .write_stdin(payload)
            .assert()
            .success();
    }
    let content = std::fs::read_to_string(&out).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines, vec![r#"{"a":1}"#, r#"{"b":2}"#]);
}
