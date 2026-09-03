use crate::outcome::BackendOutcome;
use crate::util;
use crate::PackageBackend;
use anyhow::{Context, Result};
use dotfiles_exec::ExecEnv;

/// Python (uv-managed system python) packages backend (spec prefix: `pip:`).
pub struct UvPip;

impl UvPip {
    /// Resolve the uv-managed python; drives `--python` for every pip call.
    fn python(env: &ExecEnv) -> Result<String> {
        let out = env.output("uv", &["python", "find"])?;
        let p = out.stdout.trim().to_string();
        if p.is_empty() {
            anyhow::bail!("uv python find returned nothing — is the uv toolchain installed?");
        }
        Ok(p)
    }

    fn pip_json(env: &ExecEnv, extra: &[&str]) -> Result<Vec<serde_json::Value>> {
        let python = Self::python(env)?;
        let mut args: Vec<&str> = vec!["pip"];
        args.extend_from_slice(extra);
        args.extend(["--system", "--format=json", "--python", &python]);
        let out = env.output("uv", &args)?;
        Ok(serde_json::from_str(&out.stdout).unwrap_or_default())
    }

    fn names(rows: Vec<serde_json::Value>) -> Vec<String> {
        let mut names: Vec<String> = rows
            .iter()
            .filter_map(|r| r.get("name").and_then(|n| n.as_str()))
            .map(|s| s.to_string())
            .collect();
        names.sort();
        names
    }
}

impl PackageBackend for UvPip {
    fn name(&self) -> &'static str {
        "pip"
    }
    fn tool(&self) -> &'static str {
        "uv"
    }

    fn install(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let installed = self.list_installed(env)?;
        let (todo, already) = util::filter_new(&installed, pkgs);
        if todo.is_empty() {
            return Ok(BackendOutcome {
                backend: "pip",
                unchanged: already,
                ..Default::default()
            });
        }
        let python = Self::python(env).context("resolve uv python")?;
        let mut args: Vec<&str> = vec![
            "pip",
            "install",
            "--system",
            "--break-system-packages",
            "--python",
            &python,
        ];
        args.extend(todo.iter().map(|s| s.as_str()));
        let res = env.output("uv", &args)?;
        let mut out = BackendOutcome {
            backend: "pip",
            unchanged: already,
            ..Default::default()
        };
        if res.ok() {
            out.changed = todo;
        } else {
            let err = util::summarize_error(&res.stderr, &res.stdout);
            for p in &todo {
                out.fail_one(p.clone(), err.clone());
            }
        }
        Ok(out)
    }

    fn remove(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let installed = self.list_installed(env)?;
        let (todo, absent) = util::filter_absent(&installed, pkgs);
        let mut out = BackendOutcome {
            backend: "pip",
            unchanged: absent,
            ..Default::default()
        };
        for p in &todo {
            let python = Self::python(env)?;
            let res = env.output(
                "uv",
                &[
                    "pip",
                    "uninstall",
                    "--system",
                    "--break-system-packages",
                    "--python",
                    &python,
                    p,
                ],
            )?;
            if res.ok() {
                out.changed.push(p.clone());
            } else {
                out.fail_one(p.clone(), util::summarize_error(&res.stderr, &res.stdout));
            }
        }
        Ok(out)
    }

    fn upgrade(&self, env: &ExecEnv) -> Result<BackendOutcome> {
        let outdated = self.outdated(env).unwrap_or_default();
        if outdated.is_empty() {
            return Ok(BackendOutcome::empty("pip"));
        }
        let python = Self::python(env)?;
        let mut args: Vec<&str> = vec![
            "pip",
            "install",
            "--system",
            "--break-system-packages",
            "-U",
            "--python",
            &python,
        ];
        args.extend(outdated.iter().map(|s| s.as_str()));
        let res = env.output("uv", &args)?;
        let mut out = BackendOutcome::empty("pip");
        if res.ok() {
            out.changed = outdated;
        } else {
            out.note = util::summarize_error(&res.stderr, &res.stdout);
        }
        Ok(out)
    }

    fn list_installed(&self, env: &ExecEnv) -> Result<Vec<String>> {
        Ok(Self::names(Self::pip_json(env, &["list"])?))
    }

    fn outdated(&self, env: &ExecEnv) -> Result<Vec<String>> {
        Ok(Self::names(Self::pip_json(env, &["list", "--outdated"])?))
    }

    fn search(&self, _env: &ExecEnv, _query: &str) -> Result<Vec<String>> {
        // uv/pip have no repository search; PyPI dropped its XML-RPC search API.
        Ok(vec![])
    }

    fn info(&self, env: &ExecEnv, pkg: &str) -> Result<String> {
        let python = Self::python(env)?;
        let out = env.output("uv", &["pip", "show", "--system", "--python", &python, pkg])?;
        Ok(out.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UV_STUB: &str = "case \"$1 $2\" in \
      \"python find\") echo '/usr/local/bin/python3' ;; \
      \"pip list\") echo '[{\"name\":\"pynvim\"},{\"name\":\"neovim\"}]' ;; \
      esac; exit 0";

    #[test]
    fn installed_names_from_uv_json() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub("uv", UV_STUB);
        let env = t.exec().clone();
        assert_eq!(
            UvPip.list_installed(&env).unwrap(),
            vec!["neovim", "pynvim"]
        );
    }

    #[test]
    fn idempotent_install_targets_uv_python() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub("uv", UV_STUB);
        let env = t.exec().clone();
        let out = UvPip
            .install(&env, &["pynvim".into(), "requests".into()])
            .unwrap();
        assert_eq!(out.unchanged, vec!["pynvim"]);
        assert_eq!(out.changed, vec!["requests"]);
        let install_call = t
            .calls_of("uv")
            .into_iter()
            .find(|c| c.starts_with("pip install"))
            .expect("pip install call");
        assert!(
            install_call.contains("--python /usr/local/bin/python3"),
            "{}",
            install_call
        );
        assert!(install_call.ends_with("requests"), "{}", install_call);
    }
}
