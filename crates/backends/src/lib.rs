pub mod bootstrap;
mod brew;
mod cargo;
mod composer;
mod gem;
mod go;
mod mas;
mod npm;
pub mod orchestrate;
pub mod outcome;
mod pip;
pub mod spec;
pub mod toolchain;

pub use outcome::{BackendOutcome, FailedPkg};
pub use spec::{Spec, SpecError};

use anyhow::Result;
use dotfiles_exec::ExecEnv;

/// One package ecosystem (apt/brew/mas/composer/cargo/npm/maven-style backend).
///
/// Contract: all mutating operations MUST be idempotent — `install` of an
/// already-installed package is a no-op recorded in `unchanged`, `remove` of an
/// absent package likewise. This makes `dotfiles sync` safely re-runnable.
pub trait PackageBackend: Send + Sync {
    /// Backend key used in specs (`brew`, `cask`, `mas`, ...).
    fn name(&self) -> &'static str;

    /// The external tool this backend drives (`brew`, `mas`, `gem`, ...).
    fn tool(&self) -> &'static str;

    fn is_available(&self, env: &ExecEnv) -> bool {
        env.has_command(self.tool())
    }

    fn install(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome>;
    fn remove(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome>;
    fn upgrade(&self, env: &ExecEnv) -> Result<BackendOutcome>;
    fn list_installed(&self, env: &ExecEnv) -> Result<Vec<String>>;
    fn outdated(&self, env: &ExecEnv) -> Result<Vec<String>>;
    fn search(&self, env: &ExecEnv, query: &str) -> Result<Vec<String>>;
    fn info(&self, env: &ExecEnv, pkg: &str) -> Result<String>;

    /// Refresh package metadata (`brew update`, ...). Default: nothing to do.
    fn update_index(&self, env: &ExecEnv) -> Result<BackendOutcome> {
        let _ = env;
        Ok(BackendOutcome::empty(self.name()))
    }
}

/// Every backend, in deterministic order.
pub fn all_backends() -> Vec<Box<dyn PackageBackend>> {
    vec![
        Box::new(brew::Brew),
        Box::new(brew::BrewCask),
        Box::new(mas::Mas),
        Box::new(gem::Gem),
        Box::new(npm::Npm),
        Box::new(pip::UvPip),
        Box::new(cargo::Cargo),
        Box::new(go::Go),
        Box::new(composer::Composer),
    ]
}

pub fn known_backend_names() -> Vec<&'static str> {
    vec![
        "brew", "cask", "mas", "gem", "npm", "pip", "cargo", "go", "composer",
    ]
}

pub fn by_name(name: &str) -> Option<Box<dyn PackageBackend>> {
    all_backends().into_iter().find(|b| b.name() == name)
}

/// Shared helpers for backend implementations.
pub(crate) mod util {
    use crate::BackendOutcome;
    use anyhow::Result;
    use dotfiles_exec::ExecEnv;

    /// Generic idempotent line-based install/remove for tools whose
    /// "installed" output is one package name per line.
    pub fn filter_new(installed: &[String], wanted: &[String]) -> (Vec<String>, Vec<String>) {
        let installed: std::collections::BTreeSet<&String> = installed.iter().collect();
        let mut to_install = Vec::new();
        let mut already = Vec::new();
        for p in wanted {
            if installed.contains(p) {
                already.push(p.clone());
            } else {
                to_install.push(p.clone());
            }
        }
        (to_install, already)
    }

    pub fn filter_absent(installed: &[String], wanted: &[String]) -> (Vec<String>, Vec<String>) {
        let installed: std::collections::BTreeSet<&String> = installed.iter().collect();
        let mut to_remove = Vec::new();
        let mut absent = Vec::new();
        for p in wanted {
            if installed.contains(p) {
                to_remove.push(p.clone());
            } else {
                absent.push(p.clone());
            }
        }
        (to_remove, absent)
    }

    /// Run `tool <verb> <pkgs...>`; one-shot, attributing failure to all pkgs.
    pub fn run_batch(
        env: &ExecEnv,
        tool: &str,
        verb: &str,
        extra_args: &[&str],
        pkgs: &[String],
        backend: &'static str,
    ) -> Result<BackendOutcome> {
        let mut out = BackendOutcome::empty(backend);
        if pkgs.is_empty() {
            return Ok(out);
        }
        let args: Vec<&str> = verb
            .split(' ')
            .chain(extra_args.iter().copied())
            .chain(pkgs.iter().map(|s| s.as_str()))
            .collect();
        let res = env.output(tool, &args)?;
        if res.ok() {
            out.changed = pkgs.to_vec();
        } else {
            let err = summarize_error(&res.stderr, &res.stdout);
            for p in pkgs {
                out.fail_one(p.clone(), err.clone());
            }
        }
        Ok(out)
    }

    pub fn summarize_error(stderr: &str, stdout: &str) -> String {
        let from = if stderr.trim().is_empty() {
            stdout
        } else {
            stderr
        };
        from.lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("unknown error")
            .trim()
            .to_string()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn filters_partition_new_and_absent() {
            let installed = vec!["a".to_string(), "b".to_string()];
            let (todo, already) = filter_new(&installed, &["a".into(), "c".into()]);
            assert_eq!(todo, vec!["c"]);
            assert_eq!(already, vec!["a"]);
            let (todo, absent) = filter_absent(&installed, &["b".into(), "z".into()]);
            assert_eq!(todo, vec!["b"]);
            assert_eq!(absent, vec!["z"]);
        }

        #[test]
        fn summarize_error_prefers_stderr_last_line() {
            assert_eq!(summarize_error("boom\n", ""), "boom");
            assert_eq!(summarize_error("", "out\ntail\n"), "tail");
            assert_eq!(summarize_error("", ""), "unknown error");
        }

        #[test]
        fn run_batch_empty_is_noop_and_failure_attributes_all() {
            let t = dotfiles_testkit::TestEnv::new();
            t.stub_fail("gem", 1);
            let empty = run_batch(t.exec(), "gem", "install", &[], &[], "gem").unwrap();
            assert!(empty.ok() && empty.changed.is_empty());
            assert!(t.calls_of("gem").is_empty());
            t.stub("gem", "echo 'nope' 1>&2; exit 1");
            let failed = run_batch(
                t.exec(),
                "gem",
                "install",
                &[],
                &["a".into(), "b".into()],
                "gem",
            )
            .unwrap();
            assert!(!failed.ok());
            assert_eq!(failed.failed.len(), 2);
        }
    }
}
