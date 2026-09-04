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
    pub max_total_matches: usize,
}

#[derive(Debug, Clone)]
pub struct GlobCfg {
    pub enabled: bool,
    pub max_paths: usize,
}

#[derive(Debug, Clone, Default)]
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
            max_total_matches: 100,
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
struct RawRead {
    enabled: Option<bool>,
    threshold_tokens: Option<usize>,
    never_compress: Option<Vec<String>>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGrep {
    enabled: Option<bool>,
    max_matches_per_file: Option<usize>,
    max_total_matches: Option<usize>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGlob {
    enabled: Option<bool>,
    max_paths: Option<usize>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBash {
    enabled: Option<bool>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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

    /// Load from `~/.config/cubtoken/config.toml` and `./.cubtoken.toml`
    /// (project file relative to the process working directory).
    pub fn load() -> Self {
        Self::load_for(std::path::Path::new("."))
    }

    /// Load from `~/.config/cubtoken/config.toml` then `<cwd>/.cubtoken.toml`,
    /// layered over defaults. Missing or unreadable files are skipped. The
    /// hook calls this with the payload's `cwd` so a globally-installed hook
    /// still honors each project's config (and matches where the ledger lives).
    pub fn load_for(cwd: &std::path::Path) -> Self {
        Self::try_load_for(cwd).unwrap_or_default()
    }

    /// Strict production loader. Invalid or unreadable configuration returns
    /// an error so the hook can pass through instead of silently compressing
    /// with defaults the user may have tried to disable.
    pub fn try_load_for(cwd: &std::path::Path) -> Result<Self, String> {
        let global = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map(|h| h.join(".config/cubtoken/config.toml"))
            .map(read_optional)
            .transpose()?
            .flatten();
        let project_path = project_root(cwd).join(".cubtoken.toml");
        let project = read_optional(project_path)?;

        let mut cfg = Config::default();
        for (name, raw_str) in [
            ("global", global.as_deref()),
            ("project", project.as_deref()),
        ] {
            let Some(raw_str) = raw_str else { continue };
            let raw = toml::from_str::<RawConfig>(raw_str)
                .map_err(|error| format!("invalid {name} config: {error}"))?;
            cfg.apply(raw);
        }
        Ok(cfg)
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
        if let Some(v) = raw.grep.max_total_matches {
            self.grep.max_total_matches = v;
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

pub fn project_root(cwd: &std::path::Path) -> std::path::PathBuf {
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    cwd.ancestors()
        .find(|dir| dir.join(".cubtoken.toml").is_file() || dir.join(".git").exists())
        .unwrap_or(&cwd)
        .to_path_buf()
}

fn read_optional(path: std::path::PathBuf) -> Result<Option<String>, String> {
    match std::fs::read_to_string(&path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
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
    fn strict_loader_rejects_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".cubtoken.toml"),
            "[read]\nthreshold_token = 7\n",
        )
        .unwrap();
        assert!(Config::try_load_for(dir.path()).is_err());
    }

    #[test]
    fn project_config_is_found_from_a_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("src/deep");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            dir.path().join(".cubtoken.toml"),
            "[read]\nthreshold_tokens = 77\n",
        )
        .unwrap();
        assert_eq!(
            Config::try_load_for(&nested).unwrap().read.threshold_tokens,
            77
        );
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
