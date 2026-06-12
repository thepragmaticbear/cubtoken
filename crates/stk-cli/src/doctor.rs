//! `stk doctor`: health checks for the installation. Prints PASS/FAIL lines;
//! exit code 1 if anything fails.

use crate::init;

pub fn run() -> bool {
    let mut ok = true;

    // 1. hook installed in project or global settings
    let installed = [false, true].iter().any(|&global| {
        init::settings_path(global)
            .ok()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
            .and_then(|s| {
                s.pointer("/hooks/PostToolUse")
                    .and_then(|p| p.as_array())
                    .map(|entries| {
                        entries.iter().any(|e| {
                            e.pointer("/hooks/0/command")
                                .and_then(|c| c.as_str())
                                .map(init::is_stk_hook_command)
                                .unwrap_or(false)
                        })
                    })
            })
            .unwrap_or(false)
    });
    ok &= report(
        "PostToolUse hook installed (.claude/settings.json)",
        installed,
    );

    // 2. data dir writable
    let writable = std::fs::create_dir_all(".smalltoke").is_ok();
    ok &= report(".smalltoke/ writable", writable);

    // 3. rtk coexistence (informational only)
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

fn report(what: &str, pass: bool) -> bool {
    println!("{}  {what}", if pass { "PASS" } else { "FAIL" });
    pass
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
        .unwrap_or(false)
}
