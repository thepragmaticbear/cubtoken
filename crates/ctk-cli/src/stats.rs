//! `ctk stats`: replay every session ledger under `.cubtoken/` and print a
//! per-tool savings table plus lifetime totals.

use ctk_hook::ledger::{Ledger, Totals};

pub fn run() {
    let mut per_tool: std::collections::BTreeMap<String, Totals> = Default::default();
    let mut refetches = 0usize;
    let mut refetch_tokens = 0usize;
    let mut refetch_duration_ms = 0u64;

    if let Ok(entries) = std::fs::read_dir(".cubtoken") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(session) = name
                .strip_prefix("session-")
                .and_then(|n| n.strip_suffix(".jsonl"))
            else {
                continue;
            };
            let ledger = Ledger::open(std::path::Path::new(".cubtoken"), session);
            refetches = refetches.saturating_add(ledger.refetches());
            refetch_tokens = refetch_tokens.saturating_add(ledger.refetch_tokens());
            refetch_duration_ms = refetch_duration_ms.saturating_add(ledger.refetch_duration_ms());
            for (tool, t) in ledger.per_tool() {
                let e = per_tool.entry(tool).or_default();
                e.tokens_in = e.tokens_in.saturating_add(t.tokens_in);
                e.tokens_out = e.tokens_out.saturating_add(t.tokens_out);
            }
        }
    }

    if per_tool.is_empty() {
        println!("no savings recorded yet — is the hook installed? (ctk doctor)");
        return;
    }

    println!(
        "{:<10} {:>12} {:>12} {:>12} {:>7}",
        "tool", "tokens in", "tokens out", "saved", "saved%"
    );
    let mut grand = Totals::default();
    for (tool, t) in &per_tool {
        print_row(tool, t);
        grand.tokens_in = grand.tokens_in.saturating_add(t.tokens_in);
        grand.tokens_out = grand.tokens_out.saturating_add(t.tokens_out);
    }
    print_row("TOTAL", &grand);
    println!("\ncounts are estimates (~3.5 chars/token), not tokenizer output");

    // The cost side: every targeted Read back into a file we compressed is a
    // round trip the model would not have needed on the full text.
    println!(
        "follow-up Read(offset/limit) calls into compressed files: {refetches} \
         (~{refetch_tokens} tokens, {refetch_duration_ms}ms tool time)"
    );
    let gross_saved = grand.tokens_in.saturating_sub(grand.tokens_out);
    let net_saved = gross_saved.saturating_sub(refetch_tokens);
    println!("estimated net saved after refetch output: {net_saved} tokens");
}

fn print_row(label: &str, t: &Totals) {
    let saved = t.tokens_in.saturating_sub(t.tokens_out);
    // rounded integer percentage
    let pct = (saved as u128 * 100 + t.tokens_in as u128 / 2)
        .checked_div(t.tokens_in as u128)
        .unwrap_or(0);
    println!(
        "{label:<10} {:>12} {:>12} {saved:>12} {pct:>6}%",
        t.tokens_in, t.tokens_out
    );
}
