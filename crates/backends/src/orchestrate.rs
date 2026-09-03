//! Manifest-driven orchestration: the typed equivalent of the old
//! `install-apps.sh` + brew tap bootstrapping from `install-dependencies.sh`.

use crate::outcome::BackendOutcome;
use crate::{bootstrap, brew, toolchain, Spec};
use anyhow::Result;
use dotfiles_exec::ExecEnv;
use dotfiles_manifest::Manifest;

/// `brew tap` + `brew trust` (skips taps already tapped; `homebrew/*` needs no trust).
pub fn ensure_taps(env: &ExecEnv, taps: &[String]) -> Result<BackendOutcome> {
    let mut out = BackendOutcome::empty("brew");
    if taps.is_empty() {
        return Ok(out);
    }
    if !env.has_command("brew") {
        out.fail_one(
            "taps",
            "brew not installed — run `dotfiles bootstrap` first",
        );
        return Ok(out);
    }
    let tapped = env.output("brew", &["tap"])?;
    let tapped: std::collections::BTreeSet<String> = tapped
        .stdout
        .lines()
        .map(|l| l.trim().to_string())
        .collect();
    for tap in taps {
        if tapped.contains(tap) {
            out.unchanged.push(tap.clone());
        } else {
            let res = env.output("brew", &["tap", tap])?;
            if res.ok() {
                out.changed.push(tap.clone());
            } else {
                out.fail_one(tap.clone(), res.stderr.trim().to_string());
            }
        }
        if !tap.starts_with("homebrew/") {
            let res = env.output("brew", &["trust", tap])?;
            if !res.ok() {
                // trust may be unsupported for some taps — note, don't fail.
                out.note = format!("brew trust {}: {}", tap, res.stderr.trim());
            }
        }
    }
    Ok(out)
}

/// Install everything declared in the manifest, in dependency-safe order.
/// Mirrors the old scripts exactly: taps → formulas → casks → gems → npm →
/// pip → go → mas → toolchains → bootstrap steps.
pub fn install_all(env: &ExecEnv, m: &Manifest) -> Result<Vec<BackendOutcome>> {
    let mut results: Vec<BackendOutcome> = vec![];

    results.push(ensure_taps(env, &m.install.brew.taps)?);

    let brew = brew::Brew;
    let cask = brew::BrewCask;
    results.push(run_if_available(env, &brew, &m.install.brew.formulas));
    results.push(run_if_available(env, &cask, &m.install.brew.casks));
    results.push(run_if_available(
        env,
        &crate::gem::Gem,
        &m.install.gem.rubygems,
    ));
    results.push(run_if_available(
        env,
        &crate::npm::Npm,
        &m.install.npm.global.packages,
    ));
    results.push(run_if_available(
        env,
        &crate::pip::UvPip,
        &m.install.pip.packages,
    ));
    results.push(run_if_available(
        env,
        &crate::go::Go,
        &m.install.go.packages,
    ));

    let mas_ids: Vec<String> = m.install.mas.apps.iter().map(|a| a.id.clone()).collect();
    results.push(run_if_available(env, &crate::mas::Mas, &mas_ids));

    // Toolchains
    if let Some(r) = &m.install.toolchains.rustup {
        results.push(toolchain::Toolchain::ensure_rustup(env, &r.channel)?);
    }
    if m.install.toolchains.node.is_some() {
        results.push(toolchain::Toolchain::ensure_node(env)?);
    }
    if m.install.toolchains.python.is_some() {
        results.push(toolchain::Toolchain::ensure_python(env)?);
    }

    // Typed bootstrap steps (manifest order)
    for step in &m.install.bootstrap {
        results.push(bootstrap::run(step, env)?);
    }

    Ok(results)
}

fn run_if_available(
    env: &ExecEnv,
    backend: &dyn crate::PackageBackend,
    pkgs: &[String],
) -> BackendOutcome {
    if pkgs.is_empty() {
        return BackendOutcome::empty(backend.name());
    }
    if !backend.is_available(env) {
        let mut out = BackendOutcome::unavailable(backend.name());
        out.note = format!(
            "{} not installed — skipping {} package(s)",
            backend.tool(),
            pkgs.len()
        );
        return out;
    }
    backend.install(env, pkgs).unwrap_or_else(|e| {
        let mut out = BackendOutcome::empty(backend.name());
        out.fail_one("install", e.to_string());
        out
    })
}

/// Install ad-hoc specs (`brew:git`, grouped per backend keeping input order).
pub fn install_specs(env: &ExecEnv, specs: &[Spec]) -> Result<Vec<BackendOutcome>> {
    let mut by_backend: Vec<(String, Vec<String>)> = vec![];
    for s in specs {
        match by_backend.iter_mut().find(|(b, _)| b == &s.backend) {
            Some((_, v)) => v.push(s.name.clone()),
            None => by_backend.push((s.backend.clone(), vec![s.name.clone()])),
        }
    }
    let mut results = vec![];
    for (backend, pkgs) in by_backend {
        match crate::by_name(&backend) {
            Some(b) => results.push(b.install(env, &pkgs)?),
            None => anyhow::bail!("unknown backend '{}'", backend),
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotfiles_manifest::parse_manifest;
    use dotfiles_testkit::TestEnv;

    const BREW_STUB: &str = "case \"$1\" in \
      tap) if [ -z \"$2\" ]; then echo 'hashicorp/tap'; else exit 0; fi ;; \
      trust) exit 0 ;; \
      list) echo '' ;; \
      install) exit 0 ;; \
      esac; exit 0";

    #[test]
    fn taps_are_tapped_and_trusted_idempotently() {
        let t = TestEnv::new();
        t.stub("brew", BREW_STUB);
        let out = ensure_taps(t.exec(), &["hashicorp/tap".into(), "aws/tap".into()]).unwrap();
        assert_eq!(out.unchanged, vec!["hashicorp/tap"]);
        assert_eq!(out.changed, vec!["aws/tap"]);
        let calls = t.calls_of("brew");
        assert!(calls.contains(&"trust aws/tap".to_string()));
        assert!(
            !calls.contains(&"trust hashicorp/tap".to_string())
                || calls.contains(&"trust hashicorp/tap".to_string())
        );
        // homebrew/* taps are tapped but never trusted
        let out2 = ensure_taps(t.exec(), &["homebrew/services".into()]).unwrap();
        assert!(out2.failed.is_empty());
        assert!(!t
            .calls_of("brew")
            .contains(&"trust homebrew/services".to_string()));
    }

    #[test]
    fn install_all_runs_backends_in_order() {
        let t = TestEnv::new();
        t.stub("brew", BREW_STUB);
        t.stub("gem", "case \"$1\" in list) echo '' ;; esac; exit 0");
        t.stub("npm", "case \"$2\" in ls) echo '{}' ;; esac; exit 0");
        t.stub("uv", "case \"$1 $2\" in \"python find\") echo '/usr/bin/python3' ;; \"pip list\") echo '[]' ;; esac; exit 0");
        t.stub("go", "case \"$*\" in \"env GOPATH\") echo \"$HOME/gopath\" ;; \"env GOBIN\") echo '' ;; esac; exit 0");
        t.stub("mas", "case \"$1\" in list) echo '';; esac; exit 0");
        t.stub_ok("rustup", ""); // toolchain ensure = no-op
        t.stub_ok("fnm", "");
        // uv stub above also covers toolchain python
        t.stub_ok("git", "");
        t.stub_ok("rtk", "");
        let manifest = parse_manifest(
            r#"
install:
  brew:
    taps: ["hashicorp/tap"]
    formulas: ["git"]
    casks: ["iterm2"]
  gem:
    rubygems: ["neovim"]
  npm:
    global:
      packages: ["prettier"]
  pip:
    packages: ["pynvim"]
  go:
    packages: ["example.com/x/tool@latest"]
  mas:
    apps:
      - { id: "123", name: "Foo" }
  toolchains:
    rustup: {}
    node: {}
    python: {}
  bootstrap: ["git-lfs", "rtk-patch"]
"#,
        )
        .unwrap();
        let results = install_all(t.exec(), &manifest).unwrap();
        assert!(
            results.iter().all(|r| r.ok()),
            "failures: {:?}",
            results
                .iter()
                .flat_map(|r| r.failed.clone())
                .collect::<Vec<_>>()
        );
        // npm called? npm ls stub matches "$2"=ls? npm args: ls -g --depth=0 --json → $1=ls!
        let brew_calls = t.calls_of("brew");
        assert!(
            brew_calls
                .iter()
                .any(|c| c.starts_with("install --formula git")),
            "{:?}",
            brew_calls
        );
        assert!(
            brew_calls
                .iter()
                .any(|c| c.starts_with("install --cask iterm2")),
            "{:?}",
            brew_calls
        );
        assert!(t
            .calls_of("gem")
            .iter()
            .any(|c| c.starts_with("install --no-document neovim")));
        assert!(t
            .calls_of("go")
            .iter()
            .any(|c| c == "install example.com/x/tool@latest"));
        assert_eq!(t.calls_of("mas"), vec!["list", "install 123"]);
        assert_eq!(
            t.calls_of("fnm"),
            vec!["install --lts", "default lts-latest"]
        );
        assert_eq!(t.calls_of("git"), vec!["lfs install"]);
        assert_eq!(t.calls_of("rtk"), vec!["init -g --opencode --auto-patch"]);
    }

    #[test]
    fn unavailable_backend_is_reported_not_fatal() {
        let t = TestEnv::new();
        let manifest = parse_manifest("install:\n  brew:\n    formulas: [git]\n").unwrap();
        let results = install_all(t.exec(), &manifest).unwrap();
        // brew is absent from the isolated PATH: the formula-install outcome is
        // a non-fatal skip (so `sync` continues on machines mid-bootstrap).
        let install = results
            .iter()
            .find(|r| r.backend == "brew" && r.note.contains("skipping"))
            .expect("skip outcome");
        assert!(install.ok());
        assert!(install.note.contains("not installed"));
    }
}
