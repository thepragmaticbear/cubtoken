use ctk_hook::adaptive::{self, Recommendation};

pub fn status(json: bool) -> Result<(), String> {
    let root = crate::project_root();
    let cfg = ctk_hook::Config::try_load_for(&root)
        .map_err(|error| format!("adaptive status failed: {error}"))?;
    let state = ctk_hook::adaptive_state::load(&root.join(".cubtoken"), now_ms());
    // The policy learns only from ledger-recorded decisions, so statistics
    // being off makes any non-`off` mode a silent no-op. Report the inputs
    // rather than the mode alone: an experiment has to be able to prove which
    // policy its arm actually ran.
    let learning_enabled =
        cfg.adaptive.mode != ctk_compress::config::AdaptiveMode::Off && cfg.stats.ledger;
    if json {
        let output = serde_json::to_string_pretty(&serde_json::json!({
            "mode": cfg.adaptive.mode,
            "stats_ledger": cfg.stats.ledger,
            "read_threshold_tokens": cfg.read.threshold_tokens,
            "learning_enabled": learning_enabled,
            "reset_at_ms": state.reset_at_ms,
            "policy_version": state.policy_version,
            "buckets": state.buckets,
        }))
        .map_err(|error| format!("adaptive status failed: {error}"))?;
        println!("{output}");
        return Ok(());
    }
    println!("adaptive mode: {:?}", cfg.adaptive.mode);
    if cfg.adaptive.mode != ctk_compress::config::AdaptiveMode::Off && !cfg.stats.ledger {
        println!(
            "warning: stats.ledger is false — the policy learns only from recorded \
             decisions, so this mode cannot collect observations or change a threshold"
        );
    }
    println!(
        "reset epoch: {} | policy v{}",
        state.reset_at_ms, state.policy_version
    );
    if state.buckets.is_empty() {
        println!("no attributed observations yet");
        return Ok(());
    }
    for (key, bucket) in state.buckets {
        let gross = bucket.outcomes.iter().fold(0usize, |total, outcome| {
            total.saturating_add(outcome.gross_saved)
        });
        let recovery = bucket.outcomes.iter().fold(0usize, |total, outcome| {
            total.saturating_add(outcome.recovery_tokens)
        });
        let overhead = (recovery as u128 * 100)
            .checked_div(gross as u128)
            .unwrap_or(0);
        println!(
            "{key}: {} samples, {overhead}% recovery overhead, {}",
            bucket.outcomes.len(),
            label(bucket.recommendation)
        );
    }
    Ok(())
}

pub fn explain(file: &str) -> Result<(), String> {
    let root = crate::project_root();
    let cfg = ctk_hook::Config::try_load_for(&root)
        .map_err(|error| format!("adaptive explain failed: {error}"))?;
    let path = std::path::Path::new(file);
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let preview = ctk_compress::read::preview_content(file, &content);
    let key = adaptive::bucket_key(&preview.language, preview.tokens_in, &preview.strategy);
    let state = ctk_hook::adaptive_state::load(&root.join(".cubtoken"), now_ms());
    let recommendation = ctk_hook::adaptive_state::recommendation(&state, &key);
    let observations = state
        .buckets
        .get(&key)
        .map(|bucket| bucket.outcomes.len())
        .unwrap_or(0);
    let recovery = state
        .buckets
        .get(&key)
        .map(|bucket| {
            bucket.outcomes.iter().fold(0usize, |total, outcome| {
                total.saturating_add(outcome.recovery_tokens)
            })
        })
        .unwrap_or(0);
    let gross = state
        .buckets
        .get(&key)
        .map(|bucket| {
            bucket.outcomes.iter().fold(0usize, |total, outcome| {
                total.saturating_add(outcome.gross_saved)
            })
        })
        .unwrap_or(0);
    let overhead = (recovery as u128 * 100)
        .checked_div(gross as u128)
        .unwrap_or(0);
    println!("{}", path.display());
    println!(
        "language: {language} | bucket: {} | strategy: {strategy}",
        adaptive::size_band(preview.tokens_in),
        language = preview.language,
        strategy = preview.strategy,
    );
    println!(
        "configured threshold: {} | effective threshold: {}",
        cfg.read.threshold_tokens,
        recommendation.effective_threshold(cfg.read.threshold_tokens)
    );
    println!(
        "observations: {observations} | recovery overhead: {overhead}% | decision: {}",
        label(recommendation)
    );
    Ok(())
}

pub fn reset() -> Result<(), String> {
    let path = crate::project_root().join(".cubtoken");
    let state = ctk_hook::adaptive_state::reset(&path, now_ms())
        .map_err(|error| format!("adaptive reset failed: {error}"))?;
    println!("adaptive state reset at {}", state.reset_at_ms);
    Ok(())
}

fn label(recommendation: Recommendation) -> &'static str {
    match recommendation {
        Recommendation::One => "1x",
        Recommendation::Two => "2x",
        Recommendation::Four => "4x",
        Recommendation::Disabled => "disabled",
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
