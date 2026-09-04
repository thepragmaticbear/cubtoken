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
    Refetch { path: String },
}

impl Ledger {
    /// `dir` is the data directory (e.g. `<project>/.cubtoken`); created on
    /// first write. Existing records for `session_id` are replayed.
    pub fn open(dir: &Path, session_id: &str) -> Self {
        let file = dir.join(format!("session-{session_id}.jsonl"));
        let mut ledger = Ledger {
            file,
            edited: HashSet::new(),
            compressed: HashSet::new(),
            savings: Vec::new(),
            refetches: 0,
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
                    Ok(Record::Refetch { .. }) => ledger.refetches += 1,
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

    pub fn note_refetch(&mut self, path: &str) {
        self.refetches += 1;
        self.append(&Record::Refetch {
            path: path.to_string(),
        });
    }

    /// Targeted Reads back into files this session compressed.
    pub fn refetches(&self) -> usize {
        self.refetches
    }

    pub fn totals(&self) -> Totals {
        let mut t = Totals::default();
        for (_, tin, tout) in &self.savings {
            t.tokens_in += tin;
            t.tokens_out += tout;
        }
        t
    }

    pub fn per_tool(&self) -> HashMap<String, Totals> {
        let mut map: HashMap<String, Totals> = HashMap::new();
        for (tool, tin, tout) in &self.savings {
            let e = map.entry(tool.clone()).or_default();
            e.tokens_in += tin;
            e.tokens_out += tout;
        }
        map
    }

    fn append(&self, record: &Record) {
        let Some(parent) = self.file.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let Ok(json) = serde_json::to_string(record) else {
            return;
        };
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)
        {
            let _ = writeln!(f, "{json}");
        }
    }
}
