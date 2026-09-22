//! `ctk stats`: estimated payload savings and confidence-attributed recovery.

use ctk_hook::ledger::{Confidence, Ledger, Totals};

pub fn run(json: bool) {
    let data = crate::project_root().join(".cubtoken");
    let report = collect(&data);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string())
        );
        return;
    }
    print_text(&report);
}

fn collect(data: &std::path::Path) -> serde_json::Value {
    let mut per_tool: std::collections::BTreeMap<String, Totals> = Default::default();
    let mut legacy_refetches = 0usize;
    let mut legacy_refetch_tokens = 0usize;
    let mut legacy_refetch_duration_ms = 0u64;
    let mut attributed_recovery_tokens = 0usize;
    let mut attributed_recovery_events = 0usize;
    let mut visible_output_tokens = 0usize;

    for ledger in Ledger::load_all(data) {
        legacy_refetches = legacy_refetches.saturating_add(ledger.refetches());
        legacy_refetch_tokens = legacy_refetch_tokens.saturating_add(ledger.refetch_tokens());
        legacy_refetch_duration_ms =
            legacy_refetch_duration_ms.saturating_add(ledger.refetch_duration_ms());
        visible_output_tokens =
            visible_output_tokens.saturating_add(ledger.visible_output_tokens());
        for recovery in ledger.recoveries() {
            if recovery.confidence == Some(Confidence::High) {
                attributed_recovery_events = attributed_recovery_events.saturating_add(1);
                attributed_recovery_tokens =
                    attributed_recovery_tokens.saturating_add(recovery.tokens);
            }
        }
        for (tool, totals) in ledger.per_tool() {
            let entry = per_tool.entry(tool).or_default();
            entry.tokens_in = entry.tokens_in.saturating_add(totals.tokens_in);
            entry.tokens_out = entry.tokens_out.saturating_add(totals.tokens_out);
        }
    }
    let grand = per_tool
        .values()
        .fold(Totals::default(), |mut total, value| {
            total.tokens_in = total.tokens_in.saturating_add(value.tokens_in);
            total.tokens_out = total.tokens_out.saturating_add(value.tokens_out);
            total
        });
    let gross_saved = grand.tokens_in.saturating_sub(grand.tokens_out);
    let net_context_saved = gross_saved.saturating_sub(attributed_recovery_tokens);
    let recovery_overhead_percent = (attributed_recovery_tokens as u128 * 100)
        .checked_div(gross_saved as u128)
        .unwrap_or(0);
    let rows: Vec<_> = per_tool
        .iter()
        .map(|(tool, totals)| {
            serde_json::json!({
                "tool": tool,
                "tokens_in": totals.tokens_in,
                "tokens_out": totals.tokens_out,
                "gross_saved": totals.tokens_in.saturating_sub(totals.tokens_out),
            })
        })
        .collect();
    serde_json::json!({
        "estimated": true,
        "per_tool": rows,
        "gross_context_saved": gross_saved,
        "attributed_recovery_tokens": attributed_recovery_tokens,
        "attributed_recovery_events": attributed_recovery_events,
        "recovery_overhead_percent": recovery_overhead_percent,
        "net_context_saved": net_context_saved,
        "legacy_refetch": {
            "events": legacy_refetches,
            "tokens": legacy_refetch_tokens,
            "duration_ms": legacy_refetch_duration_ms,
        },
        "visible_output_tokens": visible_output_tokens,
    })
}

fn print_text(report: &serde_json::Value) {
    let rows = report["per_tool"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        println!("no savings recorded yet — is the hook installed? (ctk doctor)");
        return;
    }
    println!(
        "{:<10} {:>12} {:>12} {:>12} {:>7}",
        "tool", "tokens in", "tokens out", "saved", "saved%"
    );
    for row in &rows {
        let label = row["tool"].as_str().unwrap_or("unknown");
        let tokens_in = row["tokens_in"].as_u64().unwrap_or(0) as usize;
        let tokens_out = row["tokens_out"].as_u64().unwrap_or(0) as usize;
        print_row(
            label,
            Totals {
                tokens_in,
                tokens_out,
            },
        );
    }
    let tokens_in = rows.iter().fold(0usize, |total, row| {
        total.saturating_add(row["tokens_in"].as_u64().unwrap_or(0) as usize)
    });
    let tokens_out = rows.iter().fold(0usize, |total, row| {
        total.saturating_add(row["tokens_out"].as_u64().unwrap_or(0) as usize)
    });
    print_row(
        "TOTAL",
        Totals {
            tokens_in,
            tokens_out,
        },
    );
    println!("\ncounts are estimates (~3.5 chars/token), not tokenizer output");
    let legacy = &report["legacy_refetch"];
    println!(
        "follow-up Read(offset/limit) calls into compressed files: {} (~{} tokens, {}ms tool time)",
        legacy["events"].as_u64().unwrap_or(0),
        legacy["tokens"].as_u64().unwrap_or(0),
        legacy["duration_ms"].as_u64().unwrap_or(0),
    );
    println!(
        "estimated net saved after refetch output: {} tokens",
        report["gross_context_saved"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(legacy["tokens"].as_u64().unwrap_or(0))
    );
    println!(
        "attributed recovery overhead: {}% ({} high-confidence events, {} tokens); estimated Net Context Saved: {} tokens",
        report["recovery_overhead_percent"].as_u64().unwrap_or(0),
        report["attributed_recovery_events"].as_u64().unwrap_or(0),
        report["attributed_recovery_tokens"].as_u64().unwrap_or(0),
        report["net_context_saved"].as_u64().unwrap_or(0),
    );
    println!(
        "estimated visible assistant output: {} tokens",
        report["visible_output_tokens"].as_u64().unwrap_or(0)
    );
}

fn print_row(label: &str, totals: Totals) {
    let saved = totals.tokens_in.saturating_sub(totals.tokens_out);
    let percent = (saved as u128 * 100 + totals.tokens_in as u128 / 2)
        .checked_div(totals.tokens_in as u128)
        .unwrap_or(0);
    println!(
        "{label:<10} {:>12} {:>12} {saved:>12} {percent:>6}%",
        totals.tokens_in, totals.tokens_out
    );
}
