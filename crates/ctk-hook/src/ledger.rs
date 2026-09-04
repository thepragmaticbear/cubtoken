//! Session-scoped JSONL ledger: which files the model has edited (never
//! compress those again this session — the Edit-correctness hazard), which
//! files we compressed, and the token record behind `ctk stats`. All I/O is
//! best-effort; failures degrade to "no protection recorded, no savings
//! recorded", never to a broken hook.

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Copy)]
pub struct Totals {
    pub tokens_in: usize,
    pub tokens_out: usize,
}

pub struct Ledger {
    file: PathBuf,
    edited: HashSet<String>,
    /// Files compressed this session — the population a refetch can come from.
    compressed: HashSet<String>,
    savings: Vec<(String, usize, usize)>,
    refetches: usize,
    refetch_tokens: usize,
    refetch_duration_ms: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "e")]
enum Record {
    #[serde(rename = "edit")]
    Edit { path: String },
    #[serde(rename = "save")]
    Save {
        tool: String,
        r#in: usize,
        out: usize,
        /// Present for Read; absent in ledgers written before refetch tracking.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// A targeted `Read(offset/limit)` into a file we compressed earlier this
    /// session: the model buying back what we elided. The cost side of the
    /// savings number.
    #[serde(rename = "refetch")]
    Refetch {
        path: String,
        #[serde(default)]
        tokens: usize,
        #[serde(default)]
        duration_ms: u64,
    },
}

impl Ledger {
    /// `dir` is the data directory (e.g. `<project>/.cubtoken`); created on
    /// first write. Existing records for `session_id` are replayed.
    pub fn open(dir: &Path, session_id: &str) -> Self {
        // ponytail: Claude currently sends UUID-like IDs. Keep that shape
        // filename-safe; hash instead if the host ever permits arbitrary IDs.
        let safe_session_id: String = session_id
            .chars()
            .take(128)
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let file = dir.join(format!("session-{safe_session_id}.jsonl"));
        let mut ledger = Ledger {
            file,
            edited: HashSet::new(),
            compressed: HashSet::new(),
            savings: Vec::new(),
            refetches: 0,
            refetch_tokens: 0,
            refetch_duration_ms: 0,
        };
        if let Ok(content) = std::fs::read_to_string(&ledger.file) {
            for line in content.lines() {
                match serde_json::from_str::<Record>(line) {
                    Ok(Record::Edit { path }) => {
                        ledger.edited.insert(path);
                    }
                    Ok(Record::Save {
                        tool,
                        r#in,
                        out,
                        path,
                    }) => {
                        ledger.savings.push((tool, r#in, out));
                        if let Some(p) = path {
                            ledger.compressed.insert(p);
                        }
                    }
                    Ok(Record::Refetch {
                        tokens,
                        duration_ms,
                        ..
                    }) => {
                        ledger.refetches = ledger.refetches.saturating_add(1);
                        ledger.refetch_tokens = ledger.refetch_tokens.saturating_add(tokens);
                        ledger.refetch_duration_ms =
                            ledger.refetch_duration_ms.saturating_add(duration_ms);
                    }
                    Err(_) => {} // skip corrupt lines
                }
            }
        }
        ledger
    }

    pub fn note_edit(&mut self, path: &str) {
        if self.edited.insert(path.to_string()) {
            self.append(&Record::Edit {
                path: path.to_string(),
            });
        }
    }

    pub fn is_protected(&self, path: &str) -> bool {
        self.edited.contains(path)
    }

    /// `path` is recorded for Read so a later targeted Read of the same file
    /// can be attributed as a refetch; other tools pass `None`.
    pub fn note_saving(
        &mut self,
        tool: &str,
        path: Option<&str>,
        tokens_in: usize,
        tokens_out: usize,
    ) {
        self.savings.push((tool.to_string(), tokens_in, tokens_out));
        if let Some(p) = path {
            self.compressed.insert(p.to_string());
        }
        self.append(&Record::Save {
            tool: tool.to_string(),
            r#in: tokens_in,
            out: tokens_out,
            path: path.map(String::from),
        });
    }

    pub fn was_compressed(&self, path: &str) -> bool {
        self.compressed.contains(path)
    }

    pub fn note_refetch(&mut self, path: &str, tokens: usize, duration_ms: u64) {
        self.refetches = self.refetches.saturating_add(1);
        self.refetch_tokens = self.refetch_tokens.saturating_add(tokens);
        self.refetch_duration_ms = self.refetch_duration_ms.saturating_add(duration_ms);
        self.append(&Record::Refetch {
            path: path.to_string(),
            tokens,
            duration_ms,
        });
    }

    /// Targeted Reads back into files this session compressed.
    pub fn refetches(&self) -> usize {
        self.refetches
    }

    pub fn refetch_tokens(&self) -> usize {
        self.refetch_tokens
    }

    pub fn refetch_duration_ms(&self) -> u64 {
        self.refetch_duration_ms
    }

    pub fn totals(&self) -> Totals {
        let mut t = Totals::default();
        for (_, tin, tout) in &self.savings {
            t.tokens_in = t.tokens_in.saturating_add(*tin);
            t.tokens_out = t.tokens_out.saturating_add(*tout);
        }
        t
    }

    pub fn per_tool(&self) -> HashMap<String, Totals> {
        let mut map: HashMap<String, Totals> = HashMap::new();
        for (tool, tin, tout) in &self.savings {
            let e = map.entry(tool.clone()).or_default();
            e.tokens_in = e.tokens_in.saturating_add(*tin);
            e.tokens_out = e.tokens_out.saturating_add(*tout);
        }
        map
    }

    fn append(&self, record: &Record) {
        let Some(parent) = self.file.parent() else {
            return;
        };
        match std::fs::symlink_metadata(parent) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => return,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if std::fs::create_dir_all(parent).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
        // A globally-installed hook drops .cubtoken/ into every project it
        // touches; a self-ignoring directory keeps it out of git status.
        let ignore = parent.join(".gitignore");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&ignore)
        {
            let _ = file.write_all(b"*\n");
        }
        let Ok(json) = serde_json::to_string(record) else {
            return;
        };
        if std::fs::symlink_metadata(&self.file)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return;
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)
        {
            let _ = writeln!(f, "{json}");
        }
    }
}
