//! Read tool compression: large file contents → signature skeleton.
//!
//! Pass-through (`None`) whenever: disabled, targeted read (offset/limit —
//! the model is drilling in, likely to quote text for an Edit), excluded
//! path, under threshold, or compression isn't worth it (<30% smaller).

use crate::config::Config;
use crate::estimate::est_tokens;
use serde_json::Value;

pub struct ReadOutcome {
    pub updated_response: Value,
    pub tokens_in: usize,
    pub tokens_out: usize,
    pub metadata: ReadMetadata,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReadMetadata {
    pub content_fingerprint: String,
    pub language: String,
    pub strategy: String,
    pub elided_ranges: Vec<ctk_sitter::LineRange>,
}

#[derive(Debug, Clone)]
pub struct ReadPreview {
    pub tokens_in: usize,
    pub language: String,
    pub strategy: String,
}

pub fn compress_read(
    tool_input: &Value,
    tool_response: &Value,
    cfg: &Config,
) -> Option<ReadOutcome> {
    compress_read_with_threshold(tool_input, tool_response, cfg, cfg.read.threshold_tokens)
}

/// Metadata used to select an adaptive threshold before compression.
pub fn preview_read(tool_input: &Value, tool_response: &Value) -> Option<ReadPreview> {
    let file_path = tool_input.get("file_path")?.as_str()?;
    let content = extract_content(tool_response)?;
    Some(preview_content(file_path, content))
}

pub fn preview_content(file_path: &str, content: &str) -> ReadPreview {
    let view = read_view(content, file_path);
    ReadPreview {
        tokens_in: est_tokens(content),
        language: view.language,
        strategy: view.strategy,
    }
}

/// A Read that passed the cheap guards, sized but **not yet parsed**.
///
/// Splitting this from the parse is what lets safe mode pick a threshold and
/// compress with a single tree-sitter pass. It also keeps the old property
/// that a Read under the threshold is never parsed at all.
pub struct ReadCandidate<'a> {
    file_path: &'a str,
    content: &'a str,
    tokens_in: usize,
}

impl<'a> ReadCandidate<'a> {
    pub fn tokens_in(&self) -> usize {
        self.tokens_in
    }

    /// Runs the tree-sitter parse. Call once; hand the result to `compress`.
    pub fn parse(self) -> ParsedRead<'a> {
        let view = read_view(self.content, self.file_path);
        ParsedRead {
            file_path: self.file_path,
            content: self.content,
            tokens_in: self.tokens_in,
            view,
        }
    }
}

/// A parsed Read: the view the compressor needs and the metadata the adaptive
/// policy buckets on, from one parse.
pub struct ParsedRead<'a> {
    file_path: &'a str,
    content: &'a str,
    tokens_in: usize,
    view: ReadView,
}

impl ParsedRead<'_> {
    pub fn tokens_in(&self) -> usize {
        self.tokens_in
    }

    pub fn language(&self) -> &str {
        &self.view.language
    }

    pub fn strategy(&self) -> &str {
        &self.view.strategy
    }

    /// Build the replacement response, reusing the parse this value holds.
    pub fn compress(self, tool_response: &Value, threshold_tokens: usize) -> Option<ReadOutcome> {
        if self.tokens_in <= threshold_tokens {
            return None;
        }
        let file_path = self.file_path;
        let content = self.content;
        let compressed = format!(
            "[cubtoken: compressed view of {file_path} — {} chars → skeleton. \
             This is NOT the full file. The Read tool adds its own sequential \
             numbering down the left edge of this block; ignore it. The real file \
             line numbers are the ones in this view, and bracketed [La-Lb] ranges \
             mark elided lines — to see any of them run Read(file_path={file_path}, \
             offset=<first line>, limit=<line count>). Before quoting or editing \
             this file, Read the exact target region first.]\n\n{}",
            content.chars().count(),
            self.view.rendered
        );

        // not worth substituting unless meaningfully smaller
        if compressed.chars().count() * 10 > content.chars().count() * 7 {
            return None;
        }
        let tokens_out = est_tokens(&compressed);
        Some(ReadOutcome {
            updated_response: rebuild_response(tool_response, &compressed),
            tokens_in: self.tokens_in,
            tokens_out,
            metadata: ReadMetadata {
                content_fingerprint: content_fingerprint(content),
                language: self.view.language,
                strategy: self.view.strategy,
                elided_ranges: self.view.elided_ranges,
            },
        })
    }
}

/// The cheap guards and sizing, with no parse. `None` means pass through:
/// disabled, targeted (offset/limit), excluded path, or no extractable content.
pub fn read_candidate<'a>(
    tool_input: &'a Value,
    tool_response: &'a Value,
    cfg: &Config,
) -> Option<ReadCandidate<'a>> {
    if !cfg.read.enabled {
        return None;
    }
    let file_path = tool_input.get("file_path")?.as_str()?;
    if tool_input.get("offset").is_some() || tool_input.get("limit").is_some() {
        return None;
    }
    if cfg.read.is_excluded(file_path) {
        return None;
    }
    let content = extract_content(tool_response)?;
    Some(ReadCandidate {
        file_path,
        content,
        tokens_in: est_tokens(content),
    })
}

/// Compress with a caller-selected threshold. The threshold can only make a
/// Read pass through; it never changes global configuration or representation.
pub fn compress_read_with_threshold(
    tool_input: &Value,
    tool_response: &Value,
    cfg: &Config,
    threshold_tokens: usize,
) -> Option<ReadOutcome> {
    let candidate = read_candidate(tool_input, tool_response, cfg)?;
    // Threshold before parse: an under-threshold Read is never parsed.
    if candidate.tokens_in() <= threshold_tokens {
        return None;
    }
    candidate.parse().compress(tool_response, threshold_tokens)
}

fn extract_content(tool_response: &Value) -> Option<&str> {
    if let Some(c) = tool_response
        .pointer("/file/content")
        .and_then(Value::as_str)
    {
        return Some(c);
    }
    tool_response.as_str()
}

pub fn response_tokens(tool_response: &Value) -> usize {
    extract_content(tool_response).map(est_tokens).unwrap_or(0)
}

fn rebuild_response(tool_response: &Value, compressed: &str) -> Value {
    let mut updated = tool_response.clone();
    if updated.pointer("/file/content").is_some() {
        updated["file"]["content"] = Value::String(compressed.to_string());
        updated["file"]["numLines"] = Value::from(compressed.lines().count());
    } else {
        updated = Value::String(compressed.to_string());
    }
    updated
}

struct ReadView {
    rendered: String,
    language: String,
    strategy: String,
    elided_ranges: Vec<ctk_sitter::LineRange>,
}

fn read_view(content: &str, file_path: &str) -> ReadView {
    skeleton_view(content, file_path).unwrap_or_else(|| head_tail_view(content))
}

fn skeleton_view(content: &str, file_path: &str) -> Option<ReadView> {
    let lang = ctk_sitter::lang_for_path(file_path)?;
    let skeleton = ctk_sitter::skeleton(content, lang)?;
    Some(ReadView {
        rendered: skeleton.rendered,
        language: format!("{lang:?}").to_lowercase(),
        strategy: "skeleton".to_string(),
        elided_ranges: skeleton.elided_ranges,
    })
}

const HEAD_LINES: usize = 40;
const TAIL_LINES: usize = 20;

/// Unknown language / unparseable: keep head and tail with line numbers.
fn head_tail_view(content: &str) -> ReadView {
    let lines: Vec<&str> = content.lines().collect();
    let mut out = String::new();
    let mut elided_ranges = Vec::new();
    for (i, line) in lines.iter().take(HEAD_LINES).enumerate() {
        out.push_str(&format!("{:>5}  {line}\n", i + 1));
    }
    if lines.len() > HEAD_LINES + TAIL_LINES {
        elided_ranges.push(ctk_sitter::LineRange {
            start_line: HEAD_LINES + 1,
            end_line: lines.len() - TAIL_LINES,
        });
        out.push_str(&format!(
            "       … [L{}-L{} elided] …\n",
            HEAD_LINES + 1,
            lines.len() - TAIL_LINES
        ));
    }
    if lines.len() > HEAD_LINES {
        let tail_start = lines.len().saturating_sub(TAIL_LINES).max(HEAD_LINES);
        for (i, line) in lines.iter().enumerate().skip(tail_start) {
            out.push_str(&format!("{:>5}  {line}\n", i + 1));
        }
    }
    ReadView {
        rendered: out,
        language: "other".to_string(),
        strategy: "head_tail".to_string(),
        elided_ranges,
    }
}

/// Versioned FNV-1a fingerprint. It is deterministic, contains no source
/// content, and leaves room for a future algorithm migration.
pub fn content_fingerprint(content: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SAMPLE: &str = include_str!("../../ctk-sitter/tests/corpus/sample.rs");

    /// The corpus file alone is ~3.5KB — small enough that the fixed-size
    /// banner eats the whole 30% savings margin and `compress_read` correctly
    /// declines. These tests are about skeleton *content*, so give them an
    /// input the size of a file that would really cross the threshold.
    fn big_sample() -> String {
        SAMPLE.repeat(4)
    }

    fn low_threshold() -> Config {
        Config::load_from(
            None,
            Some("[read]\nthreshold_tokens = 100\nnever_compress = []"),
        )
    }

    fn payload(path: &str, content: &str) -> (Value, Value) {
        (
            json!({"file_path": path}),
            json!({"type": "text", "file": {
                "filePath": path, "content": content,
                "numLines": content.lines().count(),
                "startLine": 1, "totalLines": content.lines().count()
            }}),
        )
    }

    #[test]
    fn skeleton_lines_are_verbatim_source_lines() {
        let src = big_sample();
        let (input, response) = payload("/repo/src/sample.rs", &src);
        let out = compress_read(&input, &response, &low_threshold()).unwrap();
        let compressed = out.updated_response["file"]["content"].as_str().unwrap();
        let source_lines: Vec<&str> = src.lines().collect();
        let mut checked = 0;
        for line in compressed.lines() {
            // gutter format: right-aligned number, two spaces, verbatim text
            let trimmed = line.trim_start();
            let Some((num, rest)) = trimmed.split_once("  ") else {
                continue;
            };
            let Ok(n) = num.parse::<usize>() else {
                continue;
            };
            assert_eq!(source_lines[n - 1], rest, "line {n} not verbatim");
            checked += 1;
        }
        assert!(checked > 10, "too few verbatim lines checked: {checked}");
    }

    #[test]
    fn exactly_one_banner() {
        let src = big_sample();
        let (input, response) = payload("/repo/src/sample.rs", &src);
        let out = compress_read(&input, &response, &low_threshold()).unwrap();
        let compressed = out.updated_response["file"]["content"].as_str().unwrap();
        assert_eq!(compressed.matches("[cubtoken:").count(), 1);
    }

    #[test]
    fn unknown_language_falls_back_to_head_tail() {
        let body: String = (1..=200).map(|i| format!("line number {i}\n")).collect();
        let (input, response) = payload("/repo/data.unknownext", &body);
        let out = compress_read(&input, &response, &low_threshold()).unwrap();
        let compressed = out.updated_response["file"]["content"].as_str().unwrap();
        assert!(compressed.contains("line number 1\n"), "head present");
        assert!(compressed.contains("line number 200"), "tail present");
        assert!(compressed.contains("elided"), "elision marker present");
        assert!(out.tokens_out < out.tokens_in);
    }

    #[test]
    fn preview_uses_the_parser_fallback_representation() {
        let body: String = (1..=200)
            .map(|i| format!("// line {i} {}\n", "x".repeat(80)))
            .collect();
        let (input, response) = payload("/repo/comments.rs", &body);
        let preview = preview_read(&input, &response).unwrap();
        let outcome = compress_read(&input, &response, &low_threshold()).unwrap();
        assert_eq!(preview.language, outcome.metadata.language);
        assert_eq!(preview.strategy, outcome.metadata.strategy);
        assert_eq!(preview.strategy, "head_tail");
    }

    #[test]
    fn tokens_accounted() {
        let src = big_sample();
        let (input, response) = payload("/repo/src/sample.rs", &src);
        let out = compress_read(&input, &response, &low_threshold()).unwrap();
        assert!(out.tokens_out < out.tokens_in);
        assert!(out.tokens_in > 100);
    }
}
