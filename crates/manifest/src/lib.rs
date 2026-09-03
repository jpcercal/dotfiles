pub mod apps;
pub mod commands;
pub mod error;
pub mod validate;

pub use apps::*;
pub use commands::*;
pub use error::ManifestError;

use std::path::Path;

/// Load and parse `apps.yaml`.
pub fn load_manifest(path: &Path) -> Result<Manifest, ManifestError> {
    let raw = std::fs::read_to_string(path).map_err(|e| ManifestError::Io {
        path: path.display().to_string(),
        cause: e.to_string(),
    })?;
    parse_manifest(&raw)
}

/// Parse a manifest from a YAML string.
pub fn parse_manifest(raw: &str) -> Result<Manifest, ManifestError> {
    let mut m: Manifest = serde_yaml::from_str(raw).map_err(|e| ManifestError::Yaml {
        detail: e.to_string(),
    })?;
    if m.schema_version == 0 {
        m.schema_version = 1;
    }
    validate::validate(&m)?;
    Ok(m)
}

/// Load and parse `commands.yaml`.
pub fn load_commands(path: &Path) -> Result<CommandsManifest, ManifestError> {
    let raw = std::fs::read_to_string(path).map_err(|e| ManifestError::Io {
        path: path.display().to_string(),
        cause: e.to_string(),
    })?;
    parse_commands(&raw)
}

pub fn parse_commands(raw: &str) -> Result<CommandsManifest, ManifestError> {
    serde_yaml::from_str(raw).map_err(|e| ManifestError::Yaml {
        detail: e.to_string(),
    })
}

/// The canonical JSON Schema for `apps.yaml`, derived from the types (never
/// hand-maintained). `dotfiles schema export` prints exactly this.
pub fn schema_json() -> serde_json::Result<String> {
    let schema = schemars::schema_for!(Manifest);
    serde_json::to_string_pretty(&schema)
}
