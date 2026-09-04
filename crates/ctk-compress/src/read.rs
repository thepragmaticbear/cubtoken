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
}

pub fn compress_read(
    tool_input: &Value,
    tool_response: &Value,
    cfg: &Config,
) -> Option<ReadOutcome> {
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
    let tokens_in = est_tokens(content);
    if tokens_in <= cfg.read.threshold_tokens {
        return None;
    }

    let view = skeleton_view(content, file_path).unwrap_or_else(|| head_tail_view(content));
    let compressed = format!(
        "[cubtoken: compressed view of {file_path} — {} chars → skeleton. \
         This is NOT the full file. The Read tool adds its own sequential \
         numbering down the left edge of this block; ignore it. The real file \
         line numbers are the ones in this view, and bracketed [La-Lb] ranges \
         mark elided lines — to see any of them run Read(file_path={file_path}, \
         offset=<first line>, limit=<line count>). Before quoting or editing \
         this file, Read the exact target region first.]\n\n{view}",
        content.chars().count()
    );

    // not worth substituting unless meaningfully smaller
    if compressed.chars().count() * 10 > content.chars().count() * 7 {
        return None;
    }
    let tokens_out = est_tokens(&compressed);
    Some(ReadOutcome {
        updated_response: rebuild_response(tool_response, &compressed),
        tokens_in,
        tokens_out,
    })
}

/// The Read tool_response shape (recorded fixture, 2026-06):
/// `{type:"text", file:{filePath, content, numLines, startLine, totalLines}}`.
/// Fallback: a bare string response.
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

fn skeleton_view(content: &str, file_path: &str) -> Option<String> {
    let lang = ctk_sitter::lang_for_path(file_path)?;
    Some(ctk_sitter::skeleton(content, lang)?.rendered)
}

const HEAD_LINES: usize = 40;
const TAIL_LINES: usize = 20;

/// Unknown language / unparseable: keep head and tail with line numbers.
fn head_tail_view(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut out = String::new();
    for (i, line) in lines.iter().take(HEAD_LINES).enumerate() {
        out.push_str(&format!("{:>5}  {line}\n", i + 1));
    }
    if lines.len() > HEAD_LINES + TAIL_LINES {
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
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SAMPLE: &str = include_str!("../../ctk-sitter/tests/corpus/sample.rs");

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
        let (input, response) = payload("/repo/src/sample.rs", SAMPLE);
        let out = compress_read(&input, &response, &low_threshold()).unwrap();
        let compressed = out.updated_response["file"]["content"].as_str().unwrap();
        let source_lines: Vec<&str> = SAMPLE.lines().collect();
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
        let (input, response) = payload("/repo/src/sample.rs", SAMPLE);
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
    fn tokens_accounted() {
        let (input, response) = payload("/repo/src/sample.rs", SAMPLE);
        let out = compress_read(&input, &response, &low_threshold()).unwrap();
        assert!(out.tokens_out < out.tokens_in);
        assert!(out.tokens_in > 100);
    }
}
