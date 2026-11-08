//! Text macros: user-defined trigger -> expansion pairs, applied when a
//! word boundary commits a matching trigger word (mirrors uvie-mac's
//! MacroManager).

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroEntry {
    /// The literal keystroke sequence that triggers expansion (e.g. `sg`).
    pub trigger: String,
    /// The replacement text (e.g. `Sài Gòn`).
    pub expansion: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MacroTable {
    pub entries: Vec<MacroEntry>,
}

impl MacroTable {
    pub fn new(entries: Vec<MacroEntry>) -> Self {
        Self { entries }
    }

    pub fn add(&mut self, trigger: impl Into<String>, expansion: impl Into<String>) {
        let trigger = trigger.into();
        self.entries.retain(|e| e.trigger != trigger);
        self.entries.push(MacroEntry {
            trigger,
            expansion: expansion.into(),
        });
    }

    pub fn remove(&mut self, trigger: &str) {
        self.entries.retain(|e| e.trigger != trigger);
    }

    /// Look up the expansion for a just-committed word.
    pub fn lookup(&self, word: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.trigger == word)
            .map(|e| e.expansion.as_str())
    }

    /// Load the table from `macros.json`. Missing/corrupt files yield an
    /// empty table - same resilience rule as the host settings loader.
    /// Written exclusively by the settings window; the app only reads.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }
}
