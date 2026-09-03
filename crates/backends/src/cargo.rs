use crate::outcome::BackendOutcome;
use crate::util;
use crate::PackageBackend;
use anyhow::Result;
use dotfiles_exec::ExecEnv;

/// Cargo globally-installed binaries backend (spec prefix: `cargo:`).
pub struct Cargo;

/// `cargo install --list` — crate names are the non-indented lines: "name v1.2.3:"
fn installed_names(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter(|l| !l.starts_with(' ') && !l.starts_with('\t'))
        .filter_map(|l| l.split_whitespace().next())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

impl PackageBackend for Cargo {
    fn name(&self) -> &'static str {
        "cargo"
    }
    fn tool(&self) -> &'static str {
        "cargo"
    }

    fn install(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let installed = self.list_installed(env)?;
        let (todo, already) = util::filter_new(&installed, pkgs);
        let mut out = BackendOutcome {
            backend: "cargo",
            unchanged: already,
            ..Default::default()
        };
        for p in &todo {
            let res = env.output("cargo", &["install", p])?;
            if res.ok() {
                out.changed.push(p.clone());
            } else {
                out.fail_one(p.clone(), util::summarize_error(&res.stderr, &res.stdout));
            }
        }
        Ok(out)
    }

    fn remove(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let installed = self.list_installed(env)?;
        let (todo, absent) = util::filter_absent(&installed, pkgs);
        let mut out = util::run_batch(env, "cargo", "uninstall", &[], &todo, "cargo")?;
        out.unchanged = absent;
        Ok(out)
    }

    fn upgrade(&self, env: &ExecEnv) -> Result<BackendOutcome> {
        let mut out = BackendOutcome::empty("cargo");
        // Uses cargo-update when present (the updater's historical behavior);
        // plain `cargo install` of every known crate otherwise.
        let has_update = env
            .output("cargo", &["install-update", "--version"])
            .map(|o| o.ok())
            .unwrap_or(false);
        let res = if has_update {
            env.output("cargo", &["install-update", "-a"])?
        } else {
            let installed = self.list_installed(env)?;
            if installed.is_empty() {
                return Ok(out);
            }
            let args: Vec<&str> = std::iter::once("install")
                .chain(installed.iter().map(|s| s.as_str()))
                .collect();
            env.output("cargo", &args)?
        };
        if res.ok() {
            out.changed = self.outdated(env).unwrap_or_default();
        } else {
            out.note = util::summarize_error(&res.stderr, &res.stdout);
        }
        Ok(out)
    }

    fn list_installed(&self, env: &ExecEnv) -> Result<Vec<String>> {
        let out = env.output("cargo", &["install", "--list"])?;
        Ok(installed_names(&out.stdout))
    }

    fn outdated(&self, env: &ExecEnv) -> Result<Vec<String>> {
        let out = env.output("cargo", &["install-update", "--list"])?;
        if !out.ok() {
            return Ok(vec![]);
        }
        // Lines like: "ripgrep  v13.0.0  v14.0.0  Yes"
        Ok(out
            .stdout
            .lines()
            .filter(|l| l.trim_end().ends_with("Yes"))
            .filter_map(|l| l.split_whitespace().next())
            .map(|s| s.to_string())
            .collect())
    }

    fn search(&self, env: &ExecEnv, query: &str) -> Result<Vec<String>> {
        let out = env.output("cargo", &["search", query, "--limit", "20"])?;
        Ok(out
            .stdout
            .lines()
            .filter_map(|l| l.split_whitespace().next())
            .map(|s| s.to_string())
            .collect())
    }

    fn info(&self, env: &ExecEnv, pkg: &str) -> Result<String> {
        let out = env.output("cargo", &["search", pkg, "--limit", "1"])?;
        Ok(out.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cargo_install_list() {
        let s = "ripgrep v14.1.0:\n    rg\nulid v1.1.3 (https://github.com/x/y#abc):\n    ulid\n";
        assert_eq!(installed_names(s), vec!["ripgrep", "ulid"]);
    }

    #[test]
    fn idempotent_install_one_by_one() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub(
            "cargo",
            "case \"$2\" in --list) echo 'ripgrep v14:';; esac; exit 0",
        );
        let env = t.exec().clone();
        let out = Cargo
            .install(&env, &["ripgrep".into(), "ulid".into()])
            .unwrap();
        assert_eq!(out.unchanged, vec!["ripgrep"]);
        assert_eq!(out.changed, vec!["ulid"]);
        assert!(t.calls_of("cargo").iter().any(|c| c == "install ulid"));
    }
}
