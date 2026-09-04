use ctk_sitter::{skeleton, Lang};

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
    assert!(ctk_sitter::lang_for_path("file.xyz").is_none());
    assert!(ctk_sitter::lang_for_path("Makefile").is_none());
}

#[test]
fn known_extensions_resolve() {
    use ctk_sitter::lang_for_path as l;
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

/// The escape-hatch invariant, stated generally: every non-blank source line
/// is either shown verbatim or falls inside an advertised `[La-Lb]` range.
/// Silently dropped content (Rust attributes, Python class bodies, trailing
/// closing braces) is what this catches.
fn assert_fully_advertised(src: &str, lang: Lang) {
    let sk = skeleton(src, lang).unwrap();
    let mut covered: Vec<bool> = vec![false; src.lines().count() + 1];
    for (n, _) in &sk.shown_lines {
        covered[*n] = true;
    }
    for line in sk.rendered.lines() {
        let Some((_, range)) = line.rsplit_once("[L") else {
            continue;
        };
        let Some((a, b)) = range.trim_end_matches(']').split_once("-L") else {
            continue;
        };
        let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>()) else {
            continue;
        };
        for n in a..=b.min(covered.len() - 1) {
            covered[n] = true;
        }
    }
    for (i, text) in src.lines().enumerate() {
        assert!(
            covered[i + 1] || text.trim().is_empty(),
            "line {} is neither shown nor advertised: {text:?}\n{}",
            i + 1,
            sk.rendered
        );
    }
}

#[test]
fn rust_source_has_no_unadvertised_gaps() {
    assert_fully_advertised(include_str!("corpus/sample.rs"), Lang::Rust);
}

#[test]
fn rust_attributes_are_shown() {
    let src = "#[derive(Debug, Clone)]\n#[serde(rename_all = \"camelCase\")]\n/// Doc.\npub struct C {\n    a: u32,\n}\n\n#[cfg(target_os = \"linux\")]\npub fn only_linux() -> u32 {\n    1\n}\n";
    let sk = skeleton(src, Lang::Rust).unwrap();
    for want in [
        "#[derive(Debug, Clone)]",
        "#[serde(rename_all",
        "#[cfg(target_os",
        "pub struct C {",
        "pub fn only_linux",
    ] {
        assert!(
            sk.rendered.contains(want),
            "missing {want}:\n{}",
            sk.rendered
        );
    }
    assert_fully_advertised(src, Lang::Rust);
}

#[test]
fn python_decorated_definitions_keep_their_signature() {
    let src = "@dataclass(frozen=True)\n@register(\"thing\")\nclass Thing:\n    a: int\n\n@app.route(\"/x\")\ndef handler(req):\n    x = 1\n    return x\n";
    let sk = skeleton(src, Lang::Python).unwrap();
    // the decorator alone is not a signature — both must survive
    for want in [
        "@dataclass(frozen=True)",
        "class Thing:",
        "@app.route",
        "def handler(req):",
    ] {
        assert!(
            sk.rendered.contains(want),
            "missing {want}:\n{}",
            sk.rendered
        );
    }
    assert_fully_advertised(src, Lang::Python);
}

#[test]
fn typescript_exports_have_no_unadvertised_gaps() {
    let src = "import React from \"react\";\n\nexport default function App() {\n  const a = 1;\n  return a;\n}\n\nexport const helper = (x: number) => {\n  return x * 2;\n};\n";
    assert_fully_advertised(src, Lang::TypeScript);
}

#[test]
fn multiline_signatures_are_preserved_across_languages() {
    let cases = [
        (
            Lang::Rust,
            "pub fn create_user(\n    name: String,\n    permissions: Permissions,\n) -> Result<User> {\n    todo!()\n}\n",
            "permissions: Permissions",
        ),
        (
            Lang::TypeScript,
            "export function createUser(\n  name: string,\n  permissions: Permissions,\n): Promise<User> {\n  throw new Error();\n}\n",
            "permissions: Permissions",
        ),
        (
            Lang::Python,
            "def create_user(\n    name: str,\n    permissions: Permissions,\n) -> User:\n    raise NotImplementedError\n",
            "permissions: Permissions",
        ),
        (
            Lang::Go,
            "func CreateUser(\n    name string,\n    permissions Permissions,\n) (User, error) {\n    panic(\"todo\")\n}\n",
            "permissions Permissions",
        ),
    ];
    for (lang, src, expected) in cases {
        let rendered = skeleton(src, lang).unwrap().rendered;
        assert!(rendered.contains(expected), "{lang:?}:\n{rendered}");
        assert_fully_advertised(src, lang);
    }
}
