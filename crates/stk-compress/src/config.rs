//! Layered TOML config: defaults ← global file ← project file.

use globset::{Glob, GlobSet, GlobSetBuilder};
use std::sync::OnceLock;

#[derive(Debug, Default, Clone)]
pub struct Config {
    pub read: ReadCfg,
    pub grep: GrepCfg,
    pub glob: GlobCfg,
    pub bash: BashCfg,
    pub stats: StatsCfg,
}

#[derive(Debug, Clone)]
pub struct ReadCfg {
    pub enabled: bool,
    pub threshold_tokens: usize,
    pub never_compress: Vec<String>,
    exclude_set: OnceLock<Option<GlobSet>>,
}

#[derive(Debug, Clone)]
pub struct GrepCfg {
    pub enabled: bool,
    pub max_matches_per_file: usize,
}

#[derive(Debug, Clone)]
pub struct GlobCfg {
    pub enabled: bool,
    pub max_paths: usize,
}

#[derive(Debug, Clone)]
pub struct BashCfg {
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct StatsCfg {
    pub ledger: bool,
}

impl Default for ReadCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_tokens: 2000,
            never_compress: vec!["**/*.md".into(), "**/.env*".into()],
            exclude_set: OnceLock::new(),
        }
    }
}

impl Default for GrepCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            max_matches_per_file: 5,
        }
    }
}

impl Default for GlobCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            max_paths: 50,
        }
    }
}

impl Default for BashCfg {
    fn default() -> Self {
        Self { enabled: false }
    }
}

impl Default for StatsCfg {
    fn default() -> Self {
        Self { ledger: true }
    }
}

impl ReadCfg {
    pub fn is_excluded(&self, path: &str) -> bool {
        let set = self.exclude_set.get_or_init(|| {
            let mut b = GlobSetBuilder::new();
            for pat in &self.never_compress {
                let Ok(g) = Glob::new(pat) else { return None };
                b.add(g);
            }
            b.build().ok()
        });
        match set {
            Some(s) => s.is_match(path),
            None => true, // bad pattern: exclude everything (fail open = don't compress)
        }
    }
}

/// Raw deserialization layer: every field optional so files can be partial.
#[derive(Debug, Default, serde::Deserialize)]
struct RawConfig {
    #[serde(default)]
    read: RawRead,
    #[serde(default)]
    grep: RawGrep,
    #[serde(default)]
    glob: RawGlob,
    #[serde(default)]
    bash: RawBash,
    #[serde(default)]
    stats: RawStats,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawRead {
    enabled: Option<bool>,
    threshold_tokens: Option<usize>,
    never_compress: Option<Vec<String>>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawGrep {
    enabled: Option<bool>,
    max_matches_per_file: Option<usize>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawGlob {
    enabled: Option<bool>,
    max_paths: Option<usize>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawBash {
    enabled: Option<bool>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawStats {
    ledger: Option<bool>,
}

impl Config {
    /// Layer raw TOML strings over defaults: `global` first, then `project`.
    /// Unparseable TOML is ignored (fail open: keep prior layer).
    pub fn load_from(global: Option<&str>, project: Option<&str>) -> Self {
        let mut cfg = Config::default();
        for raw_str in [global, project].into_iter().flatten() {
            let Ok(raw) = toml::from_str::<RawConfig>(raw_str) else {
                continue;
            };
            cfg.apply(raw);
        }
        cfg
    }

    /// Load from `~/.config/smalltoke/config.toml` and `<cwd>/.smalltoke.toml`.
    pub fn load() -> Self {
        let global = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map(|h| h.join(".config/smalltoke/config.toml"))
            .and_then(|p| std::fs::read_to_string(p).ok());
        let project = std::fs::read_to_string(".smalltoke.toml").ok();
        Self::load_from(global.as_deref(), project.as_deref())
    }

    fn apply(&mut self, raw: RawConfig) {
        if let Some(v) = raw.read.enabled {
            self.read.enabled = v;
        }
        if let Some(v) = raw.read.threshold_tokens {
            self.read.threshold_tokens = v;
        }
        if let Some(v) = raw.read.never_compress {
            self.read.never_compress = v;
            self.read.exclude_set = OnceLock::new();
        }
        if let Some(v) = raw.grep.enabled {
            self.grep.enabled = v;
        }
        if let Some(v) = raw.grep.max_matches_per_file {
            self.grep.max_matches_per_file = v;
        }
        if let Some(v) = raw.glob.enabled {
            self.glob.enabled = v;
        }
        if let Some(v) = raw.glob.max_paths {
            self.glob.max_paths = v;
        }
        if let Some(v) = raw.bash.enabled {
            self.bash.enabled = v;
        }
        if let Some(v) = raw.stats.ledger {
            self.stats.ledger = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_no_file() {
        let c = Config::load_from(None, None);
        assert_eq!(c.read.threshold_tokens, 2000);
        assert!(c.read.enabled && c.grep.enabled && c.glob.enabled);
        assert!(!c.bash.enabled);
        assert!(c.stats.ledger);
    }

    #[test]
    fn project_overrides_global() {
        let global = "[read]\nthreshold_tokens = 5000";
        let project = "[read]\nthreshold_tokens = 1000";
        let c = Config::load_from(Some(global), Some(project));
        assert_eq!(c.read.threshold_tokens, 1000);
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let c = Config::load_from(None, Some("[bash]\nenabled = true"));
        assert!(c.bash.enabled);
        assert_eq!(c.read.threshold_tokens, 2000);
    }

    #[test]
    fn unparseable_layer_is_ignored() {
        let c = Config::load_from(
            Some("not [valid toml"),
            Some("[read]\nthreshold_tokens = 7"),
        );
        assert_eq!(c.read.threshold_tokens, 7);
    }

    #[test]
    fn never_compress_glob_matches() {
        let c = Config::load_from(None, Some("[read]\nnever_compress = [\"**/*.md\"]"));
        assert!(c.read.is_excluded("docs/notes.md"));
        assert!(!c.read.is_excluded("src/main.rs"));
    }

    #[test]
    fn default_excludes_env_files() {
        let c = Config::default();
        assert!(c.read.is_excluded("app/.env.local"));
    }
}
