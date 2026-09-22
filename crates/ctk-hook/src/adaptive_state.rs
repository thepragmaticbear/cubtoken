//! Bounded on-disk adaptive snapshot. It never participates in correctness:
//! unreadable state simply falls back to static Read behavior.

use std::collections::{BTreeMap, HashMap};
use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::adaptive::{self, AdaptiveState, Bucket, Outcome, Recommendation};
use crate::ledger::{Confidence, Ledger};

pub const STATE_FILE: &str = "adaptive-v1.json";
static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

pub fn load(dir: &Path, now_ms: u64) -> AdaptiveState {
    let path = dir.join(STATE_FILE);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<AdaptiveState>(&raw).ok())
        .filter(|state| state.schema == 1 && state.policy_version == 1)
        .unwrap_or_else(|| AdaptiveState::empty(now_ms))
}

pub fn reset(dir: &Path, now_ms: u64) -> Result<AdaptiveState, String> {
    let state = AdaptiveState::empty(now_ms);
    store(dir, &state)?;
    Ok(state)
}

pub fn refresh(dir: &Path, now_ms: u64) -> AdaptiveState {
    let previous = load(dir, now_ms);
    let mut grouped: BTreeMap<String, Vec<Outcome>> = BTreeMap::new();
    for ledger in Ledger::load_all(dir) {
        // Group recoveries by decision in one pass. Scanning every recovery per
        // decision made this O(decisions x recoveries): at 10,000 ledger records
        // that measured 17.9 ms p95 against the 5 ms large-session target, and
        // `refresh` walks every session file in the project on each Stop.
        let mut recovered: HashMap<&str, usize> = HashMap::new();
        for recovery in ledger.recoveries() {
            if recovery.confidence != Some(Confidence::High) {
                continue;
            }
            let Some(decision_id) = recovery.decision_id.as_deref() else {
                continue;
            };
            let total = recovered.entry(decision_id).or_insert(0);
            *total = total.saturating_add(recovery.tokens);
        }
        for decision in ledger.decisions() {
            if decision.recorded_at_ms < previous.reset_at_ms {
                continue;
            }
            let recovery_tokens = recovered
                .get(decision.decision_id.as_str())
                .copied()
                .unwrap_or(0);
            grouped
                .entry(adaptive::bucket_key(
                    &decision.language,
                    decision.tokens_in,
                    &decision.strategy,
                ))
                .or_default()
                .push(Outcome {
                    recorded_at_ms: decision.recorded_at_ms,
                    gross_saved: decision.tokens_in.saturating_sub(decision.tokens_out),
                    recovery_tokens,
                });
        }
    }
    let buckets = grouped
        .into_iter()
        .map(|(key, outcomes)| {
            let outcomes = adaptive::retain_latest(outcomes);
            let recommendation = adaptive::merge_one_way(
                previous
                    .buckets
                    .get(&key)
                    .map(|bucket| bucket.recommendation)
                    .unwrap_or(Recommendation::One),
                adaptive::recommend(&outcomes),
            );
            (
                key,
                Bucket {
                    outcomes,
                    recommendation,
                },
            )
        })
        .collect();
    let state = AdaptiveState {
        schema: 1,
        policy_version: 1,
        reset_at_ms: previous.reset_at_ms,
        buckets,
    };
    let _ = store(dir, &state);
    state
}

pub fn recommendation(state: &AdaptiveState, key: &str) -> Recommendation {
    state
        .buckets
        .get(key)
        .map(|bucket| bucket.recommendation)
        .unwrap_or(Recommendation::One)
}

fn store(dir: &Path, state: &AdaptiveState) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let destination = dir.join(STATE_FILE);
    let raw = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    let (temporary, mut file) = loop {
        let nonce = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("adaptive-v1.{}.{nonce}.tmp", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => break (path, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    };
    let result = file
        .write_all(&raw)
        .and_then(|_| {
            drop(file);
            std::fs::rename(&temporary, destination)
        })
        .map_err(|error| error.to_string());
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_replaces_state_without_touching_ledgers() {
        let temp = tempfile::tempdir().unwrap();
        let ledger = temp.path().join("session-a.jsonl");
        std::fs::write(
            &ledger,
            "{\"e\":\"save\",\"tool\":\"Read\",\"in\":10,\"out\":1}\n",
        )
        .unwrap();
        let state = reset(temp.path(), 42).unwrap();
        assert_eq!(state.reset_at_ms, 42);
        assert!(ledger.exists());
        assert!(load(temp.path(), 100).buckets.is_empty());
    }

    #[test]
    fn concurrent_writers_use_independent_temporary_files() {
        let temp = tempfile::tempdir().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(32));
        let mut writers = Vec::new();
        for now_ms in 0..32 {
            let dir = temp.path().to_path_buf();
            let barrier = barrier.clone();
            writers.push(std::thread::spawn(move || {
                barrier.wait();
                reset(&dir, now_ms)
            }));
        }
        for writer in writers {
            writer.join().unwrap().unwrap();
        }
        assert_eq!(load(temp.path(), 100).schema, 1);
    }
}
