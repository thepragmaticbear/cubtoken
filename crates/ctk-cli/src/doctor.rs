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
    for hook in &found {
        println!("      {} → {}", hook.settings.display(), hook.command);
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
