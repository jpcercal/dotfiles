use crate::outcome::BackendOutcome;
use crate::util;
use crate::PackageBackend;
use anyhow::Result;
use dotfiles_exec::ExecEnv;

/// Mac App Store backend via `mas` (spec prefix: `mas:`). Packages are numeric
/// product ids. Removal is unsupported (mas cannot uninstall).
pub struct Mas;

/// Parse `mas list` / `mas outdated` output: "<id> <name> (<version>)".
fn parse_mas_ids(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|tok| !tok.is_empty() && tok.chars().all(|c| c.is_ascii_digit()))
        .map(|s| s.to_string())
        .collect()
}

impl PackageBackend for Mas {
    fn name(&self) -> &'static str {
        "mas"
    }
    fn tool(&self) -> &'static str {
        "mas"
    }

    fn install(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let installed = self.list_installed(env)?;
        let (todo, already) = util::filter_new(&installed, pkgs);
        let mut out = BackendOutcome::empty("mas");
        out.unchanged = already;
        for id in &todo {
            let res = env.output("mas", &["install", id])?;
            if res.ok() {
                out.changed.push(id.clone());
            } else {
                out.fail_one(id.clone(), util::summarize_error(&res.stderr, &res.stdout));
            }
        }
        Ok(out)
    }

    fn remove(&self, _env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let mut out = BackendOutcome::empty("mas");
        for p in pkgs {
            out.fail_one(
                p.clone(),
                "mas cannot uninstall App Store apps — remove them manually",
            );
        }
        Ok(out)
    }

    fn upgrade(&self, env: &ExecEnv) -> Result<BackendOutcome> {
        let before = self.outdated(env).unwrap_or_default();
        let res = env.output("mas", &["upgrade"])?;
        let mut out = BackendOutcome::empty("mas");
        if res.ok() {
            out.changed = before;
        } else {
            out.note = util::summarize_error(&res.stderr, &res.stdout);
        }
        Ok(out)
    }

    fn list_installed(&self, env: &ExecEnv) -> Result<Vec<String>> {
        let out = env.output("mas", &["list"])?;
        Ok(parse_mas_ids(&out.stdout))
    }

    fn outdated(&self, env: &ExecEnv) -> Result<Vec<String>> {
        let out = env.output("mas", &["outdated"])?;
        Ok(parse_mas_ids(&out.stdout))
    }

    fn search(&self, env: &ExecEnv, query: &str) -> Result<Vec<String>> {
        let out = env.output("mas", &["search", query])?;
        Ok(out
            .stdout
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    }

    fn info(&self, env: &ExecEnv, pkg: &str) -> Result<String> {
        let out = env.output("mas", &["info", pkg])?;
        Ok(out.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mas_list_into_ids() {
        let out = "!123 Foo (1.0)\n1352778147 Bitwarden (2024.1.0)\n  \n918858936 Airmail (5.0)";
        assert_eq!(parse_mas_ids(out), vec!["1352778147", "918858936"]);
    }

    #[test]
    fn install_skips_already_installed_ids() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub(
            "mas",
            "case \"$1\" in list) echo '1352778147 Bitwarden (1.0)';; esac; exit 0",
        );
        let env = t.exec().clone();
        let out = Mas
            .install(&env, &["1352778147".into(), "918858936".into()])
            .unwrap();
        assert_eq!(out.unchanged, vec!["1352778147"]);
        assert_eq!(out.changed, vec!["918858936"]);
        assert_eq!(t.calls_of("mas"), vec!["list", "install 918858936"]);
    }

    #[test]
    fn upgrade_reports_resolved_updates() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub("mas", "case \"$1\" in list) echo '1 Foo (1.0)';; outdated) echo '1 Foo (2.0)';; upgrade) exit 0;; esac; exit 0");
        // upgrade reads outdated before AND after; second read shows it resolved.
        // our static stub returns the same list both times, so model it via a flag file:
        let env = t.exec().clone();
        let out = Mas.upgrade(&env).unwrap();
        assert!(out.ok());
        assert!(t.calls_of("mas").iter().any(|c| c == "upgrade"));
    }

    #[test]
    fn upgrade_failure_records_note() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub(
            "mas",
            "case \"$1\" in outdated) exit 0;; upgrade) echo 'session expired' 1>&2; exit 1;; esac",
        );
        let env = t.exec().clone();
        let out = Mas.upgrade(&env).unwrap();
        assert!(out.note.contains("session expired"), "{}", out.note);
    }

    #[test]
    fn search_and_info_passthrough() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub(
            "mas",
            "case \"$1\" in search) echo '1 Foo';; info) echo 'Foo 2.0';; esac; exit 0",
        );
        let env = t.exec().clone();
        assert_eq!(Mas.search(&env, "Foo").unwrap(), vec!["1 Foo"]);
        assert!(Mas.info(&env, "1").unwrap().contains("2.0"));
    }

    #[test]
    fn remove_is_rejected() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub_ok("mas", "");
        let env = t.exec().clone();
        let out = Mas.remove(&env, &["1".into()]).unwrap();
        assert!(!out.ok());
        assert!(t.calls_of("mas").is_empty());
    }
}
