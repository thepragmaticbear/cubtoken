//! Reproductions for defects found by adversarial review of the adaptive
//! governor. Each test fails against the governor as first written and passes
//! after the fix in the owning module.

use ctk_hook::ledger::Ledger;
use ctk_hook::{run_hook, Config};

fn source() -> String {
    format!(
        "pub fn large() {{\n{}\n}}\n",
        "    let value = 42;\n".repeat(1_500)
    )
}

fn full_read(path: &std::path::Path, cwd: &std::path::Path, session: &str) -> String {
    let content = std::fs::read_to_string(path).unwrap();
    let lines = content.lines().count();
    serde_json::json!({
        "hook_event_name": "PostToolUse", "tool_name": "Read",
        "tool_use_id": "r1", "session_id": session, "cwd": cwd,
        "tool_input": {"file_path": path},
        "tool_response": {"type": "text", "file": {
            "filePath": path, "content": content, "numLines": lines,
            "startLine": 1, "totalLines": lines
        }}
    })
    .to_string()
}

fn edit(path: &std::path::Path, cwd: &std::path::Path, session: &str) -> String {
    serde_json::json!({
        "hook_event_name": "PostToolUse", "tool_name": "Edit",
        "session_id": session, "cwd": cwd,
        "tool_input": {"file_path": path}, "tool_response": {}
    })
    .to_string()
}

/// Edit protection is a correctness guarantee, not a statistic: a file the
/// model edited this session must never be compressed again. It was recorded
/// through the advisory lock, so an Edit that arrived while the lock was held —
/// or while a stale lockfile sat in the directory after a killed process —
/// silently dropped the record and left the file compressible forever after.
#[test]
fn edit_protection_survives_an_unavailable_ledger_lock() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    let cfg = Config::load_from(
        None,
        Some("[read]\nthreshold_tokens = 10\nnever_compress = []"),
    );

    // Hold the lock the way a concurrent hook process or a killed one would.
    let data = temp.path().join(".cubtoken");
    std::fs::create_dir_all(&data).unwrap();
    let lock = data.join("session-locked.jsonl.lock");
    std::fs::write(&lock, "").unwrap();

    assert!(run_hook(&edit(&path, temp.path(), "locked"), &cfg).is_none());

    // The contending process finishes and releases the lock.
    std::fs::remove_file(&lock).unwrap();

    assert!(
        run_hook(&full_read(&path, temp.path(), "locked"), &cfg).is_none(),
        "a file edited this session must never be compressed, even when the \
         edit arrived while the ledger lock was unavailable"
    );
    assert!(
        Ledger::open(&data, "locked").is_protected(&ctk_hook::ledger::normalize_file_identity(
            temp.path(),
            path.to_str().unwrap()
        )),
        "the edit must be durable: a later process replays it from disk"
    );
}

/// The data directory is checked for being a symlink; the session file inside
/// it was not. A `session-*.jsonl` symlink made the ledger append its records
/// into whatever the link pointed at.
#[cfg(unix)]
#[test]
fn ledger_does_not_append_through_a_symlinked_session_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    let data = temp.path().join(".cubtoken");
    std::fs::create_dir_all(&data).unwrap();

    let victim = temp.path().join("unrelated.txt");
    std::fs::write(&victim, "keep this file unchanged\n").unwrap();
    std::os::unix::fs::symlink(&victim, data.join("session-symlink.jsonl")).unwrap();

    let cfg = Config::default();
    run_hook(&edit(&path, temp.path(), "symlink"), &cfg);

    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        "keep this file unchanged\n",
        "the ledger must not write through a symlinked session file"
    );
}

/// `refresh` loaded the epoch, did its work, then wrote back the epoch it had
/// read. An `adaptive reset` landing in that window was silently undone: the
/// stale write rewound the durable epoch and resurrected the observations the
/// reset was supposed to drop.
///
/// The window is real but short, so it is held open deterministically rather
/// than by timing: `Ledger::load_all` reads every `session-*.jsonl` in the
/// directory, and a FIFO with no writer blocks until one appears.
#[cfg(unix)]
#[test]
fn a_concurrent_reset_is_not_rewound_by_an_in_flight_refresh() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join(".cubtoken");
    std::fs::create_dir_all(&data).unwrap();

    let initial = ctk_hook::adaptive_state::reset(&data, 1_000)
        .unwrap()
        .reset_at_ms;
    assert_eq!(initial, 1_000);

    let fifo = data.join("session-paused.jsonl");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo");
    assert!(made.success(), "could not create the FIFO this test needs");

    // This refresh loads epoch 1_000, then blocks inside `load_all`.
    let stalled = {
        let data = data.clone();
        std::thread::spawn(move || {
            ctk_hook::adaptive_state::refresh(&data, 0);
        })
    };

    // Give the refresh time to reach the FIFO and block on it.
    std::thread::sleep(std::time::Duration::from_millis(200));

    // A reset lands while that refresh is still in flight.
    ctk_hook::adaptive_state::reset(&data, 9_000).unwrap();
    assert_eq!(ctk_hook::adaptive_state::load(&data, 0).reset_at_ms, 9_000);

    // Release the refresh: opening for write and closing gives it EOF.
    std::fs::OpenOptions::new().write(true).open(&fifo).unwrap();
    stalled.join().unwrap();

    assert_eq!(
        ctk_hook::adaptive_state::load(&data, 0).reset_at_ms,
        9_000,
        "an in-flight refresh must not rewind an epoch a reset already advanced"
    );
}
