//! Glob compression: long path listings → directory tree with counts.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

/// Fold a path listing. `None` = small enough or not worth folding, pass
/// through. Glob returns paths newest-first, so directories are emitted in
/// first-appearance order — sorting them would destroy that signal.
pub fn fold_paths(paths: &[String], max_paths: usize) -> Option<String> {
    if paths.len() <= max_paths {
        return None;
    }

    let mut by_dir: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    for p in paths {
        let (dir, file) = p.rsplit_once('/').unwrap_or(("", p.as_str()));
        by_dir
            .entry(dir)
            .or_insert_with(|| {
                order.push(dir);
                Vec::new()
            })
            .push(file);
    }

    let mut out = format!(
        "[cubtoken: {} paths folded into a tree. Expand any directory with \
         Glob(pattern=<dir>/**)]\n",
        paths.len()
    );
    const FULL_LISTING_MAX: usize = 5;
    for dir in &order {
        let files = &by_dir[dir];
        let label = if dir.is_empty() { "." } else { *dir };
        if files.len() <= FULL_LISTING_MAX {
            for f in files {
                let _ = writeln!(out, "{label}/{f}");
            }
        } else {
            let _ = writeln!(
                out,
                "{label}/ ({} files{})",
                files.len(),
                dominant_extensions(files)
            );
        }
    }

    // A wide, flat tree folds to roughly itself plus a banner. Substituting
    // that spends tokens to save none — the same >=30% bar read/grep/bash use.
    if out.chars().count() * 10 > paths.join("\n").chars().count() * 7 {
        return None;
    }
    Some(out)
}

/// e.g. ", mostly .tsx" — top extensions by count.
fn dominant_extensions(files: &[&str]) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for f in files {
        if let Some((_, ext)) = f.rsplit_once('.') {
            *counts.entry(ext).or_default() += 1;
        }
    }
    let mut ranked: Vec<(&str, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    match ranked.first() {
        Some((ext, n)) if *n * 2 >= files.len() => format!(", mostly .{ext}"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_large_listing_into_tree() {
        let paths: Vec<String> = (0..300)
            .map(|i| format!("src/components/C{i}.tsx"))
            .collect();
        let out = fold_paths(&paths, 50).unwrap();
        assert!(
            out.contains("src/components/ (300 files, mostly .tsx)"),
            "{out}"
        );
        assert!(
            out.contains("Glob(pattern=<dir>/**)"),
            "escape hatch: {out}"
        );
        assert!(out.lines().count() < 20, "{out}");
    }

    #[test]
    fn wide_flat_tree_passes_through_instead_of_growing() {
        // one file per directory: nothing to fold, so folding would emit the
        // same listing plus a banner. Regression: this used to substitute a
        // *larger* payload and record it as a saving.
        let paths: Vec<String> = (0..80).map(|i| format!("wide/d{i}/f.ts")).collect();
        assert!(fold_paths(&paths, 50).is_none());
    }

    #[test]
    fn directory_order_follows_input_not_the_alphabet() {
        // Glob returns newest-first; that ordering must survive folding.
        let mut paths: Vec<String> = (0..60).map(|i| format!("zzz/gen/G{i}.ts")).collect();
        paths.push("aaa/Late.ts".to_string());
        let out = fold_paths(&paths, 50).unwrap();
        let body: Vec<&str> = out.lines().skip(1).collect();
        assert!(body[0].starts_with("zzz/gen/"), "{out}");
        assert!(body[1].starts_with("aaa/"), "{out}");
    }

    #[test]
    fn small_listing_passes_through() {
        let paths: Vec<String> = (0..10).map(|i| format!("src/f{i}.rs")).collect();
        assert!(fold_paths(&paths, 50).is_none());
    }

    #[test]
    fn small_directories_listed_in_full() {
        let mut paths: Vec<String> = (0..200).map(|i| format!("src/gen/G{i}.ts")).collect();
        paths.push("src/Main.ts".to_string());
        paths.push("README.md".to_string());
        let out = fold_paths(&paths, 50).unwrap();
        assert!(out.contains("src/Main.ts"), "small dir listed fully: {out}");
        assert!(
            out.contains("./README.md") || out.contains("README.md"),
            "{out}"
        );
        assert!(out.contains("src/gen/ (200 files, mostly .ts)"), "{out}");
    }
}
