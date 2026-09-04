use ctk_hook::ledger::Ledger;
use ctk_hook::{run_hook, Config};

#[test]
fn edited_file_is_protected_for_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = Ledger::open(dir.path(), "sess1");
    l.note_edit("/repo/src/main.rs");
    assert!(l.is_protected("/repo/src/main.rs"));
    assert!(!l.is_protected("/repo/src/other.rs"));
    // fresh handle, same session id — persisted
    let l2 = Ledger::open(dir.path(), "sess1");
    assert!(l2.is_protected("/repo/src/main.rs"));
    // different session — not protected
    let l3 = Ledger::open(dir.path(), "sess2");
    assert!(!l3.is_protected("/repo/src/main.rs"));
}

#[test]
fn savings_accumulate() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = Ledger::open(dir.path(), "s");
    l.note_saving("Read", Some("/a.rs"), 10_000, 1_500);
    l.note_saving("Read", Some("/b.rs"), 4_000, 900);
    let s = Ledger::open(dir.path(), "s").totals();
    assert_eq!((s.tokens_in, s.tokens_out), (14_000, 2_400));
    let per_tool = Ledger::open(dir.path(), "s").per_tool();
    assert_eq!(per_tool["Read"].tokens_in, 14_000);
}

fn read_payload(cwd: &str, content_len: usize) -> String {
    // compressible shape: short signature, long body
    let func = "fn x() {\n    let a = 1;\n    let b = a + 2;\n    let c = b * 3;\n    let d = c - 4;\n    let e = d / 5;\n    println!(\"{e}\");\n}\n";
    let content = func.repeat(content_len / func.len());
    serde_json::json!({
        "tool_name": "Read",
        "session_id": "ledger-e2e",
        "cwd": cwd,
        "tool_input": {"file_path": "/repo/src/hot.rs"},
        "tool_response": {"type": "text", "file": {
            "filePath": "/repo/src/hot.rs", "content": content,
            "numLines": 1, "startLine": 1, "totalLines": 1
        }}
    })
    .to_string()
}

#[test]
fn read_of_session_edited_file_is_never_compressed() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap().to_string();
    let cfg = Config::default();

    // big read compresses initially
    assert!(run_hook(&read_payload(&cwd, 60_000), &cfg).is_some());

    // model edits the file
    let edit = serde_json::json!({
        "tool_name": "Edit",
        "session_id": "ledger-e2e",
        "cwd": cwd,
        "tool_input": {"file_path": "/repo/src/hot.rs", "old_string": "a", "new_string": "b"},
        "tool_response": {}
    })
    .to_string();
    assert!(
        run_hook(&edit, &cfg).is_none(),
        "edit itself passes through"
    );

    // same file now protected for the rest of the session
    assert!(run_hook(&read_payload(&cwd, 60_000), &cfg).is_none());
}

#[test]
fn ledger_directory_ignores_itself() {
    let dir = tempfile::tempdir().unwrap();
    let mut ledger = Ledger::open(&dir.path().join(".cubtoken"), "gitignore");
    ledger.note_edit("/repo/src/main.rs");
    let ignore = dir.path().join(".cubtoken/.gitignore");
    assert_eq!(std::fs::read_to_string(ignore).unwrap(), "*\n");
}

#[test]
fn notebook_edit_protects_notebook_path() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap().to_string();
    let cfg = Config::default();
    let path = "/repo/notebooks/analysis.ipynb";

    let edit = serde_json::json!({
        "tool_name": "NotebookEdit",
        "session_id": "notebook-e2e",
        "cwd": cwd,
        "tool_input": {"notebook_path": path, "new_source": "print('updated')"},
        "tool_response": {}
    });
    assert!(run_hook(&edit.to_string(), &cfg).is_none());

    let ledger = Ledger::open(&dir.path().join(".cubtoken"), "notebook-e2e");
    assert!(ledger.is_protected(path));
}

#[test]
fn savings_are_recorded_by_run_hook() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap().to_string();
    run_hook(&read_payload(&cwd, 60_000), &Config::default()).unwrap();
    let l = Ledger::open(&dir.path().join(".cubtoken"), "ledger-e2e");
    let t = l.totals();
    assert!(t.tokens_in > t.tokens_out);
    assert!(t.tokens_out > 0);
}

fn offset_read_payload(cwd: &str) -> String {
    serde_json::json!({
        "tool_name": "Read",
        "session_id": "ledger-e2e",
        "cwd": cwd,
        "tool_input": {"file_path": "/repo/src/hot.rs", "offset": 40, "limit": 20},
        "tool_response": {"type": "text", "file": {
            "filePath": "/repo/src/hot.rs", "content": "fn x() {}\n",
            "numLines": 1, "startLine": 40, "totalLines": 500
        }}
    })
    .to_string()
}

#[test]
fn targeted_read_after_compression_is_recorded_as_a_refetch() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap().to_string();
    let cfg = Config::default();
    let data = dir.path().join(".cubtoken");

    // a targeted read with nothing compressed yet is not a refetch
    assert!(run_hook(&offset_read_payload(&cwd), &cfg).is_none());
    assert_eq!(Ledger::open(&data, "ledger-e2e").refetches(), 0);

    // compress the file, then read back into it
    assert!(run_hook(&read_payload(&cwd, 60_000), &cfg).is_some());
    assert!(run_hook(&offset_read_payload(&cwd), &cfg).is_none());
    assert_eq!(
        Ledger::open(&data, "ledger-e2e").refetches(),
        1,
        "the round trip compression forced must be countable"
    );
}

#[test]
fn refetch_count_survives_a_fresh_ledger_handle() {
    let dir = tempfile::tempdir().unwrap();
    let mut l = Ledger::open(dir.path(), "s");
    l.note_saving("Read", Some("/a.rs"), 100, 10);
    assert!(l.was_compressed("/a.rs"));
    l.note_refetch("/a.rs", 42, 7);
    assert_eq!(Ledger::open(dir.path(), "s").refetches(), 1);
    assert_eq!(Ledger::open(dir.path(), "s").refetch_tokens(), 42);
    assert_eq!(Ledger::open(dir.path(), "s").refetch_duration_ms(), 7);
    assert!(Ledger::open(dir.path(), "s").was_compressed("/a.rs"));
}
