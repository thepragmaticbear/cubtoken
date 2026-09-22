//! Pure causal checks for recovery accounting. A result is only trainable when
//! it is unambiguous; lower-confidence candidates remain visible in reporting.

use crate::ledger::{CompressionDecision, Confidence, ElidedRange, RecoveryKind};
use serde_json::Value;

pub fn returned_range(tool_input: &Value, tool_response: &Value) -> Option<ElidedRange> {
    let start = tool_response
        .pointer("/file/startLine")
        .and_then(Value::as_u64)
        .or_else(|| tool_input.get("offset").and_then(Value::as_u64))? as usize;
    let count = tool_response
        .pointer("/file/numLines")
        .and_then(Value::as_u64)
        .or_else(|| tool_input.get("limit").and_then(Value::as_u64))? as usize;
    (count > 0).then_some(ElidedRange {
        start_line: start,
        end_line: start.saturating_add(count.saturating_sub(1)),
    })
}

pub fn overlaps_elision(returned: &ElidedRange, elided: &[ElidedRange]) -> bool {
    elided
        .iter()
        .any(|range| returned.start_line <= range.end_line && range.start_line <= returned.end_line)
}

pub fn classify_targeted(
    decision: &CompressionDecision,
    turn: u64,
    batch: u64,
    fingerprint_matches: bool,
    invalidated: bool,
    returned: &ElidedRange,
) -> Option<Confidence> {
    if invalidated || !overlaps_elision(returned, &decision.elided_ranges) {
        return None;
    }
    if turn == decision.turn && batch <= decision.batch {
        // Independent concurrent/same-batch calls are not recoveries.
        return None;
    }
    if turn == decision.turn && batch > decision.batch && fingerprint_matches {
        return Some(Confidence::High);
    }
    if turn == decision.turn.saturating_add(1) && fingerprint_matches {
        return Some(Confidence::Medium);
    }
    Some(Confidence::Low)
}

pub fn classify_full_repeat(
    decision: &CompressionDecision,
    turn: u64,
    batch: u64,
    fingerprint_matches: bool,
    invalidated: bool,
) -> Option<Confidence> {
    if invalidated || (turn == decision.turn && batch <= decision.batch) {
        return None;
    }
    if turn == decision.turn && batch > decision.batch && fingerprint_matches {
        return Some(Confidence::High);
    }
    if turn == decision.turn.saturating_add(1) && fingerprint_matches {
        return Some(Confidence::Medium);
    }
    Some(Confidence::Low)
}

pub fn recovery_kind(is_targeted: bool) -> RecoveryKind {
    if is_targeted {
        RecoveryKind::Targeted
    } else {
        RecoveryKind::FullRepeat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision() -> CompressionDecision {
        CompressionDecision {
            decision_id: "d".to_string(),
            turn: 2,
            batch: 4,
            sequence: 1,
            recorded_at_ms: 1,
            file_identity: "/a.rs".to_string(),
            content_fingerprint: "v1".to_string(),
            language: "rust".to_string(),
            strategy: "skeleton".to_string(),
            profile: "default".to_string(),
            tokens_in: 8000,
            tokens_out: 1000,
            elided_ranges: vec![ElidedRange {
                start_line: 20,
                end_line: 40,
            }],
        }
    }

    #[test]
    fn same_batch_targeted_read_is_not_attributed() {
        let range = ElidedRange {
            start_line: 30,
            end_line: 31,
        };
        assert_eq!(
            classify_targeted(&decision(), 2, 4, true, false, &range),
            None
        );
    }

    #[test]
    fn later_batch_overlap_is_high_confidence() {
        let range = ElidedRange {
            start_line: 30,
            end_line: 31,
        };
        assert_eq!(
            classify_targeted(&decision(), 2, 5, true, false, &range),
            Some(Confidence::High)
        );
    }

    /// The backstop behind `is_protected`. In production `dispatch` returns
    /// before attribution for any edited file, so this is unreachable today —
    /// but it is what keeps an edited file out of policy training if that
    /// gate is ever relaxed, so its contract is pinned here directly.
    #[test]
    fn an_intervening_edit_is_never_trainable() {
        let range = ElidedRange {
            start_line: 30,
            end_line: 31,
        };
        // Everything else says High: later batch, overlapping, unchanged.
        assert_eq!(
            classify_targeted(&decision(), 2, 5, true, true, &range),
            None
        );
        assert_eq!(classify_full_repeat(&decision(), 2, 5, true, true), None);
        // Sanity: the same inputs without the edit are High.
        assert_eq!(
            classify_targeted(&decision(), 2, 5, true, false, &range),
            Some(Confidence::High)
        );
        assert_eq!(
            classify_full_repeat(&decision(), 2, 5, true, false),
            Some(Confidence::High)
        );
    }

    #[test]
    fn changed_or_non_overlapping_content_is_not_trainable() {
        let range = ElidedRange {
            start_line: 5,
            end_line: 6,
        };
        assert_eq!(
            classify_targeted(&decision(), 2, 5, true, false, &range),
            None
        );
        let overlap = ElidedRange {
            start_line: 30,
            end_line: 31,
        };
        assert_eq!(
            classify_targeted(&decision(), 2, 5, false, false, &overlap),
            Some(Confidence::Low)
        );
    }
}
