//! Session-scoped JSONL state. Every write is best-effort: missing or locked
//! state must never change the hook's pass-through behavior.

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Default, Clone, Copy)]
pub struct Totals {
    pub tokens_in: usize,
    pub tokens_out: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ElidedRange {
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompressionDecision {
    pub decision_id: String,
    pub turn: u64,
    pub batch: u64,
    pub sequence: u64,
    pub recorded_at_ms: u64,
    pub file_identity: String,
    pub content_fingerprint: String,
    pub language: String,
    pub strategy: String,
    pub profile: String,
    pub tokens_in: usize,
    pub tokens_out: usize,
    #[serde(default)]
    pub elided_ranges: Vec<ElidedRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryKind {
    Targeted,
    FullRepeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Recovery {
    pub path: String,
    pub tokens: usize,
    pub duration_ms: u64,
    pub decision_id: Option<String>,
    pub kind: Option<RecoveryKind>,
    pub confidence: Option<Confidence>,
    pub turn: Option<u64>,
    pub batch: Option<u64>,
}

#[derive(Debug, Clone)]
struct EditEvent {
    path: String,
    sequence: u64,
}

pub struct Ledger {
    file: PathBuf,
    /// The directory is safe to write and this handle is not read-only. Kept
    /// apart from `lock`, which additionally requires winning the advisory
    /// lockfile: edit protection is a correctness guarantee and must not be
    /// lost merely because another process holds that lock.
    writable: bool,
    lock: Option<LedgerLock>,
    edited: HashSet<String>,
    compressed: HashSet<String>,
    savings: Vec<(String, usize, usize)>,
    decisions: Vec<CompressionDecision>,
    edits: Vec<EditEvent>,
    recoveries: Vec<Recovery>,
    refetches: usize,
    refetch_tokens: usize,
    refetch_duration_ms: u64,
    turn: u64,
    batch: u64,
    sequence: u64,
    visible_output_tokens: usize,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "e")]
enum Record {
    #[serde(rename = "edit")]
    Edit {
        path: String,
        #[serde(default)]
        sequence: u64,
    },
    #[serde(rename = "save")]
    Save {
        tool: String,
        r#in: usize,
        out: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision: Option<CompressionDecision>,
    },
    #[serde(rename = "refetch")]
    Refetch {
        path: String,
        #[serde(default)]
        tokens: usize,
        #[serde(default)]
        duration_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<RecoveryKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<Confidence>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        batch: Option<u64>,
    },
    #[serde(rename = "turn")]
    Turn { turn: u64 },
    #[serde(rename = "batch")]
    Batch { turn: u64, batch: u64 },
    #[serde(rename = "output")]
    Output { tokens: usize },
}

impl Ledger {
    /// Opens the session ledger and replays all recognized old and new records.
    pub fn open(dir: &Path, session_id: &str) -> Self {
        let file = dir.join(format!("session-{}.jsonl", safe_session_id(session_id)));
        Self::from_file(file, true)
    }

    fn from_file(file: PathBuf, writable: bool) -> Self {
        let writable = writable && file.parent().is_some_and(safe_data_dir);
        let lock = if writable {
            LedgerLock::try_acquire(&file)
        } else {
            None
        };
        let mut ledger = Self {
            file,
            writable,
            lock,
            edited: HashSet::new(),
            compressed: HashSet::new(),
            savings: Vec::new(),
            decisions: Vec::new(),
            edits: Vec::new(),
            recoveries: Vec::new(),
            refetches: 0,
            refetch_tokens: 0,
            refetch_duration_ms: 0,
            turn: 0,
            batch: 0,
            sequence: 0,
            visible_output_tokens: 0,
        };
        if let Ok(content) = std::fs::read_to_string(&ledger.file) {
            for line in content.lines() {
                if let Ok(record) = serde_json::from_str::<Record>(line) {
                    ledger.replay(record);
                }
            }
        }
        ledger
    }

    fn replay(&mut self, record: Record) {
        match record {
            Record::Edit { path, sequence } => {
                self.edited.insert(path.clone());
                self.edits.push(EditEvent { path, sequence });
                self.sequence = self.sequence.max(sequence);
            }
            Record::Save {
                tool,
                r#in,
                out,
                path,
                decision,
            } => {
                self.savings.push((tool, r#in, out));
                if let Some(path) = path {
                    self.compressed.insert(path);
                }
                if let Some(decision) = decision {
                    self.sequence = self.sequence.max(decision.sequence);
                    self.turn = self.turn.max(decision.turn);
                    self.batch = self.batch.max(decision.batch);
                    self.decisions.push(decision);
                }
            }
            Record::Refetch {
                path,
                tokens,
                duration_ms,
                decision_id,
                kind,
                confidence,
                turn,
                batch,
            } => {
                if kind != Some(RecoveryKind::FullRepeat) {
                    self.refetches = self.refetches.saturating_add(1);
                    self.refetch_tokens = self.refetch_tokens.saturating_add(tokens);
                    self.refetch_duration_ms = self.refetch_duration_ms.saturating_add(duration_ms);
                }
                self.recoveries.push(Recovery {
                    path,
                    tokens,
                    duration_ms,
                    decision_id,
                    kind,
                    confidence,
                    turn,
                    batch,
                });
            }
            Record::Turn { turn } => {
                self.turn = self.turn.max(turn);
                self.batch = 0;
            }
            Record::Batch { turn, batch } => {
                self.turn = self.turn.max(turn);
                if turn == self.turn {
                    self.batch = self.batch.max(batch);
                }
            }
            Record::Output { tokens } => {
                self.visible_output_tokens = self.visible_output_tokens.saturating_add(tokens);
            }
        }
    }

    pub fn note_turn_start(&mut self) {
        self.turn = self.turn.saturating_add(1);
        self.batch = 0;
        self.append(&Record::Turn { turn: self.turn });
    }

    pub fn note_batch_end(&mut self) {
        self.batch = self.batch.saturating_add(1);
        self.append(&Record::Batch {
            turn: self.turn,
            batch: self.batch,
        });
    }

    pub fn note_visible_output(&mut self, tokens: usize) {
        self.visible_output_tokens = self.visible_output_tokens.saturating_add(tokens);
        self.append(&Record::Output { tokens });
    }

    pub fn note_edit(&mut self, path: &str) {
        self.note_edit_identity(path);
    }

    pub fn note_edit_identity(&mut self, path: &str) {
        if self.edited.insert(path.to_string()) {
            let sequence = self.next_sequence();
            self.edits.push(EditEvent {
                path: path.to_string(),
                sequence,
            });
            self.append_durable(&Record::Edit {
                path: path.to_string(),
                sequence,
            });
        }
    }

    pub fn is_protected(&self, path: &str) -> bool {
        self.edited.contains(path)
    }

    pub fn note_saving(
        &mut self,
        tool: &str,
        path: Option<&str>,
        tokens_in: usize,
        tokens_out: usize,
    ) {
        self.savings.push((tool.to_string(), tokens_in, tokens_out));
        if let Some(path) = path {
            self.compressed.insert(path.to_string());
        }
        self.append(&Record::Save {
            tool: tool.to_string(),
            r#in: tokens_in,
            out: tokens_out,
            path: path.map(String::from),
            decision: None,
        });
    }

    pub fn note_decision(&mut self, decision: CompressionDecision) {
        self.savings
            .push(("Read".to_string(), decision.tokens_in, decision.tokens_out));
        self.compressed.insert(decision.file_identity.clone());
        self.sequence = self.sequence.max(decision.sequence);
        self.decisions.push(decision.clone());
        self.append(&Record::Save {
            tool: "Read".to_string(),
            r#in: decision.tokens_in,
            out: decision.tokens_out,
            path: Some(decision.file_identity.clone()),
            decision: Some(decision),
        });
    }

    pub fn was_compressed(&self, path: &str) -> bool {
        self.compressed.contains(path)
    }

    pub fn note_refetch(&mut self, path: &str, tokens: usize, duration_ms: u64) {
        self.note_recovery(Recovery {
            path: path.to_string(),
            tokens,
            duration_ms,
            decision_id: None,
            kind: None,
            confidence: None,
            turn: None,
            batch: None,
        });
    }

    pub fn note_recovery(&mut self, recovery: Recovery) {
        if recovery.kind != Some(RecoveryKind::FullRepeat) {
            self.refetches = self.refetches.saturating_add(1);
            self.refetch_tokens = self.refetch_tokens.saturating_add(recovery.tokens);
            self.refetch_duration_ms = self
                .refetch_duration_ms
                .saturating_add(recovery.duration_ms);
        }
        self.recoveries.push(recovery.clone());
        self.append(&Record::Refetch {
            path: recovery.path,
            tokens: recovery.tokens,
            duration_ms: recovery.duration_ms,
            decision_id: recovery.decision_id,
            kind: recovery.kind,
            confidence: recovery.confidence,
            turn: recovery.turn,
            batch: recovery.batch,
        });
    }

    pub fn current_turn(&self) -> u64 {
        self.turn
    }

    pub fn lock_acquired(&self) -> bool {
        self.lock.is_some()
    }

    pub fn current_batch(&self) -> u64 {
        self.batch
    }

    pub fn next_sequence(&mut self) -> u64 {
        self.sequence = self.sequence.saturating_add(1);
        self.sequence
    }

    pub fn latest_decision(&self, file_identity: &str) -> Option<&CompressionDecision> {
        self.decisions
            .iter()
            .rev()
            .find(|decision| decision.file_identity == file_identity)
    }

    pub fn has_edit_after(&self, decision: &CompressionDecision) -> bool {
        self.edits
            .iter()
            .any(|edit| edit.path == decision.file_identity && edit.sequence > decision.sequence)
    }

    pub fn decisions(&self) -> &[CompressionDecision] {
        &self.decisions
    }

    pub fn recoveries(&self) -> &[Recovery] {
        &self.recoveries
    }

    pub fn totals(&self) -> Totals {
        self.per_tool()
            .into_values()
            .fold(Totals::default(), |mut total, value| {
                total.tokens_in = total.tokens_in.saturating_add(value.tokens_in);
                total.tokens_out = total.tokens_out.saturating_add(value.tokens_out);
                total
            })
    }

    pub fn per_tool(&self) -> HashMap<String, Totals> {
        let mut map = HashMap::new();
        for (tool, tokens_in, tokens_out) in &self.savings {
            let total = map.entry(tool.clone()).or_insert_with(Totals::default);
            total.tokens_in = total.tokens_in.saturating_add(*tokens_in);
            total.tokens_out = total.tokens_out.saturating_add(*tokens_out);
        }
        map
    }

    pub fn refetches(&self) -> usize {
        self.refetches
    }
    pub fn refetch_tokens(&self) -> usize {
        self.refetch_tokens
    }
    pub fn refetch_duration_ms(&self) -> u64 {
        self.refetch_duration_ms
    }
    pub fn visible_output_tokens(&self) -> usize {
        self.visible_output_tokens
    }

    pub fn load_all(dir: &Path) -> Vec<Self> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("session-") && name.ends_with(".jsonl"))
            })
            .map(|file| Self::from_file(file, false))
            .collect()
    }

    /// Statistics: only written by the handle holding the advisory lock, so
    /// concurrent writers do not each add their own view of the same session.
    fn append(&self, record: &Record) {
        if self.lock.is_none() {
            return;
        }
        self.append_durable(record);
    }

    /// Edit protection: written whenever the directory is safe, lock or not.
    /// A single short line opened `O_APPEND` does not interleave with another
    /// process's line, and the alternative when the lock is unavailable is not
    /// a tidier record — it is no record at all, and a file the model edited
    /// staying compressible for the rest of the session.
    fn append_durable(&self, record: &Record) {
        if !self.writable {
            return;
        }
        // The data directory is checked for being a symlink when the ledger is
        // opened; the session file inside it needs the same check, or a
        // `session-*.jsonl` symlink redirects these appends into its target.
        if std::fs::symlink_metadata(&self.file)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return;
        }
        let Ok(json) = serde_json::to_string(record) else {
            return;
        };
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)
        else {
            return;
        };
        let _ = writeln!(file, "{json}");
    }
}

/// Prefer a canonical identity, but preserve a deterministic lexical fallback
/// when a fixture or newly-created path does not yet exist on disk.
pub fn normalize_file_identity(cwd: &Path, path: &str) -> String {
    let candidate = Path::new(path);
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        cwd.join(candidate)
    };
    std::fs::canonicalize(&absolute)
        .unwrap_or_else(|_| lexical_normalize(&absolute))
        .to_string_lossy()
        .to_string()
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn safe_session_id(session_id: &str) -> String {
    session_id
        .chars()
        .take(128)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn safe_data_dir(parent: &Path) -> bool {
    let safe = match std::fs::symlink_metadata(parent) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => false,
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(parent).is_ok()
                && std::fs::symlink_metadata(parent)
                    .map(|metadata| !metadata.file_type().is_symlink() && metadata.is_dir())
                    .unwrap_or(false)
        }
        Err(_) => false,
    };
    if safe {
        let ignore = parent.join(".gitignore");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(ignore)
        {
            let _ = file.write_all(b"*\n");
        }
    }
    safe
}

// ponytail: create-new sentinel lock; use OS advisory locks only if stale-lock
// recovery or shared/network filesystems become a real requirement.
struct LedgerLock {
    path: PathBuf,
}

impl LedgerLock {
    fn try_acquire(ledger: &Path) -> Option<Self> {
        let path = ledger.with_extension("jsonl.lock");
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .ok()?;
        Some(Self { path })
    }
}

impl Drop for LedgerLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
