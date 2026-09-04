//! `ctk doctor`: health checks for the installation. Prints PASS/FAIL lines;
//! exit code 1 if anything fails.

use crate::init;
use std::path::PathBuf;

pub fn run() -> bool {
    let mut ok = true;

    let found = installed_hooks();
    ok &= report(
        "PostToolUse hook installed (project, local, or global settings)",
        !found.is_empty(),
    );
    for (settings, command) in &found {
        println!("      {} → {command}", settings.display());
    }

    // An `init` embeds an absolute path to the binary. Move or `cargo clean`
    // that binary and every hook invocation silently fails from then on, so
    // check the path still resolves rather than trusting the settings entry.
    for (settings, command) in &found {
        let bin = hook_binary(command);
        ok &= report(
            &format!("hook binary exists ({})", settings.display()),
            bin.as_ref().map(|p| p.is_file()).unwrap_or(false),
        );
    }

    ok &= report(
        ".cubtoken/ writable",
        std::fs::create_dir_all(".cubtoken").is_ok(),
    );

    // rtk coexistence (informational only)
    let rtk = which("rtk");
    println!(
        "INFO  rtk {}: bash compression {}",
        if rtk { "detected" } else { "not found" },
        if rtk {
            "left to rtk (bash.enabled should stay false)"
        } else {
            "available via [bash] enabled = true"
        }
    );

    ok
}

/// Every settings file Claude Code reads, in precedence order. `settings.local.json`
/// is included because `init` writes there for absolute-path installs and a
/// doctor that ignored it reported FAIL on a working setup.
fn settings_files() -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from(".claude/settings.json"),
        PathBuf::from(".claude/settings.local.json"),
    ];
    if let Ok(global) = init::settings_path(true) {
        paths.push(global);
    }
    paths
}

fn installed_hooks() -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    for path in settings_files() {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(settings) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        let Some(entries) = settings
            .pointer("/hooks/PostToolUse")
            .and_then(|p| p.as_array())
        else {
            continue;
        };
        for entry in entries {
            if let Some(cmd) = entry.pointer("/hooks/0/command").and_then(|c| c.as_str()) {
                if init::is_ctk_hook_command(cmd) {
                    found.push((path.clone(), cmd.to_string()));
                }
            }
        }
    }
    found
}

/// `"/abs/path/ctk" hook` or `/abs/path/ctk hook` → the binary path.
fn hook_binary(command: &str) -> Option<PathBuf> {
    let bin = command.trim_end().strip_suffix("hook")?.trim_end();
    let bin = bin.trim_matches(['"', '\'']);
    if bin.is_empty() {
        return None;
    }
    if bin.contains('/') {
        return Some(PathBuf::from(bin));
    }
    // bare `ctk`: resolve through PATH
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(bin))
            .find(|p| p.is_file())
    })
}

fn report(what: &str, pass: bool) -> bool {
    println!("{}  {what}", if pass { "PASS" } else { "FAIL" });
    pass
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
        .unwrap_or(false)
}
