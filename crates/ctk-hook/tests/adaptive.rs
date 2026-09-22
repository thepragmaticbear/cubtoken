use ctk_hook::ledger::{Confidence, Ledger};
use ctk_hook::{run_hook, Config};

fn source() -> String {
    format!(
        "pub fn large() {{\n{}\n}}\n",
        "    let value = 42;\n".repeat(1_500)
    )
}

fn full_read(path: &std::path::Path, cwd: &std::path::Path, session: &str) -> serde_json::Value {
    let content = std::fs::read_to_string(path).unwrap();
    let lines = content.lines().count();
    serde_json::json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Read",
        "tool_use_id": "first-read",
        "session_id": session,
        "cwd": cwd,
        "tool_input": {"file_path": path},
        "tool_response": {"type": "text", "file": {
            "filePath": path, "content": content, "numLines": lines,
            "startLine": 1, "totalLines": lines
        }}
    })
}

fn start_turn(cwd: &std::path::Path, session: &str) -> String {
    serde_json::json!({"hook_event_name": "UserPromptSubmit", "cwd": cwd, "session_id": session})
        .to_string()
}

#[test]
fn same_batch_read_is_not_high_confidence_but_later_batch_overlap_is() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    let cfg = Config::default();

    let same_batch = "same-batch";
    run_hook(&start_turn(temp.path(), same_batch), &cfg);
    assert!(run_hook(&full_read(&path, temp.path(), same_batch).to_string(), &cfg).is_some());
    let ledger = Ledger::open(&temp.path().join(".cubtoken"), same_batch);
    let decision = ledger.decisions().last().unwrap();
    let range = decision.elided_ranges.first().unwrap();
    let targeted = serde_json::json!({
        "hook_event_name": "PostToolUse", "tool_name": "Read", "session_id": same_batch,
        "cwd": temp.path(), "tool_input": {"file_path": path, "offset": range.start_line, "limit": 1},
        "tool_response": {"file": {"filePath": path, "content": "    let value = 42;\n", "startLine": range.start_line, "numLines": 1}}
    });
    assert!(run_hook(&targeted.to_string(), &cfg).is_none());
    let ledger = Ledger::open(&temp.path().join(".cubtoken"), same_batch);
    assert!(!ledger
        .recoveries()
        .iter()
        .any(|recovery| recovery.confidence == Some(Confidence::High)));

    let later_batch = "later-batch";
    run_hook(&start_turn(temp.path(), later_batch), &cfg);
    assert!(run_hook(
        &full_read(&path, temp.path(), later_batch).to_string(),
        &cfg
    )
    .is_some());
    run_hook(&serde_json::json!({"hook_event_name": "PostToolBatch", "cwd": temp.path(), "session_id": later_batch}).to_string(), &cfg);
    assert!(run_hook(&targeted.to_string().replace(same_batch, later_batch), &cfg).is_none());
    let ledger = Ledger::open(&temp.path().join(".cubtoken"), later_batch);
    assert_eq!(
        ledger.recoveries().last().unwrap().confidence,
        Some(Confidence::High)
    );
}

#[test]
fn changed_content_and_intervening_edits_cannot_train_policy() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    let session = "changed";
    let cfg = Config::default();
    run_hook(&start_turn(temp.path(), session), &cfg);
    assert!(run_hook(&full_read(&path, temp.path(), session).to_string(), &cfg).is_some());
    let decision = Ledger::open(&temp.path().join(".cubtoken"), session)
        .decisions()
        .last()
        .cloned()
        .unwrap();
    run_hook(&serde_json::json!({"hook_event_name": "PostToolBatch", "cwd": temp.path(), "session_id": session}).to_string(), &cfg);
    std::fs::write(&path, format!("{}// changed\n", source())).unwrap();
    let targeted = serde_json::json!({
        "hook_event_name": "PostToolUse", "tool_name": "Read", "session_id": session,
        "cwd": temp.path(), "tool_input": {"file_path": path, "offset": decision.elided_ranges[0].start_line, "limit": 1},
        "tool_response": {"file": {"filePath": path, "content": "    let value = 42;\n", "startLine": decision.elided_ranges[0].start_line, "numLines": 1}}
    });
    run_hook(&targeted.to_string(), &cfg);
    let ledger = Ledger::open(&temp.path().join(".cubtoken"), session);
    assert_eq!(
        ledger.recoveries().last().unwrap().confidence,
        Some(Confidence::Low)
    );

    let edit = serde_json::json!({
        "hook_event_name": "PostToolUse", "tool_name": "Edit", "session_id": session,
        "cwd": temp.path(), "tool_input": {"file_path": path}, "tool_response": {}
    });
    run_hook(&edit.to_string(), &cfg);
    run_hook(&targeted.to_string(), &cfg);
    let ledger = Ledger::open(&temp.path().join(".cubtoken"), session);
    assert!(!ledger
        .recoveries()
        .iter()
        .any(|recovery| recovery.confidence == Some(Confidence::High)));
}

#[test]
fn concurrent_and_legacy_ledgers_remain_readable() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join(".cubtoken");
    let mut workers = Vec::new();
    for index in 0..16 {
        let data = data.clone();
        workers.push(std::thread::spawn(move || {
            Ledger::open(&data, "parallel").note_saving("Read", None, 100 + index, 10);
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    let content = std::fs::read_to_string(data.join("session-parallel.jsonl")).unwrap();
    assert!(!content.is_empty());
    for line in content.lines() {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }

    std::fs::write(
        data.join("session-legacy.jsonl"),
        "{\"e\":\"save\",\"tool\":\"Read\",\"in\":100,\"out\":20,\"path\":\"/old.rs\"}\n{\"e\":\"edit\",\"path\":\"/old.rs\"}\n{\"e\":\"refetch\",\"path\":\"/old.rs\",\"tokens\":7,\"duration_ms\":3}\n",
    )
    .unwrap();
    let legacy = Ledger::open(&data, "legacy");
    assert!(legacy.is_protected("/old.rs"));
    assert_eq!(legacy.totals().tokens_in, 100);
    assert_eq!(legacy.refetch_tokens(), 7);
}

#[test]
fn lock_contention_makes_reads_pass_through() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    let data = temp.path().join(".cubtoken");
    let _lock_owner = Ledger::open(&data, "locked");

    assert!(run_hook(
        &full_read(&path, temp.path(), "locked").to_string(),
        &Config::default()
    )
    .is_none());
}

#[test]
fn full_repeat_escape_is_recorded_once_per_decision() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    let session = "full-repeat";
    let cfg = Config::default();
    run_hook(&start_turn(temp.path(), session), &cfg);
    assert!(run_hook(&full_read(&path, temp.path(), session).to_string(), &cfg).is_some());
    run_hook(&serde_json::json!({"hook_event_name": "PostToolBatch", "cwd": temp.path(), "session_id": session}).to_string(), &cfg);

    let mut repeat = full_read(&path, temp.path(), session);
    repeat["tool_use_id"] = serde_json::json!("repeat");
    assert!(run_hook(&repeat.to_string(), &cfg).is_none());
    assert!(run_hook(&repeat.to_string(), &cfg).is_some());

    let ledger = Ledger::open(&temp.path().join(".cubtoken"), session);
    assert_eq!(
        ledger
            .recoveries()
            .iter()
            .filter(|recovery| recovery.kind == Some(ctk_hook::ledger::RecoveryKind::FullRepeat))
            .count(),
        1
    );
    assert_eq!(ledger.refetches(), 0);
}

#[test]
fn safe_mode_only_reduces_compression() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    let static_cfg = Config::load_from(
        None,
        Some("[read]\nthreshold_tokens = 10\nnever_compress = []"),
    );
    assert!(run_hook(
        &full_read(&path, temp.path(), "static").to_string(),
        &static_cfg
    )
    .is_some());

    std::fs::create_dir_all(temp.path().join(".cubtoken")).unwrap();
    std::fs::write(
        temp.path().join(".cubtoken/adaptive-v1.json"),
        r#"{"schema":1,"policy_version":1,"reset_at_ms":0,"buckets":{"rust|8k-16k|skeleton":{"outcomes":[],"recommendation":"disabled"}}}"#,
    )
    .unwrap();
    let safe_cfg = Config::load_from(
        None,
        Some("[read]\nthreshold_tokens = 10\nnever_compress = []\n[adaptive]\nmode = \"safe\""),
    );
    assert!(run_hook(
        &full_read(&path, temp.path(), "safe").to_string(),
        &safe_cfg
    )
    .is_none());
}

/// Statistics are optional (`[stats] ledger = false`), but the two safety
/// behaviours must not be optional with them: a session-edited file is never
/// compressed, and a targeted Read always passes through. Both live outside
/// the `cfg.stats.ledger` guards in `dispatch`; this pins them there.
#[test]
fn statistics_disabled_keeps_edit_and_targeted_protection() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    let cfg = Config::load_from(
        None,
        Some("[read]\nthreshold_tokens = 10\nnever_compress = []\n[stats]\nledger = false"),
    );

    // A targeted Read passes through even with no ledger to attribute it to.
    let targeted = serde_json::json!({
        "hook_event_name": "PostToolUse", "tool_name": "Read", "session_id": "no-stats",
        "cwd": temp.path(), "tool_input": {"file_path": &path, "offset": 5, "limit": 2},
        "tool_response": {"file": {"filePath": &path, "content": "    let value = 42;\n", "startLine": 5, "numLines": 2}}
    });
    assert!(
        run_hook(&targeted.to_string(), &cfg).is_none(),
        "targeted Read must pass through with statistics disabled"
    );

    // An edited file stays protected for the session.
    let edit = serde_json::json!({
        "hook_event_name": "PostToolUse", "tool_name": "Edit", "session_id": "no-stats",
        "cwd": temp.path(), "tool_input": {"file_path": &path}, "tool_response": {}
    });
    assert!(run_hook(&edit.to_string(), &cfg).is_none());
    assert!(
        run_hook(&full_read(&path, temp.path(), "no-stats").to_string(), &cfg).is_none(),
        "an edited file must never be compressed, even with statistics disabled"
    );
}

/// `[adaptive] mode = "safe"` with `[stats] ledger = false` is a silent no-op:
/// no decision is ever recorded, so no bucket ever reaches the eight-observation
/// minimum and the effective threshold never moves. This is the current, chosen
/// behaviour (statistics stay optional) — it is pinned here because an
/// evaluation arm configured this way would report "safe" while running static.
#[test]
fn safe_mode_without_statistics_cannot_learn() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    let cfg = Config::load_from(
        None,
        Some(
            "[read]\nthreshold_tokens = 10\nnever_compress = []\n\
             [adaptive]\nmode = \"safe\"\n[stats]\nledger = false",
        ),
    );
    for _ in 0..12 {
        run_hook(&start_turn(temp.path(), "blind"), &cfg);
        assert!(run_hook(&full_read(&path, temp.path(), "blind").to_string(), &cfg).is_some());
    }
    let ledger = Ledger::open(&temp.path().join(".cubtoken"), "blind");
    assert!(
        ledger.decisions().is_empty(),
        "no decisions can be attributed with statistics disabled"
    );
    let state = ctk_hook::adaptive_state::refresh(&temp.path().join(".cubtoken"), 1_000);
    assert!(
        state.buckets.is_empty(),
        "safe mode cannot learn without the ledger it learns from"
    );
}

/// `adaptive reset` sets a new epoch; a later `refresh` must not resurrect the
/// observations that predate it. Without the `reset_at_ms` filter a reset would
/// be undone by the very next hook call.
#[test]
fn reset_survives_a_later_refresh() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    let cfg = Config::load_from(
        None,
        Some("[read]\nthreshold_tokens = 10\nnever_compress = []\n[adaptive]\nmode = \"observe\""),
    );
    let dir = temp.path().join(".cubtoken");
    run_hook(&start_turn(temp.path(), "epoch"), &cfg);
    assert!(run_hook(&full_read(&path, temp.path(), "epoch").to_string(), &cfg).is_some());
    assert!(
        !ctk_hook::adaptive_state::refresh(&dir, 1_000)
            .buckets
            .is_empty(),
        "the decision should be visible before the reset"
    );

    // Reset to an epoch after every recorded decision, then refresh again.
    let far_future = u64::MAX;
    ctk_hook::adaptive_state::reset(&dir, far_future).unwrap();
    let state = ctk_hook::adaptive_state::refresh(&dir, far_future);
    assert_eq!(state.reset_at_ms, far_future);
    assert!(
        state.buckets.is_empty(),
        "a refresh must not resurrect observations from before the reset epoch"
    );
}

/// Unreadable, truncated, or future-schema state is not a correctness input:
/// safe mode falls back to the configured threshold and still compresses.
#[test]
fn unavailable_or_malformed_state_falls_back_to_static() {
    let cfg = Config::load_from(
        None,
        Some("[read]\nthreshold_tokens = 10\nnever_compress = []\n[adaptive]\nmode = \"safe\""),
    );
    for (name, raw) in [
        ("truncated", r#"{"schema":1,"policy_version":1,"#),
        ("not json", "not json at all"),
        ("empty", ""),
        (
            "future schema",
            r#"{"schema":9,"policy_version":1,"reset_at_ms":0,"buckets":{"rust|8k-16k|skeleton":{"outcomes":[],"recommendation":"disabled"}}}"#,
        ),
        (
            "future policy",
            r#"{"schema":1,"policy_version":9,"reset_at_ms":0,"buckets":{"rust|8k-16k|skeleton":{"outcomes":[],"recommendation":"disabled"}}}"#,
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("large.rs");
        std::fs::write(&path, source()).unwrap();
        std::fs::create_dir_all(temp.path().join(".cubtoken")).unwrap();
        std::fs::write(temp.path().join(".cubtoken/adaptive-v1.json"), raw).unwrap();
        assert!(
            run_hook(&full_read(&path, temp.path(), name).to_string(), &cfg).is_some(),
            "{name} state must fall back to the configured threshold"
        );
    }

    // No state file at all is the same story.
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.rs");
    std::fs::write(&path, source()).unwrap();
    assert!(run_hook(&full_read(&path, temp.path(), "absent").to_string(), &cfg).is_some());
}

/// The eight-observation minimum, exercised through `refresh` rather than the
/// pure policy: seven refetch-heavy observations must leave the threshold
/// alone; the eighth is what earns a back-off.
#[test]
fn too_few_observations_leave_the_threshold_unchanged() {
    use ctk_hook::adaptive::Recommendation;
    use ctk_hook::ledger::{CompressionDecision, ElidedRange, Recovery, RecoveryKind};

    let seed = |count: usize, session: &str| {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".cubtoken");
        let mut ledger = Ledger::open(&dir, session);
        for index in 0..count {
            let decision_id = format!("d{index}");
            ledger.note_decision(CompressionDecision {
                decision_id: decision_id.clone(),
                turn: index as u64,
                batch: 0,
                sequence: index as u64,
                recorded_at_ms: 1_000 + index as u64,
                file_identity: format!("/a{index}.rs"),
                content_fingerprint: "v1".to_string(),
                language: "rust".to_string(),
                strategy: "skeleton".to_string(),
                profile: "default".to_string(),
                tokens_in: 10_000,
                tokens_out: 1_000,
                elided_ranges: vec![ElidedRange {
                    start_line: 2,
                    end_line: 90,
                }],
            });
            // Recovery cost >= gross saving: the shape that earns `disabled`.
            ledger.note_recovery(Recovery {
                path: format!("/a{index}.rs"),
                tokens: 9_000,
                duration_ms: 0,
                decision_id: Some(decision_id),
                kind: Some(RecoveryKind::Targeted),
                confidence: Some(Confidence::High),
                turn: Some(index as u64),
                batch: Some(1),
            });
        }
        drop(ledger);
        // Seed the epoch at 0 so the decisions above (t >= 1_000) fall after it;
        // a fresh `refresh` would otherwise open its epoch at `now` and discard
        // every earlier observation — see `reset_survives_a_later_refresh`.
        ctk_hook::adaptive_state::reset(&dir, 0).unwrap();
        let state = ctk_hook::adaptive_state::refresh(&dir, 2_000);
        let key = ctk_hook::adaptive::bucket_key("rust", 10_000, "skeleton");
        (
            ctk_hook::adaptive_state::recommendation(&state, &key),
            state
                .buckets
                .get(&key)
                .map(|b| b.outcomes.len())
                .unwrap_or(0),
            temp,
        )
    };

    let (seven, observed, _keep) = seed(7, "seven");
    assert_eq!(observed, 7, "all seven observations should be collected");
    assert_eq!(
        seven,
        Recommendation::One,
        "seven observations are below the minimum and must not move the threshold"
    );
    assert_eq!(seven.effective_threshold(2_000), 2_000);

    let (eight, observed, _keep) = seed(8, "eight");
    assert_eq!(observed, 8);
    assert_eq!(
        eight,
        Recommendation::Disabled,
        "the eighth observation is what lets the policy act"
    );
}

/// The exact aggregation `refresh` performs when it turns ledger records into
/// policy outcomes: only high-confidence recoveries carrying the decision's own
/// id count toward that decision's recovery cost, and several of them sum.
/// Pinned before optimising the aggregation so the shape cannot drift with it.
#[test]
fn refresh_sums_only_high_confidence_recoveries_for_the_matching_decision() {
    use ctk_hook::ledger::{CompressionDecision, ElidedRange, Recovery, RecoveryKind};

    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join(".cubtoken");
    let mut ledger = Ledger::open(&dir, "grouping");
    ledger.note_decision(CompressionDecision {
        decision_id: "target".to_string(),
        turn: 1,
        batch: 1,
        sequence: 1,
        recorded_at_ms: 1_000,
        file_identity: "/a.rs".to_string(),
        content_fingerprint: "v1".to_string(),
        language: "rust".to_string(),
        strategy: "skeleton".to_string(),
        profile: "default".to_string(),
        tokens_in: 10_000,
        tokens_out: 1_000,
        elided_ranges: vec![ElidedRange {
            start_line: 2,
            end_line: 90,
        }],
    });
    let recovery = |decision_id: Option<&str>, confidence, tokens| Recovery {
        path: "/a.rs".to_string(),
        tokens,
        duration_ms: 0,
        decision_id: decision_id.map(str::to_string),
        kind: Some(RecoveryKind::Targeted),
        confidence: Some(confidence),
        turn: Some(1),
        batch: Some(2),
    };
    // Two high-confidence recoveries for this decision: these sum.
    ledger.note_recovery(recovery(Some("target"), Confidence::High, 300));
    ledger.note_recovery(recovery(Some("target"), Confidence::High, 200));
    // Everything else must be ignored by the policy.
    ledger.note_recovery(recovery(Some("target"), Confidence::Medium, 9_000));
    ledger.note_recovery(recovery(Some("target"), Confidence::Low, 9_000));
    ledger.note_recovery(recovery(Some("other-decision"), Confidence::High, 9_000));
    ledger.note_recovery(recovery(None, Confidence::High, 9_000));
    drop(ledger);

    ctk_hook::adaptive_state::reset(&dir, 0).unwrap();
    let state = ctk_hook::adaptive_state::refresh(&dir, 2_000);
    let key = ctk_hook::adaptive::bucket_key("rust", 10_000, "skeleton");
    let outcomes = &state.buckets.get(&key).expect("bucket missing").outcomes;
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].gross_saved, 9_000);
    assert_eq!(
        outcomes[0].recovery_tokens, 500,
        "only the two high-confidence recoveries naming this decision may count"
    );
}

/// Recoveries attribute to decisions within one session ledger, because
/// `record_targeted_recovery` looks the decision up in that session's own
/// ledger. `refresh` must keep that scope when it aggregates: a recovery in
/// one session must not be charged to a decision in another that happens to
/// share its id. Ids come from the host's `tool_use_id` or
/// `"{session_id}:{sequence}"`, so a collision is not expected today — which
/// is exactly why nothing else would notice this property breaking.
#[test]
fn refresh_attributes_recoveries_only_within_their_own_session() {
    use ctk_hook::ledger::{CompressionDecision, ElidedRange, Recovery, RecoveryKind};

    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join(".cubtoken");
    let decision = |recorded_at_ms| CompressionDecision {
        decision_id: "shared-id".to_string(),
        turn: 1,
        batch: 1,
        sequence: 1,
        recorded_at_ms,
        file_identity: "/a.rs".to_string(),
        content_fingerprint: "v1".to_string(),
        language: "rust".to_string(),
        strategy: "skeleton".to_string(),
        profile: "default".to_string(),
        tokens_in: 10_000,
        tokens_out: 1_000,
        elided_ranges: vec![ElidedRange {
            start_line: 2,
            end_line: 90,
        }],
    };

    let mut first = Ledger::open(&dir, "first");
    first.note_decision(decision(1_000));
    first.note_recovery(Recovery {
        path: "/a.rs".to_string(),
        tokens: 500,
        duration_ms: 0,
        decision_id: Some("shared-id".to_string()),
        kind: Some(RecoveryKind::Targeted),
        confidence: Some(Confidence::High),
        turn: Some(1),
        batch: Some(2),
    });
    drop(first);

    // Same decision id, different session, no recovery of its own.
    let mut second = Ledger::open(&dir, "second");
    second.note_decision(decision(2_000));
    drop(second);

    ctk_hook::adaptive_state::reset(&dir, 0).unwrap();
    let state = ctk_hook::adaptive_state::refresh(&dir, 3_000);
    let key = ctk_hook::adaptive::bucket_key("rust", 10_000, "skeleton");
    let recovered: Vec<usize> = state.buckets[&key]
        .outcomes
        .iter()
        .map(|outcome| outcome.recovery_tokens)
        .collect();
    assert_eq!(
        recovered,
        vec![500, 0],
        "the second session's decision must not inherit the first session's recovery"
    );
}
