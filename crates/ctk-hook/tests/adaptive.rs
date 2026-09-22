use ctk_hook::ledger::{Confidence, Ledger};
use ctk_hook::{run_hook, Config};

fn source() -> String {
    format!(
        "pub fn large() {{\n{}\n}}\n",
        "    let value = 42;\n".repeat(1_500)
    )
}

fn full_read(path: &std::path::Path, cwd: &std::path::Path, session: &str) -> serde_json::Value {
    let content = std::fs::read_to_string(path).unwrap();
    let lines = content.lines().count();
    serde_json::json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Read",
        "tool_use_id": "first-read",
        "session_id": session,
        "cwd": cwd,
        "tool_input": {"file_path": path},
        "tool_response": {"type": "text", "file": {
            "filePath": path, "content": content, "numLines": lines,
            "startLine": 1, "totalLines": lines
        }}
    })
}

fn start_turn(cwd: &std::path::Path, session: &str) -> String {
    serde_json::json!({"hook_event_name": "UserPromptSubmit", "cwd": cwd, "session_id": session})
        .to_string()
}

#[test]
fn same_batch_read_is_not_high_confidence_but_later_batch_overlap_is() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    let cfg = Config::default();

    let same_batch = "same-batch";
    run_hook(&start_turn(temp.path(), same_batch), &cfg);
    assert!(run_hook(&full_read(&path, temp.path(), same_batch).to_string(), &cfg).is_some());
    let ledger = Ledger::open(&temp.path().join(".cubtoken"), same_batch);
    let decision = ledger.decisions().last().unwrap();
    let range = decision.elided_ranges.first().unwrap();
    let targeted = serde_json::json!({
        "hook_event_name": "PostToolUse", "tool_name": "Read", "session_id": same_batch,
        "cwd": temp.path(), "tool_input": {"file_path": path, "offset": range.start_line, "limit": 1},
        "tool_response": {"file": {"filePath": path, "content": "    let value = 42;\n", "startLine": range.start_line, "numLines": 1}}
    });
    assert!(run_hook(&targeted.to_string(), &cfg).is_none());
    let ledger = Ledger::open(&temp.path().join(".cubtoken"), same_batch);
    assert!(!ledger
        .recoveries()
        .iter()
        .any(|recovery| recovery.confidence == Some(Confidence::High)));

    let later_batch = "later-batch";
    run_hook(&start_turn(temp.path(), later_batch), &cfg);
    assert!(run_hook(
        &full_read(&path, temp.path(), later_batch).to_string(),
        &cfg
    )
    .is_some());
    run_hook(&serde_json::json!({"hook_event_name": "PostToolBatch", "cwd": temp.path(), "session_id": later_batch}).to_string(), &cfg);
    assert!(run_hook(&targeted.to_string().replace(same_batch, later_batch), &cfg).is_none());
    let ledger = Ledger::open(&temp.path().join(".cubtoken"), later_batch);
    assert_eq!(
        ledger.recoveries().last().unwrap().confidence,
        Some(Confidence::High)
    );
}

#[test]
fn changed_content_and_intervening_edits_cannot_train_policy() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    let session = "changed";
    let cfg = Config::default();
    run_hook(&start_turn(temp.path(), session), &cfg);
    assert!(run_hook(&full_read(&path, temp.path(), session).to_string(), &cfg).is_some());
    let decision = Ledger::open(&temp.path().join(".cubtoken"), session)
        .decisions()
        .last()
        .cloned()
        .unwrap();
    run_hook(&serde_json::json!({"hook_event_name": "PostToolBatch", "cwd": temp.path(), "session_id": session}).to_string(), &cfg);
    std::fs::write(&path, format!("{}// changed\n", source())).unwrap();
    let targeted = serde_json::json!({
        "hook_event_name": "PostToolUse", "tool_name": "Read", "session_id": session,
        "cwd": temp.path(), "tool_input": {"file_path": path, "offset": decision.elided_ranges[0].start_line, "limit": 1},
        "tool_response": {"file": {"filePath": path, "content": "    let value = 42;\n", "startLine": decision.elided_ranges[0].start_line, "numLines": 1}}
    });
    run_hook(&targeted.to_string(), &cfg);
    let ledger = Ledger::open(&temp.path().join(".cubtoken"), session);
    assert_eq!(
        ledger.recoveries().last().unwrap().confidence,
        Some(Confidence::Low)
    );

    let edit = serde_json::json!({
        "hook_event_name": "PostToolUse", "tool_name": "Edit", "session_id": session,
        "cwd": temp.path(), "tool_input": {"file_path": path}, "tool_response": {}
    });
    run_hook(&edit.to_string(), &cfg);
    run_hook(&targeted.to_string(), &cfg);
    let ledger = Ledger::open(&temp.path().join(".cubtoken"), session);
    assert!(!ledger
        .recoveries()
        .iter()
        .any(|recovery| recovery.confidence == Some(Confidence::High)));
}

#[test]
fn concurrent_and_legacy_ledgers_remain_readable() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join(".cubtoken");
    let mut workers = Vec::new();
    for index in 0..16 {
        let data = data.clone();
        workers.push(std::thread::spawn(move || {
            Ledger::open(&data, "parallel").note_saving("Read", None, 100 + index, 10);
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    let content = std::fs::read_to_string(data.join("session-parallel.jsonl")).unwrap();
    assert!(!content.is_empty());
    for line in content.lines() {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }

    std::fs::write(
        data.join("session-legacy.jsonl"),
        "{\"e\":\"save\",\"tool\":\"Read\",\"in\":100,\"out\":20,\"path\":\"/old.rs\"}\n{\"e\":\"edit\",\"path\":\"/old.rs\"}\n{\"e\":\"refetch\",\"path\":\"/old.rs\",\"tokens\":7,\"duration_ms\":3}\n",
    )
    .unwrap();
    let legacy = Ledger::open(&data, "legacy");
    assert!(legacy.is_protected("/old.rs"));
    assert_eq!(legacy.totals().tokens_in, 100);
    assert_eq!(legacy.refetch_tokens(), 7);
}

#[test]
fn lock_contention_makes_reads_pass_through() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    let data = temp.path().join(".cubtoken");
    let _lock_owner = Ledger::open(&data, "locked");

    assert!(run_hook(
        &full_read(&path, temp.path(), "locked").to_string(),
        &Config::default()
    )
    .is_none());
}

#[test]
fn full_repeat_escape_is_recorded_once_per_decision() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    let session = "full-repeat";
    let cfg = Config::default();
    run_hook(&start_turn(temp.path(), session), &cfg);
    assert!(run_hook(&full_read(&path, temp.path(), session).to_string(), &cfg).is_some());
    run_hook(&serde_json::json!({"hook_event_name": "PostToolBatch", "cwd": temp.path(), "session_id": session}).to_string(), &cfg);

    let mut repeat = full_read(&path, temp.path(), session);
    repeat["tool_use_id"] = serde_json::json!("repeat");
    assert!(run_hook(&repeat.to_string(), &cfg).is_none());
    assert!(run_hook(&repeat.to_string(), &cfg).is_some());

    let ledger = Ledger::open(&temp.path().join(".cubtoken"), session);
    assert_eq!(
        ledger
            .recoveries()
            .iter()
            .filter(|recovery| recovery.kind == Some(ctk_hook::ledger::RecoveryKind::FullRepeat))
            .count(),
        1
    );
    assert_eq!(ledger.refetches(), 0);
}

#[test]
fn safe_mode_only_reduces_compression() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    let static_cfg = Config::load_from(
        None,
        Some("[read]\nthreshold_tokens = 10\nnever_compress = []"),
    );
    assert!(run_hook(
        &full_read(&path, temp.path(), "static").to_string(),
        &static_cfg
    )
    .is_some());

    std::fs::create_dir_all(temp.path().join(".cubtoken")).unwrap();
    std::fs::write(
        temp.path().join(".cubtoken/adaptive-v1.json"),
        r#"{"schema":1,"policy_version":1,"reset_at_ms":0,"buckets":{"rust|8k-16k|skeleton":{"outcomes":[],"recommendation":"disabled"}}}"#,
    )
    .unwrap();
    let safe_cfg = Config::load_from(
        None,
        Some("[read]\nthreshold_tokens = 10\nnever_compress = []\n[adaptive]\nmode = \"safe\""),
    );
    assert!(run_hook(
        &full_read(&path, temp.path(), "safe").to_string(),
        &safe_cfg
    )
    .is_none());
}
