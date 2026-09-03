pub mod engine;
pub mod model;

pub use engine::{ApplyReport, DiffEntry, PrefStatus};
pub use model::{DefaultsValue, PrefEntry, PrefsFile, Typ};

use dotfiles_manifest::ManifestError;
use std::path::Path;

pub fn load_prefs(path: &Path) -> Result<PrefsFile, ManifestError> {
    let raw = std::fs::read_to_string(path).map_err(|e| ManifestError::Io {
        path: path.display().to_string(),
        cause: e.to_string(),
    })?;
    parse_prefs(&raw)
}

pub fn parse_prefs(raw: &str) -> Result<PrefsFile, ManifestError> {
    let file: PrefsFile = serde_yaml::from_str(raw).map_err(|e| ManifestError::Yaml {
        detail: e.to_string(),
    })?;
    file.validate()?;
    Ok(file)
}

/// Canonical JSON Schema for prefs.yaml.
pub fn prefs_schema_json() -> serde_json::Result<String> {
    let schema = schemars::schema_for!(PrefsFile);
    serde_json::to_string_pretty(&schema)
}
