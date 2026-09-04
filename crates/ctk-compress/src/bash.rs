//! Minimal, command-agnostic Bash stdout noise strip (opt-in; rtk owns
//! command-specific compression). Never applied to failing commands —
//! gating happens in the dispatcher.

/// Strip ANSI codes, flatten carriage-return progress lines, and collapse
/// blank runs. `None` = nothing worth stripping.
pub fn strip(stdout: &str) -> Option<String> {
    let no_ansi = strip_ansi(stdout);

    // for each \r-overwritten line, only the final segment is visible
    let visible: String = no_ansi
        .split('\n')
        .map(|line| {
            line.rsplit('\r')
                .find(|segment| !segment.is_empty())
                .unwrap_or(line)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let collapsed = collapse_blank_runs(&visible);

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

fn collapse_blank_runs(s: &str) -> String {
    let mut out = Vec::new();
    let mut previous_blank = false;
    for line in s.lines() {
        let blank = line.trim().is_empty();
        if !blank || !previous_blank {
            out.push(line);
        }
        previous_blank = blank;
    }
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
    fn distinct_numbered_lines_are_never_folded() {
        let output = (1..=50)
            .map(|i| format!("test {i} passed: case_{i}\n"))
            .collect::<String>();
        assert!(strip(&output).is_none(), "clean output must pass through");
    }

    #[test]
    fn carriage_return_progress_keeps_final_visible_state() {
        let noisy = format!(
            "{}\ndone\n",
            (0..100)
                .map(|i| format!("Downloading {i}%\r"))
                .collect::<String>()
        );
        let out = strip(&noisy).unwrap();
        assert!(out.contains("Downloading 99%"), "{out}");
        assert!(out.contains("done"), "{out}");
    }

    #[test]
    fn small_clean_output_passes_through() {
        assert!(strip("total 8\n-rw-r--r-- 1 u w 6 f.txt\n").is_none());
    }
}
