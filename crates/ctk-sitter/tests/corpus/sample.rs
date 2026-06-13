use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;
use std::borrow::Cow;

/// Kind of widget understood by the registry.
pub enum WidgetKind {
    Plain,
    Fancy { trim: String },
    Composite(Vec<WidgetKind>),
}

/// A widget tracked by the [`Registry`].
pub struct Widget {
    pub id: u64,
    pub name: String,
    pub kind: WidgetKind,
    created_order: usize,
    tags: HashSet<String>,
}

/// Anything that can be persisted to the backing store.
pub trait Persist {
    /// Serialize into the store's wire format.
    fn to_wire(&self) -> Vec<u8>;

    /// Best-effort human label for logs.
    fn label(&self) -> Cow<'_, str> {
        Cow::Borrowed("unlabeled")
    }
}

/// Central widget registry; the main entry point of this module.
pub struct Registry {
    widgets: HashMap<u64, Widget>,
    by_name: HashMap<String, u64>,
    next_order: usize,
}

impl Registry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            widgets: HashMap::new(),
            by_name: HashMap::new(),
            next_order: 0,
        }
    }

    /// Insert a widget, replacing any widget with the same id.
    ///
    /// Returns the previous widget if one was replaced.
    pub fn insert(&mut self, mut widget: Widget) -> Option<Widget> {
        widget.created_order = self.next_order;
        self.next_order += 1;
        self.by_name.insert(widget.name.clone(), widget.id);
        self.widgets.insert(widget.id, widget)
    }

    /// Look up a widget by name.
    pub fn by_name(&self, name: &str) -> Option<&Widget> {
        let id = self.by_name.get(name)?;
        self.widgets.get(id)
    }

    fn evict_oldest(&mut self) -> Option<u64> {
        let oldest = self
            .widgets
            .values()
            .min_by_key(|w| w.created_order)
            .map(|w| w.id)?;
        let widget = self.widgets.remove(&oldest)?;
        self.by_name.remove(&widget.name);
        Some(oldest)
    }
}

impl Persist for Registry {
    fn to_wire(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for widget in self.widgets.values() {
            out.extend_from_slice(&widget.id.to_le_bytes());
            out.extend_from_slice(widget.name.as_bytes());
            out.push(0);
        }
        out
    }
}

impl fmt::Debug for Widget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Widget")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish()
    }
}

const MAX_WIDGETS: usize = 4096;

/// Load a registry from a directory of wire files.
pub fn load_registry(dir: &Path) -> io::Result<Registry> {
    let mut registry = Registry::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let raw = fs::read(entry.path())?;
        if let Some(widget) = decode_widget(&raw) {
            registry.insert(widget);
        }
        if registry.widgets.len() >= MAX_WIDGETS {
            break;
        }
    }
    Ok(registry)
}

fn decode_widget(raw: &[u8]) -> Option<Widget> {
    if raw.len() < 9 {
        return None;
    }
    let id = u64::from_le_bytes(raw[..8].try_into().ok()?);
    let name = String::from_utf8(raw[8..].to_vec()).ok()?;
    Some(Widget {
        id,
        name: name.trim_end_matches('\0').to_string(),
        kind: WidgetKind::Plain,
        created_order: 0,
        tags: HashSet::new(),
    })
}
