//! Per-app Vi/En memory - the platform-independent port of uvie-mac's
//! MemoryManager. Remembers which input language the user last used in each
//! app and restores it on focus change.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Vi/En preference remembered per app when `per_app_language` is on.
/// The host supplies the app key (bundle ID, exe name, WM_CLASS, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LanguagePref {
    Vietnamese,
    English,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LanguageMemory {
    /// app identifier (lowercase, e.g. `code.exe`) -> last-used language.
    #[serde(flatten)]
    apps: HashMap<String, LanguagePref>,
}

impl LanguageMemory {
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

    pub fn remember(&mut self, app: &str, lang: LanguagePref) {
        self.apps.insert(app.to_ascii_lowercase(), lang);
    }

    pub fn recall(&self, app: &str) -> Option<LanguagePref> {
        self.apps.get(&app.to_ascii_lowercase()).copied()
    }
}
