//! Grep (content mode) compression: per-file match capping and cross-file
//! duplicate folding. Rows are `path:line:text` (ripgrep content format).

use crate::config::GrepCfg;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

/// Fold a content-mode grep result. `None` = not worth folding / pass through.
pub fn fold(content: &str, cfg: &GrepCfg) -> Option<String> {
    let rows: Vec<Row> = content.lines().map(Row::parse).collect();
    if rows.iter().all(|r| matches!(r, Row::Other(_))) {
        return None; // not content-mode output we understand
    }

    // identical match text appearing in many files (lockfiles, generated code)
    let mut text_files: HashMap<&str, Vec<&str>> = HashMap::new();
    for row in &rows {
        if let Row::Match { path, text, .. } = row {
            let files = text_files.entry(*text).or_default();
            if !files.contains(path) {
                files.push(*path);
            }
        }
    }
    let duped: HashMap<&str, &Vec<&str>> = text_files
        .iter()
        .filter(|(text, files)| files.len() >= DUP_FILE_MIN && !text.trim().is_empty())
        .map(|(t, f)| (*t, f))
        .collect();

    let mut out = String::new();
    let mut per_file_shown: HashMap<&str, usize> = HashMap::new();
    // BTreeMap: fold notes must emit in a deterministic order
    let mut folded_per_file: BTreeMap<&str, usize> = BTreeMap::new();
    let mut dup_emitted: Vec<&str> = Vec::new();

    for row in &rows {
        match row {
            Row::Match { path, line, text } => {
                if let Some(files) = duped.get(text) {
                    if !dup_emitted.contains(text) {
                        dup_emitted.push(text);
                        let preview: Vec<&str> = files.iter().take(3).copied().collect();
                        let _ = writeln!(
                            out,
                            "{}:{}:{}\n  [cubtoken: identical match in {} files: {}, …]",
                            path,
                            line,
                            text,
                            files.len(),
                            preview.join(", ")
                        );
                    }
                    continue;
                }
                let shown = per_file_shown.entry(*path).or_insert(0);
                if *shown < cfg.max_matches_per_file {
                    *shown += 1;
                    let _ = writeln!(out, "{path}:{line}:{text}");
                } else {
                    *folded_per_file.entry(*path).or_insert(0) += 1;
                }
            }
            Row::Other(text) => {
                let _ = writeln!(out, "{text}");
            }
        }
    }

    for (path, hidden) in folded_per_file.iter() {
        let _ = writeln!(
            out,
            "[cubtoken: +{hidden} more matches in {path} — rerun Grep with path={path} to see all]"
        );
    }

    if folded_per_file.is_empty() && dup_emitted.is_empty() {
        return None; // nothing folded
    }
    // only substitute when meaningfully smaller
    if out.chars().count() * 10 > content.chars().count() * 7 {
        return None;
    }
    Some(out)
}

const DUP_FILE_MIN: usize = 5;

enum Row<'a> {
    Match {
        path: &'a str,
        line: &'a str,
        text: &'a str,
    },
    Other(&'a str),
}

impl<'a> Row<'a> {
    /// Parse `path:line:text`; anything else (separators, headers, binary
    /// notices) passes through as `Other`.
    fn parse(row: &'a str) -> Row<'a> {
        let Some((path, rest)) = row.split_once(':') else {
            return Row::Other(row);
        };
        let Some((line, text)) = rest.split_once(':') else {
            return Row::Other(row);
        };
        if path.is_empty() || line.is_empty() || !line.bytes().all(|b| b.is_ascii_digit()) {
            return Row::Other(row);
        }
        Row::Match { path, line, text }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> GrepCfg {
        GrepCfg {
            enabled: true,
            max_matches_per_file: 5,
        }
    }

    fn lines_for_file(path: &str, n: usize) -> String {
        (1..=n)
            .map(|i| format!("{path}:{i}:fn helper_{i}() -> u32 {{ {i} }}\n"))
            .collect()
    }

    fn same_line_in_files(text: &str, n: usize) -> String {
        (1..=n)
            .map(|i| format!("pkg{i}/package.json:14:{text}\n"))
            .collect()
    }

    #[test]
    fn folds_excess_matches_per_file() {
        let raw = lines_for_file("src/big.rs", 40);
        let out = fold(&raw, &cfg()).unwrap();
        let rows = out.lines().filter(|l| l.starts_with("src/big.rs:")).count();
        assert_eq!(rows, 5);
        assert!(out.contains("+35 more matches in src/big.rs"));
        assert!(out.contains("path=src/big.rs"), "escape hatch: {out}");
    }

    #[test]
    fn dedupes_identical_lines_across_files() {
        let raw = same_line_in_files(r#"    "lodash": "^4.17.21","#, 30);
        let out = fold(&raw, &cfg()).unwrap();
        assert!(out.contains("identical match in 30 files"), "{out}");
        // the match text appears once, not 30 times
        assert_eq!(out.matches("lodash").count(), 1, "{out}");
    }

    #[test]
    fn few_matches_pass_through() {
        assert!(fold(&lines_for_file("a.rs", 3), &cfg()).is_none());
    }

    #[test]
    fn non_content_output_passes_through() {
        assert!(fold("src/a.rs\nsrc/b.rs\n", &cfg()).is_none());
    }
}
