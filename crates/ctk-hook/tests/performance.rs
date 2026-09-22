//! Governor performance driver for step 5 of the evaluation spec.
//!
//! Ignored by default — it is a measurement, not an assertion about the
//! machine it happens to run on. Run it deliberately:
//!
//! ```sh
//! cargo test --release -p ctk-hook --test performance -- --ignored --nocapture
//! ```
//!
//! It reports p95 rather than a single timing, and separates the governor-only
//! cost from process startup and end-to-end hook latency, because the runtime
//! targets (p95 under 2 ms for typical sessions, under 5 ms for large ones)
//! apply to the governor and not to spawning a binary.

use ctk_hook::attribution;
use ctk_hook::ledger::{
    CompressionDecision, Confidence, ElidedRange, Ledger, Recovery, RecoveryKind,
};
use std::path::Path;
use std::time::{Duration, Instant};

/// Fixture sizes. The spec asks which represent "typical" and which "large";
/// this driver states it rather than leaving the reader to guess.
const SIZES: &[(usize, &str)] = &[
    (100, "typical"),
    (1_000, "typical"),
    (5_000, "large"),
    (10_000, "large"),
];

/// The fixture the safe-mode preview parses: one signature over a fat body,
/// the shape a skeleton actually compresses.
fn source() -> String {
    format!(
        "pub fn large() {{\n{}\n}}\n",
        "    let value = 42;\n".repeat(1_500)
    )
}

/// Nearest-rank percentile. Note the sample-count requirement: with n = 10,
/// `ceil(10 * 0.95) = 10`, so "p95" would be the maximum sample and a single
/// scheduling hiccup becomes the reported figure. Callers use n >= 100 so the
/// 95th percentile is an actual percentile.
fn percentile(samples: &[Duration], fraction: f64) -> Duration {
    let rank = ((samples.len() as f64) * fraction).ceil() as usize;
    samples[rank.saturating_sub(1).min(samples.len() - 1)]
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

/// Write `records` ledger records: a realistic mix of decisions, recoveries,
/// edits, and savings rather than one record type repeated.
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

struct Row {
    records: usize,
    category: &'static str,
    operation: &'static str,
    median_ms: f64,
    p95_ms: f64,
    min_ms: f64,
    iterations: usize,
}

fn measure(
    rows: &mut Vec<Row>,
    records: usize,
    category: &'static str,
    operation: &'static str,
    iterations: usize,
    mut body: impl FnMut(),
) {
    // One warm-up pass so the first sample does not carry page-cache cost.
    body();
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        body();
        samples.push(started.elapsed());
    }
    samples.sort();
    // Median and minimum sit beside p95: on a shared machine the tail carries
    // scheduler noise, and a wide median-to-p95 gap says the measurement needs
    // attention before the code does.
    rows.push(Row {
        records,
        category,
        operation,
        median_ms: ms(percentile(&samples, 0.50)),
        p95_ms: ms(percentile(&samples, 0.95)),
        min_ms: ms(samples[0]),
        iterations,
    });
}

#[test]
#[ignore = "performance measurement; run explicitly with --ignored --nocapture"]
fn governor_p95_by_ledger_size() {
    let mut rows: Vec<Row> = Vec::new();

    for &(records, category) in SIZES {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".cubtoken");
        seed(&dir, "perf", records);
        // A second session file, so `load_all`/rebuild has more than one to walk.
        seed(&dir, "perf-b", records / 2);

        // >= 100 everywhere: see `percentile`. Below that, p95 is the maximum.
        let iterations = if records >= 5_000 { 100 } else { 200 };

        measure(
            &mut rows,
            records,
            category,
            "replay (Ledger::open)",
            iterations,
            || {
                let ledger = Ledger::open(&dir, "perf");
                std::hint::black_box(ledger.decisions().len());
            },
        );

        // The per-Read state snapshot safe mode loads: cheap.
        //
        // The row after it is NOT on the Read path any more. `preview_content`
        // is now reached only by `ctk adaptive explain`; the hook parses once,
        // inside `ReadCandidate::parse`, and hands that parse to the
        // compressor. It is kept because `explain` still pays it.
        measure(
            &mut rows,
            records,
            category,
            "policy load (adaptive load)",
            iterations,
            || {
                std::hint::black_box(ctk_hook::adaptive_state::load(&dir, 500).buckets.len());
            },
        );
        let preview_source = source();
        measure(
            &mut rows,
            records,
            category,
            "explain preview (tree-sitter)",
            iterations,
            || {
                std::hint::black_box(
                    ctk_compress::read::preview_content("large.rs", &preview_source).tokens_in,
                );
            },
        );

        let probe = Ledger::open(&dir, "perf");
        measure(
            &mut rows,
            records,
            category,
            "lookup (is_protected)",
            iterations * 10,
            || {
                std::hint::black_box(probe.is_protected("/src/file7.rs"));
            },
        );

        let decision = probe.decisions().last().cloned().unwrap();
        let returned = ElidedRange {
            start_line: 30,
            end_line: 31,
        };
        measure(
            &mut rows,
            records,
            category,
            "attribution (classify)",
            iterations * 10,
            || {
                std::hint::black_box(attribution::classify_targeted(
                    &decision,
                    decision.turn + 1,
                    decision.batch + 1,
                    true,
                    false,
                    &returned,
                ));
            },
        );
        drop(probe);

        measure(
            &mut rows,
            records,
            category,
            "append (note_saving)",
            iterations,
            || {
                let mut ledger = Ledger::open(&dir, "perf-append");
                ledger.note_saving("Read", Some("/src/file1.rs"), 9_000, 1_200);
            },
        );

        let rebuild_iterations = if records >= 5_000 { 100 } else { 200 };
        measure(
            &mut rows,
            records,
            category,
            "rebuild (adaptive refresh)",
            rebuild_iterations,
            || {
                std::hint::black_box(ctk_hook::adaptive_state::refresh(&dir, 500).buckets.len());
            },
        );
    }

    // The number that actually matters for safe mode: one whole compressed
    // Read through `run_hook`, static versus safe. Safe pays the threshold
    // preview on top, and that preview parses the same content the compressor
    // is about to parse again.
    {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("large.rs");
        std::fs::write(&path, source()).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let lines = content.lines().count();
        let payload = serde_json::json!({
            "hook_event_name": "PostToolUse", "tool_name": "Read",
            "session_id": "endtoend", "cwd": temp.path(),
            "tool_input": {"file_path": &path},
            "tool_response": {"type": "text", "file": {
                "filePath": &path, "content": content, "numLines": lines,
                "startLine": 1, "totalLines": lines
            }}
        })
        .to_string();
        // A big file that still sits UNDER the threshold: it can never
        // compress, so safe mode should cost what static costs. The parse the
        // old code ran scaled with file size, so this — not a three-line file
        // — is where skipping it matters.
        let under = "[read]\nthreshold_tokens = 1000000\nnever_compress = []";
        for (label, raw) in [
            ("under-threshold Read, static", under.to_string()),
            (
                "under-threshold Read, safe",
                format!("{under}\n[adaptive]\nmode = \"safe\""),
            ),
        ] {
            let cfg = ctk_hook::Config::load_from(None, Some(&raw));
            measure(&mut rows, 0, "per-Read", label, 200, || {
                std::hint::black_box(ctk_hook::run_hook(&payload, &cfg));
            });
        }

        let base = "[read]\nthreshold_tokens = 10\nnever_compress = []";
        for (label, raw) in [
            ("whole Read through run_hook, static", base.to_string()),
            (
                "whole Read through run_hook, safe",
                format!("{base}\n[adaptive]\nmode = \"safe\""),
            ),
        ] {
            let cfg = ctk_hook::Config::load_from(None, Some(&raw));
            measure(&mut rows, 0, "per-Read", label, 200, || {
                std::hint::black_box(ctk_hook::run_hook(&payload, &cfg));
            });
        }
    }

    println!("\n## Governor-only cost (p95)\n");
    println!("| records | category | operation | min ms | median ms | p95 ms | iterations |");
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    for row in &rows {
        println!(
            "| {} | {} | {} | {:.3} | {:.3} | {:.3} | {} |",
            row.records,
            row.category,
            row.operation,
            row.min_ms,
            row.median_ms,
            row.p95_ms,
            row.iterations
        );
    }

    // What the spec's targets actually cover is governor OVERHEAD — what
    // adaptive mode adds over static — not the compressor's own work, which
    // runs whether or not this feature exists. Report it that way.
    let find = |operation: &str| {
        rows.iter()
            .find(|row| row.operation == operation)
            .map(|row| row.p95_ms)
            .unwrap_or(f64::NAN)
    };
    let per_read_overhead =
        find("whole Read through run_hook, safe") - find("whole Read through run_hook, static");
    let rebuild = |category: &str| {
        rows.iter()
            .filter(|row| row.category == category && row.operation == "rebuild (adaptive refresh)")
            .fold(0.0_f64, |worst, row| worst.max(row.p95_ms))
    };
    println!(
        "\nGovernor overhead, p95:\n  per Read (safe minus static): {per_read_overhead:.3} ms \
         (target 2 ms)\n  per turn, typical (rebuild): {:.3} ms (target 2 ms)\n  \
         per turn, large (rebuild): {:.3} ms (target 5 ms)",
        rebuild("typical"),
        rebuild("large")
    );
    println!(
        "\nWhole compressed Read, p95: static {:.3} ms, safe {:.3} ms. The static \
         figure is the compressor, not the governor.",
        find("whole Read through run_hook, static"),
        find("whole Read through run_hook, safe")
    );
}
