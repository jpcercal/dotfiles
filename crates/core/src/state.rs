use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct State {
    /// Unix seconds or null
    pub last_attempt_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub last_dialog_at: Option<i64>,
    #[serde(default)]
    pub last_failed_steps: Vec<String>,
    pub last_outcome: Option<String>,
}

impl State {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading state file {}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        let s: Self =
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(s)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let raw = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp, raw)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn init_if_missing(path: &Path) -> anyhow::Result<Self> {
        if path.exists() {
            return Self::load(path);
        }
        let s = Self::default();
        s.save(path)?;
        Ok(s)
    }
}
