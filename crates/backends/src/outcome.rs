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

/// Uniform result of a backend operation (install/uninstall/upgrade/...).
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

    /// One-line human summary for live `UnitFinished` events: the first
    /// failure, `changed`, or `already ok` (with any note appended).
    pub fn detail(&self) -> String {
        if let Some(f) = self.failed.first() {
            return f.to_string();
        }
        if !self.changed.is_empty() {
            return "changed".to_string();
        }
        if self.note.is_empty() {
            "already ok".to_string()
        } else {
            format!("already ok — {}", self.note)
        }
    }

    /// Structural twin of [`BackendOutcome::detail`] for renderer visibility
    /// decisions: `failed` (including `blocked by …` skips) → `Failed`,
    /// anything changed → `Changed`, otherwise `NoOp`.
    pub fn outcome_kind(&self) -> dotfiles_exec::UnitOutcome {
        if !self.failed.is_empty() {
            dotfiles_exec::UnitOutcome::Failed
        } else if !self.changed.is_empty() {
            dotfiles_exec::UnitOutcome::Changed
        } else {
            dotfiles_exec::UnitOutcome::NoOp
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_summarizes_outcome() {
        let mut ok = BackendOutcome::empty("brew");
        ok.unchanged.push("git".into());
        assert_eq!(ok.detail(), "already ok");
        let noted = BackendOutcome::unavailable("brew");
        assert!(
            noted.detail().contains("not available"),
            "{}",
            noted.detail()
        );
        let mut changed = BackendOutcome::empty("brew");
        changed.changed.push("git".into());
        assert_eq!(changed.detail(), "changed");
        let mut failed = BackendOutcome::empty("brew");
        failed.fail_one("git", "boom");
        assert_eq!(failed.detail(), "git: boom");
    }

    #[test]
    fn outcome_kind_classifies_outcomes() {
        use dotfiles_exec::UnitOutcome;
        let mut ok = BackendOutcome::empty("brew");
        ok.unchanged.push("git".into());
        assert_eq!(ok.outcome_kind(), UnitOutcome::NoOp);
        // Notes alone (missing tools, skips) are still no-ops to show.
        assert_eq!(
            BackendOutcome::unavailable("brew").outcome_kind(),
            UnitOutcome::NoOp
        );
        let mut changed = BackendOutcome::empty("brew");
        changed.changed.push("git".into());
        assert_eq!(changed.outcome_kind(), UnitOutcome::Changed);
        let mut failed = BackendOutcome::empty("brew");
        failed.fail_one("child", "blocked by 'fail' (not attempted)");
        assert_eq!(failed.outcome_kind(), UnitOutcome::Failed);
    }
}
