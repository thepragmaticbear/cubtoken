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
