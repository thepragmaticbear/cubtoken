//! Runs the OpenCode adapter's self-check against the freshly built `ctk`, so
//! `cargo test` (and therefore CI) covers the JS shim too. Skips when node
//! isn't available rather than failing — the adapter is optional.

use std::path::Path;
use std::process::Command;

#[test]
fn opencode_adapter_self_check() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = root.join("packages/opencode/test.js");

    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipping: node not available");
        return;
    }

    let output = Command::new("node")
        .arg(&script)
        .env("CUBTOKEN_CTK_BIN", assert_cmd::cargo::cargo_bin("ctk"))
        .output()
        .expect("failed to run node");

    assert!(
        output.status.success(),
        "opencode self-check failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
