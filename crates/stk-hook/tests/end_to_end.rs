use stk_hook::{run_hook, Config};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../../tests/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn large_read_is_compressed_with_escape_hatch() {
    let out = run_hook(&fixture("read_large"), &Config::default()).expect("should compress");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["hookSpecificOutput"]["hookEventName"], "PostToolUse",
        "must label the hook event"
    );
    let content = v["hookSpecificOutput"]["updatedToolOutput"]["file"]["content"]
        .as_str()
        .expect("compressed content keeps the Read response shape");
    assert!(content.contains("smalltoke"), "banner present");
    assert!(content.contains("offset"), "escape hatch present");
    assert!(
        content.contains("pub fn operation_0"),
        "skeleton shows signatures"
    );
    let original: serde_json::Value = serde_json::from_str(&fixture("read_large")).unwrap();
    let orig_content = original["tool_response"]["file"]["content"]
        .as_str()
        .unwrap();
    assert!(
        content.len() < orig_content.len() / 2,
        "compressed {} vs original {}",
        content.len(),
        orig_content.len()
    );
}

#[test]
fn offset_read_passes_through() {
    assert!(run_hook(&fixture("read_offset"), &Config::default()).is_none());
}

#[test]
fn small_read_passes_through() {
    let mut v: serde_json::Value = serde_json::from_str(&fixture("read_large")).unwrap();
    let content = v["tool_response"]["file"]["content"].as_str().unwrap();
    let truncated: String = content.chars().take(200).collect();
    v["tool_response"]["file"]["content"] = serde_json::Value::String(truncated);
    assert!(run_hook(&v.to_string(), &Config::default()).is_none());
}

#[test]
fn grep_with_many_matches_is_folded() {
    let out = run_hook(&fixture("grep_many"), &Config::default()).expect("should fold");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let content = v["hookSpecificOutput"]["updatedToolOutput"]["content"]
        .as_str()
        .unwrap();
    assert!(content.contains("more matches in"), "fold note: {content}");
    assert!(content.contains("rerun Grep with path="), "escape hatch");
    let original: serde_json::Value = serde_json::from_str(&fixture("grep_many")).unwrap();
    let orig = original["tool_response"]["content"].as_str().unwrap();
    assert!(content.len() < orig.len() * 7 / 10);
}

#[test]
fn grep_files_mode_passes_through() {
    let mut v: serde_json::Value = serde_json::from_str(&fixture("grep_many")).unwrap();
    v["tool_response"]["mode"] = serde_json::json!("files_with_matches");
    assert!(run_hook(&v.to_string(), &Config::default()).is_none());
}

#[test]
fn glob_with_many_paths_is_folded() {
    let mut v: serde_json::Value = serde_json::from_str(&fixture("glob_many")).unwrap();
    let many: Vec<String> = (0..300)
        .map(|i| format!("src/components/C{i}.tsx"))
        .collect();
    v["tool_response"]["filenames"] = serde_json::json!(many);
    v["tool_response"]["numFiles"] = serde_json::json!(300);
    let out = run_hook(&v.to_string(), &Config::default()).expect("should fold");
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    let folded = parsed["hookSpecificOutput"]["updatedToolOutput"]["filenames"][0]
        .as_str()
        .unwrap();
    assert!(folded.contains("300 paths folded"), "{folded}");
    assert!(folded.contains("src/components/ (300 files"), "{folded}");
}

#[test]
fn glob_with_few_paths_passes_through() {
    // recorded fixture has 31 files, under the 50-path default
    assert!(run_hook(&fixture("glob_many"), &Config::default()).is_none());
}

#[test]
fn bash_disabled_by_default() {
    let mut v: serde_json::Value = serde_json::from_str(&fixture("bash_simple")).unwrap();
    v["tool_response"]["stdout"] =
        serde_json::json!("\u{1b}[32mok\u{1b}[0m\n".repeat(200) + "done\n");
    assert!(run_hook(&v.to_string(), &Config::default()).is_none());
}

#[test]
fn bash_enabled_strips_noise_on_success_only() {
    let cfg = Config::load_from(None, Some("[bash]\nenabled = true"));
    let mut v: serde_json::Value = serde_json::from_str(&fixture("bash_simple")).unwrap();
    v["tool_response"]["stdout"] =
        serde_json::json!("\u{1b}[32mok\u{1b}[0m\n".repeat(200) + "done\n");

    let out = run_hook(&v.to_string(), &cfg).expect("should strip");
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    let stdout = parsed["hookSpecificOutput"]["updatedToolOutput"]["stdout"]
        .as_str()
        .unwrap();
    assert!(!stdout.contains('\u{1b}'));
    assert!(stdout.contains("done"));

    // same payload but failing (stderr non-empty): untouched
    v["tool_response"]["stderr"] = serde_json::json!("assertion failed: left == right");
    assert!(run_hook(&v.to_string(), &cfg).is_none());
}

#[test]
fn excluded_path_passes_through() {
    let mut v: serde_json::Value = serde_json::from_str(&fixture("read_large")).unwrap();
    v["tool_input"]["file_path"] = serde_json::json!("/private/tmp/stk-capture/NOTES.md");
    v["tool_response"]["file"]["filePath"] = serde_json::json!("/private/tmp/stk-capture/NOTES.md");
    assert!(run_hook(&v.to_string(), &Config::default()).is_none());
}

#[test]
fn read_disabled_passes_through() {
    let cfg = Config::load_from(None, Some("[read]\nenabled = false"));
    assert!(run_hook(&fixture("read_large"), &cfg).is_none());
}
