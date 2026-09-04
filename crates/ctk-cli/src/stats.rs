//! `ctk stats`: replay every session ledger under `.cubtoken/` and print a
//! per-tool savings table plus lifetime totals.

use ctk_hook::ledger::{Ledger, Totals};

pub fn run() {
    let mut per_tool: std::collections::BTreeMap<String, Totals> = Default::default();
    let mut refetches = 0usize;

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
            refetches += ledger.refetches();
            for (tool, t) in ledger.per_tool() {
                let e = per_tool.entry(tool).or_default();
                e.tokens_in += t.tokens_in;
                e.tokens_out += t.tokens_out;
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
        grand.tokens_in += t.tokens_in;
        grand.tokens_out += t.tokens_out;
    }
    print_row("TOTAL", &grand);
    println!("\ncounts are estimates (~3.5 chars/token), not tokenizer output");

    // The cost side: every targeted Read back into a file we compressed is a
    // round trip the model would not have needed on the full text.
    println!(
        "follow-up Read(offset/limit) calls into compressed files: {refetches}\n\
         a high count means skeletons are eliding what the model needs \
         — raise read.threshold_tokens"
    );
}

fn print_row(label: &str, t: &Totals) {
    let saved = t.tokens_in.saturating_sub(t.tokens_out);
    // rounded integer percentage
    let pct = (saved * 100 + t.tokens_in / 2)
        .checked_div(t.tokens_in)
        .unwrap_or(0);
    println!(
        "{label:<10} {:>12} {:>12} {saved:>12} {pct:>6}%",
        t.tokens_in, t.tokens_out
    );
}
