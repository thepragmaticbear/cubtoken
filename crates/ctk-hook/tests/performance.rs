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

fn p95(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    // Nearest-rank p95: the smallest value at or above 95% of the samples.
    let rank = ((samples.len() as f64) * 0.95).ceil() as usize;
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
    p95_ms: f64,
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
    rows.push(Row {
        records,
        category,
        operation,
        p95_ms: ms(p95(samples)),
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

        let iterations = if records >= 5_000 { 40 } else { 200 };

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

        // The per-Read cost of safe mode: it reads the small state snapshot,
        // not the ledgers. Measured separately because it runs far more often
        // than `refresh`, which only fires on Stop and SessionEnd.
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

        let rebuild_iterations = if records >= 5_000 { 10 } else { 40 };
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

    println!("\n## Governor-only cost (p95)\n");
    println!("| records | category | operation | p95 ms | iterations |");
    println!("| --- | --- | --- | --- | --- |");
    for row in &rows {
        println!(
            "| {} | {} | {} | {:.3} | {} |",
            row.records, row.category, row.operation, row.p95_ms, row.iterations
        );
    }

    // Per-tool-call work and per-turn work have very different frequencies, so
    // reporting one worst number over both would misrepresent the cost.
    const PER_TURN: &[&str] = &["rebuild (adaptive refresh)"];
    let worst = |category: &str, per_turn: bool| {
        rows.iter()
            .filter(|row| row.category == category && PER_TURN.contains(&row.operation) == per_turn)
            .fold(0.0_f64, |worst, row| worst.max(row.p95_ms))
    };
    println!(
        "\nPer-tool-call p95: typical {:.3} ms (target 2 ms), large {:.3} ms (target 5 ms).",
        worst("typical", false),
        worst("large", false)
    );
    println!(
        "Per-turn p95 (Stop/SessionEnd only): typical {:.3} ms (target 2 ms), \
         large {:.3} ms (target 5 ms).",
        worst("typical", true),
        worst("large", true)
    );

    let worst_typical = rows
        .iter()
        .filter(|row| row.category == "typical")
        .fold(0.0_f64, |worst, row| worst.max(row.p95_ms));
    let worst_large = rows
        .iter()
        .filter(|row| row.category == "large")
        .fold(0.0_f64, |worst, row| worst.max(row.p95_ms));
    println!(
        "\nWorst governor-only p95: typical (100-1,000 records) {worst_typical:.3} ms \
         against a 2 ms target; large (5,000-10,000 records) {worst_large:.3} ms \
         against a 5 ms target."
    );
}
