//! Process cost for step 5 of the evaluation spec: startup and end-to-end hook
//! latency, measured against the real `ctk` binary. These are reported
//! separately from governor-only cost because the runtime targets (p95 under
//! 2 ms typical, 5 ms large) apply to the governor, not to spawning a process.
//!
//! Ignored by default. Run it deliberately:
//!
//! ```sh
//! cargo test --release -p ctk-cli --test performance -- --ignored --nocapture
//! ```

use ctk_hook::ledger::{
    CompressionDecision, Confidence, ElidedRange, Ledger, Recovery, RecoveryKind,
};
use std::path::Path;
use std::time::{Duration, Instant};

fn p95(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    let rank = ((samples.len() as f64) * 0.95).ceil() as usize;
    samples[rank.saturating_sub(1).min(samples.len() - 1)]
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

struct Row {
    operation: &'static str,
    p95_ms: f64,
    iterations: usize,
}

fn measure(
    rows: &mut Vec<Row>,
    operation: &'static str,
    iterations: usize,
    mut body: impl FnMut(),
) {
    body();
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        body();
        samples.push(started.elapsed());
    }
    rows.push(Row {
        operation,
        p95_ms: ms(p95(samples)),
        iterations,
    });
}

/// Same mixed-record fixture the governor-only driver uses.
fn seed(dir: &Path, session: &str, records: usize) {
    let mut ledger = Ledger::open(dir, session);
    for index in 0..records {
        let identity = format!("/src/file{}.rs", index % 64);
        match index % 4 {
            0 => ledger.note_decision(CompressionDecision {
                decision_id: format!("d{index}"),
                turn: (index / 8) as u64,
                batch: (index / 2) as u64,
                sequence: index as u64,
                recorded_at_ms: 1_000 + index as u64,
                file_identity: identity,
                content_fingerprint: format!("fp{}", index % 64),
                language: "rust".to_string(),
                strategy: "skeleton".to_string(),
                profile: "default".to_string(),
                tokens_in: 9_000,
                tokens_out: 1_200,
                elided_ranges: vec![ElidedRange {
                    start_line: 20,
                    end_line: 400,
                }],
            }),
            1 => ledger.note_recovery(Recovery {
                path: identity,
                tokens: 600,
                duration_ms: 3,
                decision_id: Some(format!("d{}", index - 1)),
                kind: Some(RecoveryKind::Targeted),
                confidence: Some(Confidence::High),
                turn: Some((index / 8) as u64),
                batch: Some((index / 2) as u64 + 1),
            }),
            2 => ledger.note_edit_identity(&identity),
            _ => ledger.note_saving("Read", Some(&identity), 9_000, 1_200),
        }
    }
}

/// Process startup and end-to-end hook latency, reported separately: these are
/// not governor cost and the runtime targets do not apply to them.
#[test]
#[ignore = "performance measurement; run explicitly with --ignored --nocapture"]
fn hook_process_latency() {
    let exe = assert_cmd::cargo::cargo_bin("ctk");
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let source = format!(
        "pub fn large() {{\n{}\n}}\n",
        "    let value = 42;\n".repeat(1_500)
    );
    let path = dir.join("large.rs");
    std::fs::write(&path, &source).unwrap();
    seed(&dir.join(".cubtoken"), "latency", 1_000);

    let lines = source.lines().count();
    let payload = serde_json::json!({
        "hook_event_name": "PostToolUse", "tool_name": "Read", "session_id": "latency",
        "cwd": dir, "tool_input": {"file_path": &path},
        "tool_response": {"type": "text", "file": {
            "filePath": &path, "content": source, "numLines": lines,
            "startLine": 1, "totalLines": lines
        }}
    })
    .to_string();

    let run = |args: &[&str], stdin: &str| {
        let mut command = std::process::Command::new(&exe);
        command
            .args(args)
            .current_dir(dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut child = command.spawn().unwrap();
        use std::io::Write as _;
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).ok();
        child.wait().unwrap();
    };

    let mut rows: Vec<Row> = Vec::new();
    // `--version` does no work: whatever it costs is pure process startup.
    measure(&mut rows, "startup (ctk --version)", 30, || {
        run(&["--version"], "");
    });
    measure(
        &mut rows,
        "end-to-end (ctk hook), 1,000 ledger records",
        30,
        || {
            run(&["hook"], &payload);
        },
    );

    println!("\n## Process cost (p95), excluded from the governor targets\n");
    println!("| operation | p95 ms | iterations |");
    println!("| --- | --- | --- |");
    for row in &rows {
        println!(
            "| {} | {:.3} | {} |",
            row.operation, row.p95_ms, row.iterations
        );
    }
}
