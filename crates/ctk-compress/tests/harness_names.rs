//! Invariant 2: the compressed view names the exact tool call that retrieves
//! the elided lines. That call differs per harness, so this pins both spellings.
//!
//! Its own integration test binary on purpose: `CUBTOKEN_HARNESS` is
//! process-global, and setting it inside the unit-test binary would race the
//! other tests that assert on header text.

use ctk_compress::{read::compress_read, Config};

fn fat_rust_file() -> String {
    let mut src = String::new();
    for i in 0..40 {
        src.push_str(&format!("pub fn operation_{i}(id: u64) -> usize {{\n"));
        for j in 0..20 {
            src.push_str(&format!("    let step_{j} = id + {j};\n"));
        }
        src.push_str("    0\n}\n\n");
    }
    src
}

fn compressed_header(file_path: &str) -> String {
    let content = fat_rust_file();
    let lines = content.lines().count();
    let cfg = Config::load_from(
        None,
        Some("[read]\nthreshold_tokens = 10\nnever_compress = []\n[stats]\nledger = false"),
    );
    let outcome = compress_read(
        &serde_json::json!({ "file_path": file_path }),
        &serde_json::json!({"type": "text", "file": {
            "filePath": file_path, "content": content,
            "numLines": lines, "startLine": 1, "totalLines": lines
        }}),
        &cfg,
    )
    .expect("fat file should compress");
    outcome.updated_response["file"]["content"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn names_the_hosts_own_read_call() {
    // Default (Claude Code): `Read(file_path=…)`.
    let header = compressed_header("/tmp/a.rs");
    assert!(
        header.contains("run Read(file_path=/tmp/a.rs, offset="),
        "default header was: {}",
        &header[..header.len().min(400)]
    );

    // OpenCode spells the same tool `read(filePath=…)`; pointing the model at
    // `Read(file_path=…)` there would name a call it cannot make.
    std::env::set_var("CUBTOKEN_HARNESS", "opencode");
    let header = compressed_header("/tmp/a.rs");
    assert!(
        header.contains("run read(filePath=/tmp/a.rs, offset="),
        "opencode header was: {}",
        &header[..header.len().min(400)]
    );
    assert!(!header.contains("Read(file_path="));

    // An unknown harness falls back to the Claude Code spelling.
    std::env::set_var("CUBTOKEN_HARNESS", "something-else");
    assert!(compressed_header("/tmp/a.rs").contains("run Read(file_path="));
    std::env::remove_var("CUBTOKEN_HARNESS");
}
