use std::fmt;

/// A package that failed an operation, with a human-readable reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedPkg {
    pub name: String,
    pub error: String,
}

impl fmt::Display for FailedPkg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.error)
    }
}

/// Uniform result of a backend operation (install/remove/upgrade/...).
///
/// - `changed`:   packages that were actually modified
/// - `unchanged`: packages already in the desired state (idempotency no-ops)
/// - `failed`:    packages that errored
#[derive(Debug, Clone, Default)]
pub struct BackendOutcome {
    pub backend: &'static str,
    pub changed: Vec<String>,
    pub unchanged: Vec<String>,
    pub failed: Vec<FailedPkg>,
    pub note: String,
}

impl BackendOutcome {
    pub fn empty(backend: &'static str) -> Self {
        Self {
            backend,
            ..Default::default()
        }
    }

    /// The backend tool is not installed on this machine.
    pub fn unavailable(backend: &'static str) -> Self {
        Self {
            backend,
            note: format!("{} not available", backend),
            ..Default::default()
        }
    }

    pub fn ok(&self) -> bool {
        self.failed.is_empty()
    }

    pub fn fail_one(&mut self, name: impl Into<String>, error: impl Into<String>) {
        self.failed.push(FailedPkg {
            name: name.into(),
            error: error.into(),
        });
    }
}
