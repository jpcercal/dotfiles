use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepReport {
    pub name: String,
    pub status: String, // success | failed | skipped
    pub duration_seconds: i64,
    pub updated: Value,
    pub failed: Value,
    pub note: String,
    pub raw_log: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    pub on_ac_power: bool,
    pub battery_pct: i32,
    pub free_disk_gb: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub schema: String,
    pub run_id: String,
    pub trigger: String,
    pub started_at: String,
    pub finished_at: String,
    pub duration_seconds: i64,
    pub status: String,
    pub environment: Environment,
    pub steps: Vec<StepReport>,
    pub audit: Value,
}

impl Report {
    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(self)?;
        std::fs::write(path, raw)?;
        Ok(())
    }

    pub fn now_iso() -> String {
        Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
    }
}
