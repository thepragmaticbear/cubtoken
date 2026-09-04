//! File text → signature skeleton via tree-sitter.
//!
//! Invariant: every line placed in `shown_lines` is the exact source line at
//! the stated 1-based line number — never paraphrased (the model may quote it
//! in an Edit). Elided regions always advertise their `[La-Lb]` range so the
//! caller can offer a precise escape hatch.

use tree_sitter::{Language, Node, Parser};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    TypeScript,
    Python,
    Go,
}

pub struct Skeleton {
    /// Skeleton text with a line-number gutter.
    pub rendered: String,
    /// (1-based line number, verbatim text) for every source line shown.
    pub shown_lines: Vec<(usize, String)>,
}

pub fn lang_for_path(path: &str) -> Option<Lang> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let (_, ext) = name.rsplit_once('.')?;
    match ext {
        "rs" => Some(Lang::Rust),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some(Lang::TypeScript),
        "py" => Some(Lang::Python),
        "go" => Some(Lang::Go),
        _ => None,
    }
}

pub fn skeleton(src: &str, lang: Lang) -> Option<Skeleton> {
    let mut parser = Parser::new();
    parser.set_language(&language(lang)).ok()?;
    let tree = parser.parse(src, None)?;

    let mut b = Builder {
        lines: src.lines().collect(),
        out: String::new(),
        shown: Vec::new(),
        decls: 0,
        lang,
        covered: None,
        open_marker: None,
    };
    b.process_children(tree.root_node(), 0);
    if b.decls == 0 {
        return None;
    }
    let total = b.lines.len();
    b.emit_gap(total);
    Some(Skeleton {
        rendered: b.out,
        shown_lines: b.shown,
    })
}

fn language(lang: Lang) -> Language {
    match lang {
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Lang::Python => tree_sitter_python::LANGUAGE.into(),
        Lang::Go => tree_sitter_go::LANGUAGE.into(),
    }
}

fn decl_kinds(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Rust => &[
            "function_item",
            "struct_item",
            "enum_item",
            "impl_item",
            "trait_item",
            "mod_item",
            "type_item",
            "const_item",
            "static_item",
            "macro_definition",
            "union_item",
            "function_signature_item",
            "associated_type",
        ],
        Lang::TypeScript => &[
            "function_declaration",
            "class_declaration",
            "abstract_class_declaration",
            "interface_declaration",
            "type_alias_declaration",
            "enum_declaration",
            "method_definition",
            "public_field_definition",
            "lexical_declaration",
            "variable_declaration",
        ],
        Lang::Python => &["function_definition", "class_definition"],
        Lang::Go => &[
            "function_declaration",
            "method_declaration",
            "type_declaration",
            "const_declaration",
            "var_declaration",
            "package_clause",
        ],
    }
}

fn import_kinds(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Rust => &["use_declaration"],
        Lang::TypeScript => &["import_statement"],
        Lang::Python => &["import_statement", "import_from_statement"],
        Lang::Go => &["import_declaration"],
    }
}

fn container_kinds(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Rust => &["impl_item", "trait_item", "mod_item"],
        Lang::TypeScript => &["class_declaration", "abstract_class_declaration"],
        Lang::Python => &["class_definition"],
        Lang::Go => &[],
    }
}

/// Wrapper kinds whose interesting declaration is a named child.
fn unwrap_decl<'t>(node: Node<'t>) -> Node<'t> {
    match node.kind() {
        "export_statement" | "decorated_definition" => {
            let mut cursor = node.walk();
            let inner = node
                .named_children(&mut cursor)
                .find(|c| !c.kind().contains("comment") && c.kind() != "decorator");
            inner.unwrap_or(node)
        }
        _ => node,
    }
}

fn body_node<'t>(node: Node<'t>) -> Option<Node<'t>> {
    if let Some(body) = node.child_by_field_name("body") {
        return Some(body);
    }
    let mut cursor = node.walk();
    let found = node.named_children(&mut cursor).find_map(body_node);
    found
}

const MAX_IMPORTS_SHOWN: usize = 10;
const GUTTER: &str = "     ";

struct Builder<'a> {
    lines: Vec<&'a str>,
    out: String,
    shown: Vec<(usize, String)>,
    decls: usize,
    lang: Lang,
    /// Highest 0-based row already shown or covered by an elision marker.
    /// Anything skipped past it must be advertised (escape-hatch invariant).
    covered: Option<usize>,
    /// While the last line written is a `… [La-Lb]` marker: its byte offset in
    /// `out` and its start line, so an adjacent gap widens it instead of
    /// stacking a second marker (closing braces would otherwise emit a run).
    open_marker: Option<(usize, usize)>,
}

impl<'a> Builder<'a> {
    fn process_children(&mut self, node: Node<'_>, depth: usize) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();

        let mut i = 0;
        while i < children.len() {
            let child = unwrap_decl(children[i]);
            let kind = child.kind();

            if import_kinds(self.lang).contains(&kind) {
                // group consecutive imports
                let start = i;
                while i < children.len()
                    && import_kinds(self.lang).contains(&unwrap_decl(children[i]).kind())
                {
                    i += 1;
                }
                self.emit_import_group(&children[start..i]);
                continue;
            }

            if decl_kinds(self.lang).contains(&kind) {
                self.emit_decl(children[i], child, depth);
            }
            i += 1;
        }
    }

    fn emit_import_group(&mut self, group: &[Node<'_>]) {
        self.decls += group.len();
        let shown = if group.len() > MAX_IMPORTS_SHOWN {
            3
        } else {
            group.len()
        };
        for node in &group[..shown] {
            self.emit_verbatim(node.start_position().row);
        }
        if shown < group.len() {
            let first_hidden = group[shown].start_position().row + 1;
            let last = group.last().unwrap().end_position().row + 1;
            self.emit_marker(&format!(
                "… +{} more imports [L{first_hidden}-L{last}]",
                group.len() - shown
            ));
            self.covered = Some(last - 1);
        }
    }

    fn emit_decl(&mut self, outer: Node<'_>, decl: Node<'_>, depth: usize) {
        self.emit_prefix_lines(outer);
        // `outer` may wrap `decl` (Python `@decorator`, TS `export`): show every
        // row from the wrapper down to the declaration's own start, so the
        // signature is never hidden behind its decorators.
        let sig = decl.start_position().row;
        let end = outer.end_position().row;
        let signature_end = body_node(decl)
            .map(|body| {
                if self.lang == Lang::Python && body.start_position().row > sig {
                    body.start_position().row - 1
                } else {
                    body.start_position().row
                }
            })
            .unwrap_or_else(|| decl.end_position().row)
            .min(end);
        for row in outer.start_position().row..=signature_end {
            self.emit_verbatim(row);
        }
        self.decls += 1;

        if depth == 0 && container_kinds(self.lang).contains(&decl.kind()) {
            if let Some(body) = decl.child_by_field_name("body") {
                self.process_children(body, depth + 1);
                return;
            }
        }
        if end > signature_end {
            self.emit_range_marker(signature_end + 2, end + 1);
            self.covered = Some(end);
        }
    }

    /// Contiguous comments *and attributes* immediately above a declaration.
    /// Rust `#[derive(...)]` / `#[cfg(...)]` are siblings of the item, not
    /// children, so they are load-bearing context that would otherwise vanish.
    fn emit_prefix_lines(&mut self, node: Node<'_>) {
        let mut rows = Vec::new();
        let mut expected_end = node.start_position().row;
        let mut prev = node.prev_named_sibling();
        while let Some(p) = prev {
            // a node ending at a newline reports the *next* row with column 0
            let p_end = if p.end_position().column == 0 {
                p.end_position().row.saturating_sub(1)
            } else {
                p.end_position().row
            };
            let attached = p.kind().contains("comment") || p.kind() == "attribute_item";
            if !attached || p_end + 1 != expected_end {
                break;
            }
            for row in (p.start_position().row..=p_end).rev() {
                rows.push(row);
            }
            expected_end = p.start_position().row;
            prev = p.prev_named_sibling();
        }
        for row in rows.into_iter().rev() {
            self.emit_verbatim(row);
        }
    }

    fn emit_verbatim(&mut self, row: usize) {
        let Some(&text) = self.lines.get(row) else {
            return;
        };
        self.emit_gap(row);
        self.out.push_str(&format!("{:>5}  {text}\n", row + 1));
        self.shown.push((row + 1, text.to_string()));
        self.covered = Some(row);
        self.open_marker = None;
    }

    /// Advertise rows skipped between the last covered row and `upto`
    /// (exclusive, 0-based). Blank-only gaps are not worth a marker.
    fn emit_gap(&mut self, upto: usize) {
        let from = self.covered.map_or(0, |c| c + 1);
        let upto = upto.min(self.lines.len());
        if from >= upto || self.lines[from..upto].iter().all(|l| l.trim().is_empty()) {
            return;
        }
        self.emit_range_marker(from + 1, upto);
        self.covered = Some(upto - 1);
    }

    fn emit_marker(&mut self, text: &str) {
        self.out.push_str(&format!("{GUTTER}  {text}\n"));
        self.open_marker = None;
    }

    /// Elision marker for lines `from..=to` (1-based). Widens the marker
    /// already at the tail of `out` when this range continues it.
    fn emit_range_marker(&mut self, from: usize, to: usize) {
        let (offset, start) = match self.open_marker {
            Some((offset, start)) => {
                self.out.truncate(offset);
                (offset, start)
            }
            None => (self.out.len(), from),
        };
        self.out
            .push_str(&format!("{GUTTER}  … [L{start}-L{to}]\n"));
        self.open_marker = Some((offset, start));
    }
}
