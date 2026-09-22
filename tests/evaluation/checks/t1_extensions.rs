//! Independent check for task t1. Copied into the task checkout after the
//! agent stops; the agent never sees it and cannot edit it.

use ctk_sitter::{lang_for_path, Lang};

#[test]
fn mts_and_cts_resolve_to_the_typescript_grammar() {
    assert!(
        matches!(lang_for_path("src/api.mts"), Some(Lang::TypeScript)),
        ".mts must resolve to the TypeScript grammar"
    );
    assert!(
        matches!(lang_for_path("src/api.cts"), Some(Lang::TypeScript)),
        ".cts must resolve to the TypeScript grammar"
    );
}

/// The prompt said "change nothing else": the extensions that already resolved
/// must still resolve, and an unknown extension must still be `None`.
#[test]
fn existing_resolution_is_unchanged() {
    for (path, expected) in [
        ("src/main.rs", Some(Lang::Rust)),
        ("app/page.tsx", Some(Lang::TypeScript)),
        ("lib/util.js", Some(Lang::TypeScript)),
        ("lib/util.mjs", Some(Lang::TypeScript)),
        ("lib/util.cjs", Some(Lang::TypeScript)),
        ("tool.py", Some(Lang::Python)),
        ("cmd/main.go", Some(Lang::Go)),
    ] {
        assert_eq!(
            lang_for_path(path).is_some(),
            expected.is_some(),
            "{path} stopped resolving"
        );
    }
    assert!(lang_for_path("notes.txt").is_none());
    assert!(lang_for_path("Makefile").is_none());
}
