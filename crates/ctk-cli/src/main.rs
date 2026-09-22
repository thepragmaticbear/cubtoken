mod adaptive;
mod doctor;
mod init;
mod stats;

use std::io::Read as _;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// The project the current directory belongs to — the nearest ancestor with a
/// `.cubtoken.toml` or `.git`, the same rule the hook uses to place
/// `.cubtoken/`. Without this, reporting commands only work from the root.
fn project_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    ctk_hook::project_root(&cwd)
}

#[derive(Parser)]
#[command(
    name = "ctk",
    version,
    about = "cubtoken: compress Claude Code tool outputs"
)]
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
    /// Install the hook into local Claude Code settings
    Init {
        #[arg(long)]
        global: bool,
    },
    /// Remove the hook from local Claude Code settings
    Uninstall {
        #[arg(long)]
        global: bool,
    },
    /// Report estimated token savings from the local ledger
    Stats {
        #[arg(long)]
        json: bool,
    },
    /// Inspect or reset the Read governor
    Adaptive {
        #[command(subcommand)]
        cmd: AdaptiveCmd,
    },
    /// Inspect the opt-in output profile
    Output {
        #[command(subcommand)]
        cmd: OutputCmd,
    },
    /// Check installation health
    Doctor,
}

#[derive(Subcommand)]
enum AdaptiveCmd {
    Status {
        #[arg(long)]
        json: bool,
    },
    Explain {
        file: String,
    },
    Reset,
}

#[derive(Subcommand)]
enum OutputCmd {
    Status,
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
        Cmd::Uninstall { global } => {
            if let Err(e) = init::uninstall(global) {
                eprintln!("uninstall failed: {e}");
                std::process::exit(1);
            }
        }
        Cmd::Doctor => {
            if !doctor::run() {
                std::process::exit(1);
            }
        }
        Cmd::Stats { json } => stats::run(json),
        Cmd::Adaptive { cmd } => {
            let result = match cmd {
                AdaptiveCmd::Status { json } => adaptive::status(json),
                AdaptiveCmd::Explain { file } => adaptive::explain(&file),
                AdaptiveCmd::Reset => adaptive::reset(),
            };
            if let Err(error) = result {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        Cmd::Output {
            cmd: OutputCmd::Status,
        } => {
            let root = project_root();
            let cfg = match ctk_hook::Config::try_load_for(&root) {
                Ok(cfg) => cfg,
                Err(error) => {
                    eprintln!("output status failed: {error}");
                    std::process::exit(1);
                }
            };
            let visible = ctk_hook::ledger::Ledger::load_all(&root.join(".cubtoken"))
                .iter()
                .fold(0usize, |total, ledger| {
                    total.saturating_add(ledger.visible_output_tokens())
                });
            println!("output profile: {:?}", cfg.output.mode);
            println!(
                "estimated visible assistant output: {visible} tokens (not authoritative API usage)"
            );
        }
    }
}

/// Fail-open wrapper: any panic or error inside the hook prints nothing and
/// exits 0, so the original tool output passes through untouched.
fn hook() {
    let result = std::panic::catch_unwind(|| {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input).ok()?;
        ctk_hook::run_hook_auto(&input)
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
