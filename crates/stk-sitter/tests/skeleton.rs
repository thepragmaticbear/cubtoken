use stk_sitter::{skeleton, Lang};

#[test]
fn rust_skeleton_snapshot() {
    let src = include_str!("corpus/sample.rs");
    let sk = skeleton(src, Lang::Rust).unwrap();
    insta::assert_snapshot!(sk.rendered);
}

#[test]
fn skeleton_lines_are_verbatim() {
    let src = include_str!("corpus/sample.rs");
    let lines: Vec<&str> = src.lines().collect();
    for (line_no, text) in skeleton(src, Lang::Rust).unwrap().shown_lines {
        assert_eq!(lines[line_no - 1], text, "line {line_no} not verbatim");
    }
}

#[test]
fn skeleton_is_much_smaller_than_source() {
    let src = include_str!("corpus/sample.rs");
    let sk = skeleton(src, Lang::Rust).unwrap();
    assert!(
        sk.rendered.len() < src.len() * 6 / 10,
        "skeleton {} bytes vs source {} bytes",
        sk.rendered.len(),
        src.len()
    );
}

#[test]
fn elided_ranges_are_advertised() {
    let src = include_str!("corpus/sample.rs");
    let sk = skeleton(src, Lang::Rust).unwrap();
    // multi-line bodies must advertise their [La-Lb] range so the model can
    // Read(offset, limit) them — the escape-hatch invariant
    assert!(
        sk.rendered.contains("[L"),
        "no elision ranges in:\n{}",
        sk.rendered
    );
}

#[test]
fn unknown_language_returns_none() {
    assert!(stk_sitter::lang_for_path("file.xyz").is_none());
    assert!(stk_sitter::lang_for_path("Makefile").is_none());
}

#[test]
fn known_extensions_resolve() {
    use stk_sitter::lang_for_path as l;
    assert!(matches!(l("src/main.rs"), Some(Lang::Rust)));
    assert!(matches!(l("app/page.tsx"), Some(Lang::TypeScript)));
    assert!(matches!(l("lib/util.js"), Some(Lang::TypeScript)));
    assert!(matches!(l("tool.py"), Some(Lang::Python)));
    assert!(matches!(l("cmd/main.go"), Some(Lang::Go)));
}

#[test]
fn garbage_input_returns_none() {
    // binary-ish garbage: no declarations → None so caller falls back
    let garbage = "\u{0}\u{1}\u{2} not really code at all ;;;;";
    assert!(skeleton(garbage, Lang::Rust).is_none());
}
