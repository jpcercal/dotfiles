use crate::outcome::BackendOutcome;
use crate::util;
use crate::PackageBackend;
use anyhow::Result;
use dotfiles_exec::ExecEnv;

/// `brew` formula backend (spec prefix: `brew:`). Idempotent installs via
/// `brew list --formula -1` snapshot filtering.
pub struct Brew;

/// `brew --cask` backend (spec prefix: `cask:`).
pub struct BrewCask;

struct BrewKind {
    backend: &'static str,
    flags: &'static [&'static str],
}

const FORMULA: BrewKind = BrewKind {
    backend: "brew",
    flags: &["--formula"],
};
const CASK: BrewKind = BrewKind {
    backend: "cask",
    flags: &["--cask"],
};

impl BrewKind {
    fn list_installed(&self, env: &ExecEnv) -> Result<Vec<String>> {
        let args: Vec<&str> = ["list", "-1"]
            .iter()
            .chain(self.flags.iter())
            .copied()
            .collect();
        let out = env.output("brew", &args)?;
        Ok(out
            .stdout
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    }

    fn outdated(&self, env: &ExecEnv) -> Result<Vec<String>> {
        let args: Vec<&str> = ["outdated", "--quiet"]
            .iter()
            .chain(self.flags.iter())
            .copied()
            .collect();
        let out = env.output("brew", &args)?;
        let mut names: Vec<String> = out
            .stdout
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        names.sort();
        Ok(names)
    }

    fn install(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let installed = self.list_installed(env)?;
        let (todo, already) = util::filter_new(&installed, pkgs);
        let mut out = util::run_batch(env, "brew", "install", self.flags, &todo, self.backend)?;
        out.unchanged = already;
        Ok(out)
    }

    fn remove(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let installed = self.list_installed(env)?;
        let (todo, absent) = util::filter_absent(&installed, pkgs);
        let mut out = util::run_batch(env, "brew", "uninstall", self.flags, &todo, self.backend)?;
        out.unchanged = absent;
        Ok(out)
    }

    fn upgrade(&self, env: &ExecEnv) -> Result<BackendOutcome> {
        let before = self.outdated(env).unwrap_or_default();
        let args: Vec<&str> = ["upgrade"]
            .iter()
            .chain(self.flags.iter())
            .copied()
            .collect();
        let res = env.output("brew", &args)?;
        let mut out = BackendOutcome::empty(self.backend);
        if res.ok() {
            out.changed = before;
        } else {
            out.note = util::summarize_error(&res.stderr, &res.stdout);
        }
        Ok(out)
    }
}

macro_rules! brew_backend {
    ($ty:ty, $kind:expr, $name:literal) => {
        impl PackageBackend for $ty {
            fn name(&self) -> &'static str {
                $name
            }
            fn tool(&self) -> &'static str {
                "brew"
            }

            fn install(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
                $kind.install(env, pkgs)
            }
            fn remove(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
                $kind.remove(env, pkgs)
            }
            fn upgrade(&self, env: &ExecEnv) -> Result<BackendOutcome> {
                $kind.upgrade(env)
            }
            fn list_installed(&self, env: &ExecEnv) -> Result<Vec<String>> {
                $kind.list_installed(env)
            }
            fn outdated(&self, env: &ExecEnv) -> Result<Vec<String>> {
                $kind.outdated(env)
            }

            fn search(&self, env: &ExecEnv, query: &str) -> Result<Vec<String>> {
                let args: Vec<&str> = ["search"]
                    .iter()
                    .chain($kind.flags.iter())
                    .copied()
                    .chain([query])
                    .collect();
                let out = env.output("brew", &args)?;
                Ok(out
                    .stdout
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect())
            }
            fn info(&self, env: &ExecEnv, pkg: &str) -> Result<String> {
                let args: Vec<&str> = ["info"]
                    .iter()
                    .chain($kind.flags.iter())
                    .copied()
                    .chain([pkg])
                    .collect();
                let out = env.output("brew", &args)?;
                Ok(out.stdout)
            }
            fn update_index(&self, env: &ExecEnv) -> Result<BackendOutcome> {
                let mut out = BackendOutcome::empty($name);
                let res = env.output("brew", &["update"])?;
                if !res.ok() {
                    out.note = util::summarize_error(&res.stderr, &res.stdout);
                }
                Ok(out)
            }
        }
    };
}

brew_backend!(Brew, FORMULA, "brew");
brew_backend!(BrewCask, CASK, "cask");

#[cfg(test)]
mod tests {
    use super::*;
    use dotfiles_testkit::TestEnv;

    fn brew_listing(formulas: &str, casks: &str) -> String {
        // One stub handles both list variants via case on flags.
        format!(
            "case \"$*\" in \
             \"list -1 --formula\") printf '%s' '{formulas}' ;; \
             \"list -1 --cask\") printf '%s' '{casks}' ;; \
             \"outdated --quiet --formula\") printf '' ;; \
             \"outdated --quiet --cask\") printf '' ;; \
             esac\nexit 0",
        )
    }

    #[test]
    fn install_is_idempotent_via_list_snapshot() {
        let t = TestEnv::new();
        t.stub("brew", &brew_listing("git\nripgrep\n", "iterm2\n"));
        let env = t.exec().clone();

        let out = Brew.install(&env, &["git".into(), "bat".into()]).unwrap();
        assert_eq!(out.changed, vec!["bat"]);
        assert_eq!(out.unchanged, vec!["git"]);
        assert!(out.ok());
        // Exactly one install call, for the missing package only.
        assert_eq!(
            t.calls_of("brew")
                .iter()
                .filter(|c| c.starts_with("install"))
                .count(),
            1
        );
        assert!(t
            .calls_of("brew")
            .iter()
            .any(|c| c.starts_with("install --formula bat")));
        assert!(!t
            .calls_of("brew")
            .iter()
            .any(|c| c.contains("install --formula git")));
    }

    #[test]
    fn remove_skips_absent_packages() {
        let t = TestEnv::new();
        t.stub("brew", &brew_listing("git\n", ""));
        let env = t.exec().clone();
        let out = Brew.remove(&env, &["git".into(), "node".into()]).unwrap();
        assert_eq!(out.changed, vec!["git"]);
        assert_eq!(out.unchanged, vec!["node"]);
        assert!(t
            .calls_of("brew")
            .iter()
            .any(|c| c.starts_with("uninstall --formula git")));
    }

    #[test]
    fn cask_install_uses_cask_flag() {
        let t = TestEnv::new();
        t.stub("brew", &brew_listing("", ""));
        let env = t.exec().clone();
        let out = BrewCask.install(&env, &["iterm2".into()]).unwrap();
        assert_eq!(out.changed, vec!["iterm2"]);
        assert!(t
            .calls_of("brew")
            .iter()
            .any(|c| c.starts_with("install --cask iterm2")));
    }

    #[test]
    fn failed_install_reports_all_packages() {
        let t = TestEnv::new();
        t.stub(
            "brew",
            "case \"$1\" in list) echo git;; *) echo 'Error: no such formulae' 1>&2; exit 1;; esac",
        );
        let env = t.exec().clone();
        let out = Brew
            .install(&env, &["nope1".into(), "nope2".into()])
            .unwrap();
        assert!(!out.ok());
        assert_eq!(out.failed.len(), 2);
        assert!(
            out.failed[0].error.contains("no such formulae"),
            "{}",
            out.failed[0]
        );
    }

    #[test]
    fn upgrade_failure_records_note() {
        let t = TestEnv::new();
        t.stub("brew", "case \"$*\" in \"outdated --quiet --formula\") exit 0;; \"upgrade --formula\") echo 'conflict' 1>&2; exit 1;; esac; exit 0");
        let env = t.exec().clone();
        let out = Brew.upgrade(&env).unwrap();
        assert!(out.changed.is_empty());
        assert!(out.note.contains("conflict"), "{}", out.note);
    }

    #[test]
    fn search_and_info_passthrough() {
        let t = TestEnv::new();
        t.stub("brew", "case \"$1\" in search) echo 'ripgrep';; info) echo 'ripgrep: fast grep';; esac; exit 0");
        let env = t.exec().clone();
        assert_eq!(Brew.search(&env, "rg").unwrap(), vec!["ripgrep"]);
        assert!(Brew.info(&env, "ripgrep").unwrap().contains("fast grep"));
        // flags travel along
        assert!(t
            .calls_of("brew")
            .iter()
            .any(|c| c.starts_with("search --formula rg")));
    }

    #[test]
    fn upgrade_reports_previously_outdated() {
        let t = TestEnv::new();
        t.stub(
            "brew",
            "case \"$*\" in \"outdated --quiet --formula\") echo 'git'; echo 'bat';; esac; exit 0",
        );
        let env = t.exec().clone();
        let out = Brew.upgrade(&env).unwrap();
        assert_eq!(out.changed, vec!["bat", "git"]);
        assert!(t
            .calls_of("brew")
            .iter()
            .any(|c| c.starts_with("upgrade --formula")));
    }
}
