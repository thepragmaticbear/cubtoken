//! Minimal, command-agnostic Bash stdout noise strip (opt-in; rtk owns
//! command-specific compression). Never applied to failing commands —
//! gating happens in the dispatcher.

/// Strip ANSI codes, flatten carriage-return progress lines, collapse
/// near-duplicate runs and blank runs. `None` = nothing worth stripping.
pub fn strip(stdout: &str) -> Option<String> {
    let no_ansi = strip_ansi(stdout);

    // for each \r-overwritten line, only the final segment is visible
    let visible: String = no_ansi
        .split('\n')
        .map(|line| line.rsplit('\r').next().unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");

    let collapsed = collapse_repeats(&visible);

    if collapsed.chars().count() * 10 > stdout.chars().count() * 7 {
        return None; // not meaningfully smaller
    }
    Some(collapsed)
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // CSI sequence: ESC [ ... final byte 0x40-0x7e
        if chars.peek() == Some(&'[') {
            chars.next();
            for f in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&f) {
                    break;
                }
            }
        }
    }
    out
}

/// Lines sharing the same prefix-before-first-digit (progress counters,
/// download lines) collapse to the last one + a repeat note; blank runs
/// collapse to one blank line.
fn collapse_repeats(s: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut run_key: Option<String> = None;
    let mut run_last = String::new();
    let mut run_len = 0usize;

    let flush =
        |out: &mut Vec<String>, key: &mut Option<String>, last: &mut String, len: &mut usize| {
            if *len > 0 {
                if *len > 2 {
                    out.push(format!("{last}  (repeated x{len})"));
                } else {
                    out.push(last.clone());
                }
            }
            *key = None;
            *len = 0;
            last.clear();
        };

    for line in s.lines() {
        if line.trim().is_empty() {
            flush(&mut out, &mut run_key, &mut run_last, &mut run_len);
            if out.last().map(|l| !l.is_empty()).unwrap_or(false) {
                out.push(String::new());
            }
            continue;
        }
        let key: String = line
            .split(|c: char| c.is_ascii_digit())
            .next()
            .unwrap_or(line)
            .to_string();
        match &run_key {
            Some(k) if *k == key && !key.trim().is_empty() => {
                run_last = line.to_string();
                run_len += 1;
            }
            _ => {
                flush(&mut out, &mut run_key, &mut run_last, &mut run_len);
                run_key = Some(key);
                run_last = line.to_string();
                run_len = 1;
            }
        }
    }
    flush(&mut out, &mut run_key, &mut run_last, &mut run_len);
    let mut joined = out.join("\n");
    joined.push('\n');
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi_and_collapses_progress_lines() {
        let noisy = "\x1b[32mok\x1b[0m\n".to_string()
            + &(0..200)
                .map(|i| format!("Downloading [{}%]\r", i / 2))
                .collect::<String>()
            + "\ndone\n";
        let out = strip(&noisy).unwrap();
        assert!(!out.contains('\u{1b}'), "{out:?}");
        assert!(out.lines().count() < 10, "{out}");
        assert!(out.contains("done"), "{out}");
        assert!(out.contains("ok"), "{out}");
    }

    #[test]
    fn repeated_counter_lines_collapse() {
        let noisy: String = (1..=50)
            .map(|i| format!("Compiling crate {i} of 50\n"))
            .collect();
        let out = strip(&noisy).unwrap();
        assert!(out.contains("(repeated x50)"), "{out}");
        assert!(
            out.contains("Compiling crate 50 of 50"),
            "keeps last: {out}"
        );
    }

    #[test]
    fn small_clean_output_passes_through() {
        assert!(strip("total 8\n-rw-r--r-- 1 u w 6 f.txt\n").is_none());
    }
}
