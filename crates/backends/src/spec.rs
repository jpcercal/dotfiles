use std::fmt;
use thiserror::Error;

/// A `backend:name` package reference, e.g. `brew:git`, `cask:iterm2`,
/// `mas:1352778147`. A bare name defaults to the `brew` (formula) backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub backend: String,
    pub name: String,
}

#[derive(Error, Debug)]
pub enum SpecError {
    #[error("empty package name")]
    EmptyName,
    #[error("unknown backend '{0}' (known: brew, cask, mas, gem, npm, pip, cargo, go, composer)")]
    UnknownBackend(String),
}

impl Spec {
    pub const DEFAULT_BACKEND: &'static str = "brew";

    pub fn parse(input: &str) -> Result<Self, SpecError> {
        let input = input.trim();
        let (backend, name) = match input.split_once(':') {
            Some((b, n)) => (normalize_alias(b), n.trim()),
            None => (Self::DEFAULT_BACKEND.to_string(), input),
        };
        if name.is_empty() {
            return Err(SpecError::EmptyName);
        }
        // Taps are ensured (`dotfiles bootstrap`), never installed.
        if backend == "brew-tap" {
            return Err(SpecError::UnknownBackend(
                "brew-tap (taps are ensured with `dotfiles bootstrap`, not installed)".to_string(),
            ));
        }
        if !crate::known_backend_names().contains(&backend.as_str()) {
            return Err(SpecError::UnknownBackend(backend));
        }
        Ok(Self {
            backend,
            name: name.to_string(),
        })
    }
}

/// Accept aliases: `formula`→`brew`, `formula/cask` spelled out, and the
/// canonical dependency-graph prefixes `brew-formula:`/`brew-cask:` (so
/// `requires:` entries, `install` args and `verify` output spell a package
/// identically everywhere).
fn normalize_alias(b: &str) -> String {
    match b.trim().to_ascii_lowercase().as_str() {
        "formula" | "homebrew" | "brew-formula" => "brew".to_string(),
        "brew-cask" => "cask".to_string(),
        "mas" => "mas".to_string(),
        other => other.to_string(),
    }
}

impl fmt::Display for Spec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.backend, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_name_defaults_to_brew() {
        let s = Spec::parse("ripgrep").unwrap();
        assert_eq!(s.backend, "brew");
        assert_eq!(s.name, "ripgrep");
    }

    #[test]
    fn prefixed_specs() {
        assert_eq!(Spec::parse("cask:iterm2").unwrap().backend, "cask");
        assert_eq!(Spec::parse("mas:1352778147").unwrap().name, "1352778147");
        assert_eq!(Spec::parse("formula:git").unwrap().backend, "brew");
    }

    #[test]
    fn graph_prefix_aliases() {
        // Canonical dependency-graph IDs are accepted as install specs.
        let s = Spec::parse("brew-formula:git").unwrap();
        assert_eq!((s.backend.as_str(), s.name.as_str()), ("brew", "git"));
        let s = Spec::parse("brew-cask:iterm2").unwrap();
        assert_eq!((s.backend.as_str(), s.name.as_str()), ("cask", "iterm2"));
        let err = Spec::parse("brew-tap:hashicorp/tap").unwrap_err();
        assert!(err.to_string().contains("bootstrap"), "{}", err);
    }

    #[test]
    fn rejects_unknown_backend() {
        assert!(matches!(
            Spec::parse("apt:vim"),
            Err(SpecError::UnknownBackend(_))
        ));
    }

    #[test]
    fn rejects_empty_name() {
        assert!(matches!(Spec::parse("brew:"), Err(SpecError::EmptyName)));
        assert!(matches!(Spec::parse(""), Err(SpecError::EmptyName)));
    }
}
