use crate::outcome::BackendOutcome;
use crate::util;
use crate::PackageBackend;
use anyhow::Result;
use dotfiles_exec::ExecEnv;

/// npm global packages backend (spec prefix: `npm:`).
pub struct Npm;

fn installed_from_json(stdout: &str) -> Vec<String> {
    let v: serde_json::Value = match serde_json::from_str(stdout) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    match v.get("dependencies").and_then(|d| d.as_object()) {
        Some(deps) => {
            let mut names: Vec<String> = deps.keys().cloned().collect();
            // npm itself is always listed at depth 0 of the prefix; it is not a
            // user-managed package here.
            names.retain(|n| n != "npm" && n != "corepack");
            names
        }
        None => vec![],
    }
}

impl PackageBackend for Npm {
    fn name(&self) -> &'static str {
        "npm"
    }
    fn tool(&self) -> &'static str {
        "npm"
    }

    fn install(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let installed = self.list_installed(env)?;
        let (todo, already) = util::filter_new(&installed, pkgs);
        let mut out = util::run_batch(env, "npm", "install", &["-g"], &todo, "npm")?;
        out.unchanged = already;
        Ok(out)
    }

    fn remove(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let installed = self.list_installed(env)?;
        let (todo, absent) = util::filter_absent(&installed, pkgs);
        let mut out = util::run_batch(env, "npm", "uninstall", &["-g"], &todo, "npm")?;
        out.unchanged = absent;
        Ok(out)
    }

    fn upgrade(&self, env: &ExecEnv) -> Result<BackendOutcome> {
        let before = self.outdated(env).unwrap_or_default();
        let res = env.output("npm", &["update", "-g"])?;
        let mut out = BackendOutcome::empty("npm");
        if res.ok() {
            out.changed = before;
        } else {
            out.note = util::summarize_error(&res.stderr, &res.stdout);
        }
        Ok(out)
    }

    fn list_installed(&self, env: &ExecEnv) -> Result<Vec<String>> {
        let out = env.output("npm", &["ls", "-g", "--depth=0", "--json"])?;
        Ok(installed_from_json(&out.stdout))
    }

    fn outdated(&self, env: &ExecEnv) -> Result<Vec<String>> {
        // `npm outdated --json` exits 1 when there are outdated packages; capture anyway.
        let out = env.output("npm", &["outdated", "-g", "--depth=0", "--json"])?;
        let v: serde_json::Value =
            serde_json::from_str(&out.stdout).unwrap_or(serde_json::json!({}));
        Ok(v.as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default())
    }

    fn search(&self, env: &ExecEnv, query: &str) -> Result<Vec<String>> {
        let out = env.output("npm", &["search", query, "--parseable"])?;
        Ok(out
            .stdout
            .lines()
            .filter_map(|l| l.split('\t').next())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect())
    }

    fn info(&self, env: &ExecEnv, pkg: &str) -> Result<String> {
        let out = env.output("npm", &["info", pkg])?;
        Ok(out.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_parsed_from_json_and_filters_npm_itself() {
        let s = r#"{"dependencies":{"npm":{"version":"10.0"},"neovim":{"version":"5.0"},"prettier":{}}}"#;
        assert_eq!(installed_from_json(s), vec!["neovim", "prettier"]);
    }

    #[test]
    fn installed_empty_on_garbage() {
        assert!(installed_from_json("not json").is_empty());
    }

    #[test]
    fn outdated_parsed_despite_exit_1() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub("npm", "case \"$1\" in outdated) echo '{\"neovim\":{\"current\":\"5.0\"}}'; exit 1;; esac; exit 0");
        let env = t.exec().clone();
        assert_eq!(Npm.outdated(&env).unwrap(), vec!["neovim"]);
    }

    #[test]
    fn upgrade_reports_previously_outdated() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub(
            "npm",
            "case \"$1\" in ls) echo '{\"dependencies\":{\"neovim\":{}}}';; outdated) echo '{\"neovim\":{}}'; exit 1;; update) exit 0;; esac; exit 0",
        );
        let env = t.exec().clone();
        let out = Npm.upgrade(&env).unwrap();
        assert_eq!(out.changed, vec!["neovim"]);
        assert!(t.calls_of("npm").iter().any(|c| c == "update -g"));
    }

    #[test]
    fn upgrade_failure_records_note() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub(
            "npm",
            "case \"$1\" in outdated) exit 0;; update) echo 'EAI_AGAIN' 1>&2; exit 1;; esac",
        );
        let env = t.exec().clone();
        let out = Npm.upgrade(&env).unwrap();
        assert!(out.note.contains("EAI_AGAIN"), "{}", out.note);
    }

    #[test]
    fn remove_uninstalls_present_only() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub(
            "npm",
            "case \"$1\" in ls) echo '{\"dependencies\":{\"neovim\":{}}}';; esac; exit 0",
        );
        let env = t.exec().clone();
        let out = Npm
            .remove(&env, &["neovim".into(), "prettier".into()])
            .unwrap();
        assert_eq!(out.changed, vec!["neovim"]);
        assert_eq!(out.unchanged, vec!["prettier"]);
        assert!(t.calls_of("npm").iter().any(|c| c == "uninstall -g neovim"));
    }

    #[test]
    fn search_splits_tsv_first_column() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub_ok("npm", "neovim\tneovim client\n");
        let env = t.exec().clone();
        assert_eq!(Npm.search(&env, "neovim").unwrap(), vec!["neovim"]);
    }

    #[test]
    fn idempotent_install() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub(
            "npm",
            "case \"$1\" in ls) echo '{\"dependencies\":{\"neovim\":{}}}';; esac; exit 0",
        );
        let env = t.exec().clone();
        let out = Npm
            .install(&env, &["neovim".into(), "prettier".into()])
            .unwrap();
        assert_eq!(out.unchanged, vec!["neovim"]);
        assert_eq!(out.changed, vec!["prettier"]);
        assert!(t
            .calls_of("npm")
            .iter()
            .any(|c| c.starts_with("install -g prettier")));
    }
}
