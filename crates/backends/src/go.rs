use crate::outcome::BackendOutcome;
use crate::util;
use crate::PackageBackend;
use anyhow::Result;
use dotfiles_exec::ExecEnv;

/// Go module binaries backend (spec prefix: `go:`). Packages are module paths
/// with version suffix, e.g. `github.com/oklog/ulid/v2/cmd/ulid@latest`.
/// Go has no cheap "installed modules" registry — idempotency uses the
/// `$GOPATH/bin`/`~/go/bin` binary names derived from the package path.
pub struct Go;

fn binary_name(module: &str) -> String {
    let path = module.split('@').next().unwrap_or(module);
    path.rsplit('/').next().unwrap_or(path).to_string()
}

impl Go {
    fn bin_dir(env: &ExecEnv) -> std::path::PathBuf {
        let out = env.output("go", &["env", "GOBIN"]).ok();
        let gobin = out.map(|o| o.stdout.trim().to_string()).unwrap_or_default();
        if !gobin.is_empty() {
            return std::path::PathBuf::from(gobin);
        }
        let out = env.output("go", &["env", "GOPATH"]).ok();
        let gopath = out.map(|o| o.stdout.trim().to_string()).unwrap_or_default();
        if !gopath.is_empty() {
            return std::path::PathBuf::from(gopath).join("bin");
        }
        env.home.join("go/bin")
    }
}

impl PackageBackend for Go {
    fn name(&self) -> &'static str {
        "go"
    }
    fn tool(&self) -> &'static str {
        "go"
    }

    fn install(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let installed = self.list_installed(env)?;
        let bin_dir = Self::bin_dir(env);
        let mut out = BackendOutcome::empty("go");
        for p in pkgs {
            if installed.contains(&binary_name(p)) && bin_dir.join(binary_name(p)).exists() {
                out.unchanged.push(p.clone());
                continue;
            }
            let res = env.output("go", &["install", p])?;
            if res.ok() {
                out.changed.push(p.clone());
            } else {
                out.fail_one(p.clone(), util::summarize_error(&res.stderr, &res.stdout));
            }
        }
        Ok(out)
    }

    fn remove(&self, env: &ExecEnv, pkgs: &[String]) -> Result<BackendOutcome> {
        let mut out = BackendOutcome::empty("go");
        let bin_dir = Self::bin_dir(env);
        for p in pkgs {
            let bin = bin_dir.join(binary_name(p));
            if bin.exists() {
                match std::fs::remove_file(&bin) {
                    Ok(_) => out.changed.push(p.clone()),
                    Err(e) => out.fail_one(p.clone(), e.to_string()),
                }
            } else {
                out.unchanged.push(p.clone());
            }
        }
        Ok(out)
    }

    fn upgrade(&self, env: &ExecEnv) -> Result<BackendOutcome> {
        // Reinstall every known binary at @latest.
        let bins = self.list_installed(env)?;
        let pkgs: Vec<String> = bins.iter().map(|b| format!("{}@latest", b)).collect();
        let mut out = BackendOutcome::empty("go");
        out.note =
            "go binaries reinstall from their module paths only when tracked; skipping".into();
        let _ = pkgs;
        Ok(out)
    }

    fn list_installed(&self, env: &ExecEnv) -> Result<Vec<String>> {
        let bin = Self::bin_dir(env);
        let mut names = vec![];
        if let Ok(entries) = std::fs::read_dir(&bin) {
            for e in entries.flatten() {
                if let Some(n) = e.file_name().to_str() {
                    names.push(n.to_string());
                }
            }
        }
        names.sort();
        Ok(names)
    }

    fn outdated(&self, _env: &ExecEnv) -> Result<Vec<String>> {
        Ok(vec![])
    }

    fn search(&self, _env: &ExecEnv, _query: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }

    fn info(&self, env: &ExecEnv, pkg: &str) -> Result<String> {
        let out = env.output(
            "go",
            &[
                "version",
                "-m",
                &Self::bin_dir(env).join(binary_name(pkg)).to_string_lossy(),
            ],
        )?;
        Ok(out.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_name_strips_version_suffix_and_dirs() {
        assert_eq!(
            binary_name("github.com/oklog/ulid/v2/cmd/ulid@latest"),
            "ulid"
        );
        assert_eq!(binary_name("github.com/foo/bar"), "bar");
    }

    #[test]
    fn remove_deletes_existing_binary() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub("go", "case \"$*\" in \"env GOPATH\") echo \"$HOME/gopath\" ;; \"env GOBIN\") echo '' ;; esac; exit 0");
        let bin = t.home().join("gopath/bin/tool");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"").unwrap();
        let env = t.exec().clone();
        let out = Go
            .remove(
                &env,
                &[
                    "example.com/x/tool@latest".into(),
                    "example.com/y/absent@latest".into(),
                ],
            )
            .unwrap();
        assert_eq!(out.changed, vec!["example.com/x/tool@latest"]);
        assert_eq!(out.unchanged, vec!["example.com/y/absent@latest"]);
        assert!(!bin.exists());
    }

    #[test]
    fn upgrade_is_documented_noop() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub_ok("go", "");
        let env = t.exec().clone();
        let out = Go.upgrade(&env).unwrap();
        assert!(out.ok());
        assert!(!out.note.is_empty());
    }

    #[test]
    fn install_skips_when_binary_exists() {
        let t = dotfiles_testkit::TestEnv::new();
        t.stub("go", "case \"$*\" in \"env GOBIN\") echo '' ;; \"env GOPATH\") printf '%s/gopath' \"$HOME\" ;; esac; exit 0");
        std::fs::create_dir_all(t.home().join("gopath/bin")).unwrap();
        std::fs::write(t.home().join("gopath/bin/ulid"), b"").unwrap();
        let env = t
            .exec()
            .clone()
            .with_env("HOME", &t.home().to_string_lossy());
        let out = Go
            .install(
                &env,
                &[
                    "github.com/oklog/ulid/v2/cmd/ulid@latest".into(),
                    "example.com/x/tool@latest".into(),
                ],
            )
            .unwrap();
        assert_eq!(out.unchanged.len(), 1);
        assert_eq!(out.changed, vec!["example.com/x/tool@latest"]);
        assert!(t
            .calls_of("go")
            .iter()
            .any(|c| c == "install example.com/x/tool@latest"));
        assert!(!t.calls_of("go").iter().any(|c| c.contains("ulid")));
    }
}
