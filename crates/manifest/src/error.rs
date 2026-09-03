use thiserror::Error;

#[derive(Error, Debug)]
pub enum ManifestError {
    #[error("cannot read {path}: {cause}")]
    Io { path: String, cause: String },

    #[error("invalid YAML: {detail}")]
    Yaml { detail: String },

    #[error("manifest validation failed:\n{}", .0.join("\n"))]
    Validation(Vec<String>),
}
