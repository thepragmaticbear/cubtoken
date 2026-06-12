//! `stk init`: install the PostToolUse hook into Claude Code settings and
//! write a starter project config. Read-modify-write; preserves everything
//! we don't own, idempotent for what we do.

use std::path::{Path, PathBuf};

pub const MATCHER: &str = "Read|Grep|Glob|Bash|Edit|Write|NotebookEdit";

pub fn run(global: bool) -> Result<(), String> {
    let settings_path = settings_path(global)?;
    install_hook(&settings_path)?;
    if !global {
        write_starter_config(Path::new(".smalltoke.toml"));
    }
    println!("smalltoke hook installed in {}", settings_path.display());
    println!(
        "note: Claude Code snapshots hooks at session start — restart your session to activate"
    );
    Ok(())
}

pub fn settings_path(global: bool) -> Result<PathBuf, String> {
    if global {
        let home = std::env::var_os("HOME").ok_or("HOME not set")?;
        Ok(PathBuf::from(home).join(".claude/settings.json"))
    } else {
        Ok(PathBuf::from(".claude/settings.json"))
    }
}

/// The command we install: absolute path to this binary, so the hook works
/// regardless of PATH in the hook's execution environment.
pub fn hook_command() -> String {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "stk".to_string());
    format!("{exe} hook")
}

fn install_hook(settings_path: &Path) -> Result<(), String> {
    let mut settings: serde_json::Value = match std::fs::read_to_string(settings_path) {
        Ok(content) => serde_json::from_str(&content)
            .map_err(|e| format!("{} is not valid JSON: {e}", settings_path.display()))?,
        Err(_) => serde_json::json!({}),
    };

    if !settings.is_object() {
        return Err(format!("{} is not a JSON object", settings_path.display()));
    }
    let hooks = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let post = hooks
        .as_object_mut()
        .ok_or("settings.hooks is not an object")?
        .entry("PostToolUse")
        .or_insert_with(|| serde_json::json!([]));
    let entries = post
        .as_array_mut()
        .ok_or("settings.hooks.PostToolUse is not an array")?;

    // remove any prior stk entry, then append ours (idempotent)
    entries.retain(|e| {
        e.pointer("/hooks/0/command")
            .and_then(|c| c.as_str())
            .map(|c| !is_stk_hook_command(c))
            .unwrap_or(true)
    });
    entries.push(serde_json::json!({
        "matcher": MATCHER,
        "hooks": [{ "type": "command", "command": hook_command() }]
    }));

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let pretty = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(settings_path, pretty + "\n").map_err(|e| e.to_string())
}

pub fn is_stk_hook_command(cmd: &str) -> bool {
    cmd.ends_with("stk hook") || cmd.contains("stk\" hook") || cmd.contains("stk' hook")
}

fn write_starter_config(path: &Path) {
    if path.exists() {
        return;
    }
    let starter = "\
# smalltoke project config — see https://github.com/brandon/smalltoke
[read]
enabled = true
threshold_tokens = 2000
never_compress = [\"**/*.md\", \"**/.env*\"]

[grep]
enabled = true
max_matches_per_file = 5

[glob]
enabled = true
max_paths = 50

[bash]
enabled = false  # set true if you don't use rtk

[stats]
ledger = true
";
    let _ = std::fs::write(path, starter);
}
