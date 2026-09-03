use crate::outcome::BackendOutcome;
use crate::util;
use crate::PackageBackend;
use anyhow::Result;
use dotfiles_exec::ExecEnv;

/// Ruby gems backend (spec prefix: `gem:`).
pub struct Gem;

/// `gem list --local` first-column names (skips "*** LOCAL GEMS ***" headers).
fn gem_names(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter(|l| !l.trim_start().starts_with('*'))
        .filter_map(|l| l.split_whitespace().next())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

impl PackageBackend for Gem {
    fn name(&self) -> &'static str {
        "gem"
    }
    fn tool(&self) -> &'static str {
        "gem"
    }

    fn install(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let installed = self.list_installed(env)?;
        let (todo, already) = util::filter_new(&installed, pkgs);
        let mut out = util::run_batch(env, "gem", "install", &["--no-document"], &todo, "gem")?;
        out.unchanged = already;
        Ok(out)
    }

    fn remove(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let installed = self.list_installed(env)?;
        let (todo, absent) = util::filter_absent(&installed, pkgs);
        let mut out = util::run_batch(env, "gem", "uninstall", &["-x", "-a", "-I"], &todo, "gem")?;
        out.unchanged = absent;
        Ok(out)
    }

    fn upgrade(&self, env: &ExecEnv) -> Result<BackendOutcome> {
        let before = self.outdated(env).unwrap_or_default();
        let res = env.output("gem", &["update", "--no-document"])?;
        let mut out = BackendOutcome::empty("gem");
        if res.ok() {
            out.changed = before;
        } else {
            out.note = util::summarize_error(&res.stderr, &res.stdout);
        }
        Ok(out)
    }

    fn list_installed(&self, env: &ExecEnv) -> Result<Vec<String>> {
        let out = env.output("gem", &["list", "--local"])?;
        Ok(gem_names(&out.stdout))
    }

    fn outdated(&self, env: &ExecEnv) -> Result<Vec<String>> {
        let out = env.output("gem", &["outdated"])?;
        Ok(gem_names(&out.stdout))
    }

    fn search(&self, env: &ExecEnv, query: &str) -> Result<Vec<String>> {
        let out = env.output("gem", &["search", query])?;
        Ok(out
            .stdout
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    }

    fn info(&self, env: &ExecEnv, pkg: &str) -> Result<String> {
        let out = env.output("gem", &["info", pkg])?;
        Ok(out.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gem_list_names() {
        let s = "neovim (0.9.0)\nrake (13.0.6, 12.3.3)\n*** LOCAL GEMS ***\n";
        assert_eq!(gem_names(s), vec!["neovim", "rake"]);
    }

    #[test]
    fn idempotent_install() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub(
            "gem",
            "case \"$1\" in list) echo 'neovim (0.9.0)';; esac; exit 0",
        );
        let env = t.exec().clone();
        let out = Gem
            .install(&env, &["neovim".into(), "rake".into()])
            .unwrap();
        assert_eq!(out.unchanged, vec!["neovim"]);
        assert_eq!(out.changed, vec!["rake"]);
        assert!(t
            .calls_of("gem")
            .iter()
            .any(|c| c.starts_with("install --no-document rake")));
    }
}
