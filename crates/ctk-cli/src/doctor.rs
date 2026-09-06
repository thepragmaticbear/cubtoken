//! `ctk doctor`: health checks for the installation. Prints PASS/FAIL lines;
//! exit code 1 if anything fails.

use crate::init;
use std::path::PathBuf;

pub fn run() -> bool {
    let mut ok = true;

    let found = installed_hooks();
    let plugins = installed_plugin_hooks();
    ok &= report(
        "PostToolUse hook installed (settings or plugin)",
        !found.is_empty() || !plugins.is_empty(),
    );
    for hook in &found {
        println!("      {} → {}", hook.settings.display(), hook.command);
    }
    for plugin in &plugins {
        println!("      plugin {} → {}", plugin.key, plugin.wrapper.display());
    }

    // A plugin ships no binary: the wrapper resolves `ctk` at call time and
    // fails open when it can't. That silence is the whole failure mode, so
    // check the wrapper is runnable and that `ctk` is actually findable.
    for plugin in &plugins {
        ok &= report(
            &format!("plugin hook wrapper is executable ({})", plugin.key),
            is_executable(&plugin.wrapper),
        );
    }
    if !plugins.is_empty() {
        ok &= report(
            "ctk resolvable for the plugin wrapper (PATH or its fallbacks)",
            which("ctk") || wrapper_fallback_ctk().is_some(),
        );
    }

    // An `init` embeds an absolute path to the binary. Move or `cargo clean`
    // that binary and every hook invocation silently fails from then on, so
    // check the path still resolves rather than trusting the settings entry.
    for hook in &found {
        let bin = hook_binary(&hook.command, hook.exec_form);
        ok &= report(
            &format!("hook binary is executable ({})", hook.settings.display()),
            bin.as_ref().map(|p| is_executable(p)).unwrap_or(false),
        );
    }

    ok &= report("hook core self-test", hook_self_test());

    match ctk_hook::Config::try_load_for(std::path::Path::new(".")) {
        Ok(_) => ok &= report("configuration parses", true),
        Err(error) => {
            ok &= report("configuration parses", false);
            println!("      {error}");
        }
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

struct InstalledHook {
    settings: PathBuf,
    command: String,
    exec_form: bool,
}

fn installed_hooks() -> Vec<InstalledHook> {
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
            if !init::is_ctk_hook_entry(entry) {
                continue;
            }
            if let Some(cmd) = entry.pointer("/hooks/0/command").and_then(|c| c.as_str()) {
                let exec_form = entry.pointer("/hooks/0/args").is_some();
                found.push(InstalledHook {
                    settings: path.clone(),
                    command: if exec_form {
                        format!("{cmd} hook")
                    } else {
                        cmd.to_string()
                    },
                    exec_form,
                });
            }
        }
    }
    found
}

struct PluginHook {
    key: String,
    wrapper: PathBuf,
}

/// A plugin registers its hook in the plugin's own `hooks/hooks.json`, which no
/// settings file mentions — so `installed_hooks` is structurally blind to it and
/// reported FAIL on a working plugin install. Claude Code's install index is the
/// only place that maps a plugin to the directory it was installed into.
///
/// Best-effort throughout: a missing or reshaped index just means "no plugin
/// found", never an error. Ownership is decided by the hook command naming our
/// wrapper, not by the plugin's name, which a user can change.
fn installed_plugin_hooks() -> Vec<PluginHook> {
    let Ok(config) = init::config_dir() else {
        return Vec::new();
    };
    let enabled = read_json(&config.join("settings.json"))
        .and_then(|s| s.get("enabledPlugins").cloned())
        .unwrap_or(serde_json::Value::Null);
    let Some(index) = read_json(&config.join("plugins/installed_plugins.json")) else {
        return Vec::new();
    };
    let Some(plugins) = index.get("plugins").and_then(|p| p.as_object()) else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for (key, installs) in plugins {
        if enabled.get(key).and_then(|v| v.as_bool()) != Some(true) {
            continue;
        }
        for install in installs.as_array().unwrap_or(&Vec::new()) {
            let Some(path) = install.get("installPath").and_then(|p| p.as_str()) else {
                continue;
            };
            let root = PathBuf::from(path);
            let Some(hooks) = read_json(&root.join("hooks/hooks.json")) else {
                continue;
            };
            let Some(entries) = hooks
                .pointer("/hooks/PostToolUse")
                .and_then(|p| p.as_array())
            else {
                continue;
            };
            for command in entries
                .iter()
                .filter_map(|e| e.pointer("/hooks/0/command").and_then(|c| c.as_str()))
            {
                if let Some(wrapper) = our_plugin_wrapper(command, &root) {
                    found.push(PluginHook {
                        key: key.clone(),
                        wrapper,
                    });
                }
            }
        }
    }
    found
}

/// `${CLAUDE_PLUGIN_ROOT}/bin/ctk-hook` → the resolved wrapper path, if the
/// command is ours. Claude Code expands that variable at hook time; here we
/// substitute the install directory the index gave us.
fn our_plugin_wrapper(command: &str, root: &std::path::Path) -> Option<PathBuf> {
    let command = command.trim().trim_matches(['"', '\'']);
    let expanded = command
        .replace("${CLAUDE_PLUGIN_ROOT}", &root.display().to_string())
        .replace("$CLAUDE_PLUGIN_ROOT", &root.display().to_string());
    let path = PathBuf::from(&expanded);
    let name = path.file_name().and_then(std::ffi::OsStr::to_str)?;
    (name == "ctk-hook" || name == "ctk-hook.cmd").then_some(path)
}

fn read_json(path: &std::path::Path) -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The same fallbacks `plugins/cubtoken/bin/ctk-hook` searches when `ctk` is
/// not on PATH. Kept in step with that script.
fn wrapper_fallback_ctk() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let home = PathBuf::from(home);
    [
        home.join(".cargo/bin/ctk"),
        home.join(".local/bin/ctk"),
        PathBuf::from("/usr/local/bin/ctk"),
    ]
    .into_iter()
    .find(|candidate| is_executable(candidate))
}

/// `"/abs/path/ctk" hook` or `/abs/path/ctk hook` → the binary path.
fn hook_binary(command: &str, exec_form: bool) -> Option<PathBuf> {
    let bin = if exec_form {
        command.trim_end().strip_suffix(" hook")?
    } else {
        command.trim_end().strip_suffix("hook")?.trim_end()
    };
    let bin = bin.trim_matches(['"', '\'']);
    if bin.is_empty() {
        return None;
    }
    let candidate = PathBuf::from(bin);
    if candidate.is_absolute() || candidate.components().count() > 1 {
        return Some(PathBuf::from(bin));
    }
    // bare `ctk`: resolve through PATH
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(bin))
            .find(|p| p.is_file())
    })
}

fn is_executable(path: &std::path::Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn hook_self_test() -> bool {
    // Fat bodies, one signature each: the shape a real compression has. A
    // file of one-line functions is mostly signature and would (correctly)
    // fail the worth-it gate, which would make this check useless.
    let mut content = String::new();
    for i in 0..40 {
        content.push_str(&format!("pub fn sample_{i}(input: &str) -> usize {{\n"));
        for j in 0..20 {
            content.push_str(&format!("    let step_{j} = input.len() + {j};\n"));
        }
        content.push_str("    input.len()\n}\n\n");
    }
    let lines = content.lines().count();
    let payload = serde_json::json!({
        "tool_name": "Read",
        "session_id": "doctor-self-test",
        "cwd": ".",
        "tool_input": {"file_path": "/tmp/cubtoken-doctor.rs"},
        "tool_response": {"type": "text", "file": {
            "filePath": "/tmp/cubtoken-doctor.rs",
            "content": content,
            "numLines": lines,
            "startLine": 1,
            "totalLines": lines
        }}
    });
    let cfg = ctk_hook::Config::load_from(
        None,
        Some("[read]\nthreshold_tokens = 10\nnever_compress = []\n[stats]\nledger = false"),
    );
    ctk_hook::run_hook(&payload.to_string(), &cfg).is_some()
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
