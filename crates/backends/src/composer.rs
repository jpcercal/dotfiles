use crate::outcome::BackendOutcome;
use crate::util;
use crate::PackageBackend;
use anyhow::Result;
use dotfiles_exec::ExecEnv;

/// Composer global packages backend (spec prefix: `composer:`).
pub struct Composer;

/// `composer global show -N` — one package name per line.
fn names(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && l.contains('/'))
        .collect()
}

impl PackageBackend for Composer {
    fn name(&self) -> &'static str {
        "composer"
    }
    fn tool(&self) -> &'static str {
        "composer"
    }

    fn install(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let installed = self.list_installed(env)?;
        let (todo, already) = util::filter_new(&installed, pkgs);
        let mut out = util::run_batch(
            env,
            "composer",
            "global require",
            &["--no-interaction"],
            &todo,
            "composer",
        )?;
        out.unchanged = already;
        Ok(out)
    }

    fn remove(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let installed = self.list_installed(env)?;
        let (todo, absent) = util::filter_absent(&installed, pkgs);
        let mut out = util::run_batch(
            env,
            "composer",
            "global remove",
            &["--no-interaction"],
            &todo,
            "composer",
        )?;
        out.unchanged = absent;
        Ok(out)
    }

    fn upgrade(&self, env: &ExecEnv) -> Result<BackendOutcome> {
        let before = self.outdated(env).unwrap_or_default();
        let res = env.output("composer", &["global", "update", "--no-interaction"])?;
        let mut out = BackendOutcome::empty("composer");
        if res.ok() {
            out.changed = before;
        } else {
            out.note = util::summarize_error(&res.stderr, &res.stdout);
        }
        Ok(out)
    }

    fn list_installed(&self, env: &ExecEnv) -> Result<Vec<String>> {
        let out = env.output("composer", &["global", "show", "-N", "--no-interaction"])?;
        Ok(names(&out.stdout))
    }

    fn outdated(&self, env: &ExecEnv) -> Result<Vec<String>> {
        let out = env.output(
            "composer",
            &["global", "outdated", "--format=json", "--no-interaction"],
        )?;
        let v: serde_json::Value =
            serde_json::from_str(&out.stdout).unwrap_or(serde_json::json!({}));
        let mut names = vec![];
        if let Some(arr) = v.get("installed").and_then(|x| x.as_array()) {
            for e in arr {
                if let Some(n) = e.get("name").and_then(|n| n.as_str()) {
                    names.push(n.to_string());
                }
            }
        }
        Ok(names)
    }

    fn search(&self, env: &ExecEnv, query: &str) -> Result<Vec<String>> {
        let out = env.output("composer", &["search", query, "-N"])?;
        Ok(names(&out.stdout))
    }

    fn info(&self, env: &ExecEnv, pkg: &str) -> Result<String> {
        let out = env.output("composer", &["show", pkg, "--no-interaction"])?;
        Ok(out.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrade_reports_outdated() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub(
            "composer",
            "case \"$2\" in outdated) echo '{\"installed\":[{\"name\":\"a/b\"}]}';; *) exit 0;; esac",
        );
        let env = t.exec().clone();
        let out = Composer.upgrade(&env).unwrap();
        assert_eq!(out.changed, vec!["a/b"]);
        assert!(t
            .calls_of("composer")
            .iter()
            .any(|c| c.starts_with("global update")));
    }

    #[test]
    fn upgrade_failure_records_note() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub("composer", "case \"$2\" in outdated) echo '{}';; update) echo 'killed' 1>&2; exit 1;; *) exit 0;; esac");
        let env = t.exec().clone();
        let out = Composer.upgrade(&env).unwrap();
        assert!(out.note.contains("killed"), "{}", out.note);
    }

    #[test]
    fn remove_uninstalls_present_only() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub(
            "composer",
            "case \"$1 $2 $3\" in \"global show -N\") echo 'a/b';; esac; exit 0",
        );
        let env = t.exec().clone();
        let out = Composer
            .remove(&env, &["a/b".into(), "c/d".into()])
            .unwrap();
        assert_eq!(out.changed, vec!["a/b"]);
        assert_eq!(out.unchanged, vec!["c/d"]);
    }

    #[test]
    fn search_keeps_vendor_names_only() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub_ok("composer", "a/b\nplainline\n");
        let env = t.exec().clone();
        assert_eq!(Composer.search(&env, "a").unwrap(), vec!["a/b"]);
    }

    #[test]
    fn idempotent_global_require() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub("composer", "case \"$1 $2 $3\" in \"global show -N\") echo 'friendsofphp/php-cs-fixer';; esac; exit 0");
        let env = t.exec().clone();
        let out = Composer
            .install(
                &env,
                &["friendsofphp/php-cs-fixer".into(), "phpstan/phpstan".into()],
            )
            .unwrap();
        assert_eq!(out.unchanged, vec!["friendsofphp/php-cs-fixer"]);
        assert_eq!(out.changed, vec!["phpstan/phpstan"]);
        assert!(t
            .calls_of("composer")
            .iter()
            .any(|c| c.starts_with("global require --no-interaction phpstan/phpstan")));
    }
}
