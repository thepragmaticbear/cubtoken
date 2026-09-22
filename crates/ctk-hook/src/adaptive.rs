//! Pure aggregation and policy for the conservative Read governor.

use std::collections::BTreeMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Outcome {
    pub recorded_at_ms: u64,
    pub gross_saved: usize,
    pub recovery_tokens: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Recommendation {
    #[default]
    One,
    Two,
    Four,
    Disabled,
}

impl Recommendation {
    pub fn effective_threshold(self, configured: usize) -> usize {
        match self {
            Self::One => configured,
            Self::Two => configured.saturating_mul(2),
            Self::Four => configured.saturating_mul(4),
            Self::Disabled => usize::MAX,
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Four => 4,
            Self::Disabled => u8::MAX,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Bucket {
    #[serde(default)]
    pub outcomes: Vec<Outcome>,
    #[serde(default = "default_recommendation")]
    pub recommendation: Recommendation,
}

fn default_recommendation() -> Recommendation {
    Recommendation::One
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AdaptiveState {
    pub schema: u8,
    pub policy_version: u8,
    pub reset_at_ms: u64,
    #[serde(default)]
    pub buckets: BTreeMap<String, Bucket>,
}

impl AdaptiveState {
    pub fn empty(reset_at_ms: u64) -> Self {
        Self {
            schema: 1,
            policy_version: 1,
            reset_at_ms,
            buckets: BTreeMap::new(),
        }
    }
}

pub fn bucket_key(language: &str, tokens_in: usize, strategy: &str) -> String {
    format!("{language}|{}|{strategy}", size_band(tokens_in))
}

pub fn size_band(tokens: usize) -> &'static str {
    match tokens {
        0..=3_999 => "2k-4k",
        4_000..=7_999 => "4k-8k",
        8_000..=15_999 => "8k-16k",
        16_000..=31_999 => "16k-32k",
        _ => "32k+",
    }
}

pub fn recommend(outcomes: &[Outcome]) -> Recommendation {
    if outcomes.len() < 8 {
        return Recommendation::One;
    }
    let gross = outcomes.iter().fold(0usize, |total, outcome| {
        total.saturating_add(outcome.gross_saved)
    });
    let recovery = outcomes.iter().fold(0usize, |total, outcome| {
        total.saturating_add(outcome.recovery_tokens)
    });
    let events = outcomes
        .iter()
        .filter(|outcome| outcome.recovery_tokens > 0)
        .count();
    if recovery >= gross {
        Recommendation::Disabled
    } else if recovery.saturating_mul(100) > gross.saturating_mul(35) {
        Recommendation::Four
    } else if recovery.saturating_mul(100) > gross.saturating_mul(20)
        || events.saturating_mul(100) > outcomes.len().saturating_mul(40)
    {
        Recommendation::Two
    } else {
        Recommendation::One
    }
}

pub fn retain_latest(mut outcomes: Vec<Outcome>) -> Vec<Outcome> {
    outcomes.sort_by_key(|outcome| outcome.recorded_at_ms);
    outcomes
        .into_iter()
        .rev()
        .take(32)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

pub fn merge_one_way(previous: Recommendation, next: Recommendation) -> Recommendation {
    if previous.rank() >= next.rank() {
        previous
    } else {
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_only_backs_off_after_enough_observations() {
        assert_eq!(
            recommend(&vec![
                Outcome {
                    recorded_at_ms: 1,
                    gross_saved: 100,
                    recovery_tokens: 100
                };
                7
            ]),
            Recommendation::One
        );
        assert_eq!(
            recommend(&vec![
                Outcome {
                    recorded_at_ms: 1,
                    gross_saved: 100,
                    recovery_tokens: 25
                };
                8
            ]),
            Recommendation::Two
        );
        assert_eq!(
            recommend(&vec![
                Outcome {
                    recorded_at_ms: 1,
                    gross_saved: 100,
                    recovery_tokens: 40
                };
                8
            ]),
            Recommendation::Four
        );
        assert_eq!(
            recommend(&vec![
                Outcome {
                    recorded_at_ms: 1,
                    gross_saved: 100,
                    recovery_tokens: 100
                };
                8
            ]),
            Recommendation::Disabled
        );
    }

    #[test]
    fn backoff_is_one_way() {
        assert_eq!(
            merge_one_way(Recommendation::Four, Recommendation::One),
            Recommendation::Four
        );
        assert_eq!(
            Recommendation::Disabled.effective_threshold(2_000),
            usize::MAX
        );
    }
}
