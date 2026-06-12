use std::io::Read as _;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "stk", about = "smalltoke: compress Claude Code tool outputs")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Append raw stdin to a JSONL file (debug/fixture capture)
    Record { path: PathBuf },
    /// PostToolUse hook handler: hook JSON on stdin, decision JSON on stdout
    Hook,
    /// Install the hook into .claude/settings.json
    Init {
        #[arg(long)]
        global: bool,
    },
    /// Report token savings from the local ledger
    Stats,
    /// Check installation health
    Doctor,
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Record { path } => record(&path),
        // Stubs: silent success until implemented (hook must never break a session)
        Cmd::Hook | Cmd::Init { .. } | Cmd::Stats | Cmd::Doctor => {}
    }
}

fn record(path: &std::path::Path) {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let line = input.trim_end_matches('\n');
    if line.is_empty() {
        return;
    }
    let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    use std::io::Write as _;
    let _ = writeln!(f, "{line}");
}
