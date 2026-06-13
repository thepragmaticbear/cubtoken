mod doctor;
mod init;
mod stats;

use std::io::Read as _;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ctk", about = "cubtoken: compress Claude Code tool outputs")]
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
        Cmd::Hook => hook(),
        Cmd::Init { global } => {
            if let Err(e) = init::run(global) {
                eprintln!("init failed: {e}");
                std::process::exit(1);
            }
        }
        Cmd::Doctor => {
            if !doctor::run() {
                std::process::exit(1);
            }
        }
        Cmd::Stats => stats::run(),
    }
}

/// Fail-open wrapper: any panic or error inside the hook prints nothing and
/// exits 0, so the original tool output passes through untouched.
fn hook() {
    let result = std::panic::catch_unwind(|| {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input).ok()?;
        let cfg = ctk_hook::Config::default();
        ctk_hook::run_hook(&input, &cfg)
    });
    match result {
        Ok(Some(json)) => println!("{json}"),
        Ok(None) => {}
        Err(panic) => log_error(&format!("hook panicked: {panic:?}")),
    }
}

/// Best-effort error log; never writes to stdout/stderr (those belong to the
/// hook protocol).
fn log_error(msg: &str) {
    let dir = std::path::Path::new(".cubtoken");
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("errors.log"))
    {
        use std::io::Write as _;
        let _ = writeln!(f, "{msg}");
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
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    use std::io::Write as _;
    let _ = writeln!(f, "{line}");
}
