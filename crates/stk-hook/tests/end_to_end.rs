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
