//! `ctk init`: install the PostToolUse hook into Claude Code settings and
//! write a starter project config. Read-modify-write; preserves everything
//! we don't own, idempotent for what we do.

use std::path::{Path, PathBuf};
use std::{ffi::OsStr, io::Write as _};

pub const MATCHER: &str = "Read|Grep|Glob|Bash|Edit|Write|NotebookEdit";

pub fn run(global: bool) -> Result<(), String> {
    let settings_path = settings_path(global)?;
    install_hook(&settings_path)?;
    if !global {
        write_starter_config(Path::new(".cubtoken.toml"));
    }
    println!("cubtoken hook installed in {}", settings_path.display());
    if let Some(exe) = installed_from_build_dir() {
        println!(
            "warning: the hook points at a build directory ({exe}) — `cargo clean` or a \
             rebuild will silently break it. Run `cargo install --path crates/ctk-cli`, \
             then `ctk init` again."
        );
    }
    println!(
        "note: Claude Code snapshots hooks at session start — restart your session to activate"
    );
    Ok(())
}

/// `ctk uninstall`: the inverse of `run`. Removes our PostToolUse entry from
/// the same settings file `init` writes, leaves everything else alone, and
/// never deletes `.cubtoken.toml` or `.cubtoken/` — those are the user's
/// config and recorded savings, not install state.
pub fn uninstall(global: bool) -> Result<(), String> {
    let settings_path = settings_path(global)?;
    let removed = remove_hook(&settings_path)?;

    if removed == 0 {
        println!("no cubtoken hook found in {}", settings_path.display());
        // `init --global` followed by a bare `uninstall` would otherwise report
        // "nothing found" while the global hook keeps running. Say where it is.
        let elsewhere = other_scopes_with_hook(&settings_path);
        if !elsewhere.is_empty() {
            println!("but a cubtoken hook is still installed in:");
            for path in &elsewhere {
                println!("      {}", path.display());
            }
            println!(
                "remove it with `ctk uninstall{}`",
                if global { "" } else { " --global" }
            );
        }
        return Ok(());
    }

    println!(
        "cubtoken hook removed from {} ({removed} {})",
        settings_path.display(),
        if removed == 1 { "entry" } else { "entries" }
    );
    println!("note: Claude Code snapshots hooks at session start — restart your session");
    println!("`.cubtoken.toml` and `.cubtoken/` were left in place; delete them to remove config and saved stats");
    Ok(())
}

/// Strip our entries from `hooks.PostToolUse`, returning how many went. The
/// file is only rewritten when something actually changed, and the empty
/// `PostToolUse`/`hooks` containers we would leave behind are cleaned up.
fn remove_hook(settings_path: &Path) -> Result<usize, String> {
    reject_symlink(settings_path)?;
    if let Some(parent) = settings_path.parent() {
        reject_symlink(parent)?;
    }
    let mut settings: serde_json::Value = match std::fs::read_to_string(settings_path) {
        Ok(content) => serde_json::from_str(&content)
            .map_err(|e| format!("{} is not valid JSON: {e}", settings_path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("cannot read {}: {error}", settings_path.display())),
    };

    let Some(entries) = settings
        .pointer_mut("/hooks/PostToolUse")
        .and_then(|p| p.as_array_mut())
    else {
        return Ok(0);
    };
    let before = entries.len();
    entries.retain(|e| !is_ctk_hook_entry(e));
    let removed = before - entries.len();
    if removed == 0 {
        return Ok(0);
    }
    let post_empty = entries.is_empty();

    if let Some(hooks) = settings.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        if post_empty {
            hooks.remove("PostToolUse");
        }
        if hooks.is_empty() {
            settings
                .as_object_mut()
                .expect("settings is an object: we just indexed it")
                .remove("hooks");
        }
    }

    let pretty = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    atomic_write(settings_path, &(pretty + "\n"))?;
    Ok(removed)
}

/// Settings files other than `exclude` that still carry a cubtoken hook.
fn other_scopes_with_hook(exclude: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![
        PathBuf::from(".claude/settings.json"),
        PathBuf::from(".claude/settings.local.json"),
    ];
    if let Ok(global) = settings_path(true) {
        candidates.push(global);
    }
    candidates
        .into_iter()
        .filter(|path| path != exclude && has_ctk_hook(path))
        .collect()
}

fn has_ctk_hook(path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(settings) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    settings
        .pointer("/hooks/PostToolUse")
        .and_then(|p| p.as_array())
        .is_some_and(|entries| entries.iter().any(is_ctk_hook_entry))
}

/// `cargo run`/`target/release` installs embed a path that `cargo clean`
/// deletes. Detect that so `init` can say so instead of failing silently later.
fn installed_from_build_dir() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .any(|dir| dir.file_name() == Some(OsStr::new("target")))
        .then(|| exe.display().to_string())
}

pub fn settings_path(global: bool) -> Result<PathBuf, String> {
    if global {
        if let Some(config_dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
            return Ok(PathBuf::from(config_dir).join("settings.json"));
        }
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .ok_or("HOME/USERPROFILE not set")?;
        Ok(PathBuf::from(home).join(".claude/settings.json"))
    } else {
        Ok(PathBuf::from(".claude/settings.local.json"))
    }
}

/// The executable we install: absolute path to this binary, so the hook works
/// regardless of PATH in the hook's execution environment. The `hook` argument
/// is stored separately so paths are never reparsed by a shell.
pub fn hook_command() -> String {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "ctk".to_string());
    exe
}

fn install_hook(settings_path: &Path) -> Result<(), String> {
    reject_symlink(settings_path)?;
    if let Some(parent) = settings_path.parent() {
        reject_symlink(parent)?;
    }
    let mut settings: serde_json::Value = match std::fs::read_to_string(settings_path) {
        Ok(content) => serde_json::from_str(&content)
            .map_err(|e| format!("{} is not valid JSON: {e}", settings_path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(error) => return Err(format!("cannot read {}: {error}", settings_path.display())),
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

    // remove any prior ctk entry, then append ours (idempotent)
    entries.retain(|e| !is_ctk_hook_entry(e));
    entries.push(serde_json::json!({
        "matcher": MATCHER,
        "hooks": [{ "type": "command", "command": hook_command(), "args": ["hook"] }]
    }));

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let pretty = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    atomic_write(settings_path, &(pretty + "\n"))
}

pub fn is_ctk_hook_command(cmd: &str) -> bool {
    let Some(exe) = cmd.trim().strip_suffix(" hook") else {
        return false;
    };
    is_ctk_executable(exe.trim().trim_matches(['"', '\'']))
}

pub fn is_ctk_hook_entry(entry: &serde_json::Value) -> bool {
    let Some(hook) = entry.pointer("/hooks/0") else {
        return false;
    };
    let Some(command) = hook.get("command").and_then(|v| v.as_str()) else {
        return false;
    };
    match hook.get("args") {
        Some(args) => {
            args.as_array()
                .filter(|args| args.len() == 1)
                .and_then(|args| args[0].as_str())
                == Some("hook")
                && is_ctk_executable(command)
        }
        None => is_ctk_hook_command(command), // migrate legacy shell-form installs
    }
}

fn is_ctk_executable(exe: &str) -> bool {
    Path::new(exe)
        .file_name()
        .and_then(OsStr::to_str)
        .map(|name| name == "ctk" || name == "ctk.exe")
        .unwrap_or(false)
}

fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    reject_symlink(parent)?;
    reject_symlink(path)?;
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("settings.json");
    let temp = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        if let Ok(metadata) = std::fs::metadata(path) {
            std::fs::set_permissions(&temp, metadata.permissions()).map_err(|e| e.to_string())?;
        }
        file.write_all(content.as_bytes())
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        replace_file(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing to write through symlink: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

#[cfg(not(windows))]
fn replace_file(temp: &Path, destination: &Path) -> Result<(), String> {
    std::fs::rename(temp, destination).map_err(|e| e.to_string())
}

#[cfg(windows)]
fn replace_file(temp: &Path, destination: &Path) -> Result<(), String> {
    if !destination.exists() {
        return std::fs::rename(temp, destination).map_err(|e| e.to_string());
    }
    let backup = destination.with_extension("cubtoken-backup");
    std::fs::rename(destination, &backup).map_err(|e| e.to_string())?;
    match std::fs::rename(temp, destination) {
        Ok(()) => {
            let _ = std::fs::remove_file(backup);
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::rename(backup, destination);
            Err(error.to_string())
        }
    }
}

fn write_starter_config(path: &Path) {
    let starter = "\
# cubtoken project config — see https://github.com/thepragmaticbear/cubtoken
[read]
enabled = true
threshold_tokens = 2000
never_compress = [\"**/*.md\", \"**/.env*\"]

[grep]
enabled = true
max_matches_per_file = 5
max_total_matches = 100

[glob]
enabled = true
max_paths = 50

[bash]
enabled = false  # set true if you don't use rtk

[stats]
ledger = true
";
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        let _ = file.write_all(starter.as_bytes());
    }
}
