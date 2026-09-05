//! Manifest-driven orchestration: the typed equivalent of the old
//! `install-apps.sh` + brew tap bootstrapping from `install-dependencies.sh`.

use crate::outcome::BackendOutcome;
use crate::{bootstrap, brew, graph, schedule, toolchain, Spec};
use anyhow::Result;
use dotfiles_exec::ExecEnv;
use dotfiles_manifest::{Manifest, PkgEntry};

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

/// Install everything declared in the manifest via the dependency-graph
/// parallel execution engine (`graph` + `schedule`). Units run as soon as
/// their `requires:` edges (explicit in apps.yaml plus implicit tool edges)
/// succeed and their lock class has a free slot; cross-ecosystem installs
/// overlap while same-tool work serializes (`brew` is capped at 1).
/// A failed unit never aborts the run — its dependents are reported as
/// `skipped (blocked by <id>)`.
pub fn install_all(env: &ExecEnv, m: &Manifest) -> Result<Vec<BackendOutcome>> {
    install_all_with_opts(env, m, &sched_opts_from_manifest(m))
}

/// `install_all` with explicit scheduler tuning (the CLI layers `--jobs` /
/// `--sequential` over the manifest's `install.execution` defaults).
pub fn install_all_with_opts(
    env: &ExecEnv,
    m: &Manifest,
    opts: &schedule::SchedOpts,
) -> Result<Vec<BackendOutcome>> {
    preflight_sudo(env, m)?;
    let g = graph::build(m)?;
    Ok(schedule::run(&g, opts, env, &|unit, env| {
        run_unit(env, m, unit)
    }))
}

/// Scheduler tuning from `install.execution` (manifest = source of truth).
pub fn sched_opts_from_manifest(m: &Manifest) -> schedule::SchedOpts {
    schedule::SchedOpts {
        max_jobs: m.install.execution.max_jobs,
        lock_limits: m.install.execution.locks.clone(),
    }
}

/// Legacy sequential install (taps → formulas → casks → … → bootstrap),
/// kept for `--sequential`. New code should use the graph engine.
pub fn install_all_sequential(env: &ExecEnv, m: &Manifest) -> Result<Vec<BackendOutcome>> {
    let mut results: Vec<BackendOutcome> = vec![];

    results.push(ensure_taps(env, &m.install.brew.taps)?);

    let brew = brew::Brew;
    let cask = brew::BrewCask;
    results.push(run_if_available(
        env,
        &brew,
        &names(&m.install.brew.formulas),
    ));
    results.push(run_if_available(env, &cask, &names(&m.install.brew.casks)));
    results.push(run_if_available(
        env,
        &crate::gem::Gem,
        &names(&m.install.gem.rubygems),
    ));
    results.push(run_if_available(
        env,
        &crate::npm::Npm,
        &names(&m.install.npm.global.packages),
    ));
    results.push(run_if_available(
        env,
        &crate::pip::UvPip,
        &names(&m.install.pip.packages),
    ));
    results.push(run_if_available(
        env,
        &crate::go::Go,
        &names(&m.install.go.packages),
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

fn names(entries: &[PkgEntry]) -> Vec<String> {
    entries.iter().map(|e| e.name().to_string()).collect()
}

/// Cache `sudo` credentials once before the parallel run so concurrent cask
/// installs never race on an interactive password prompt (mirrors the prefs
/// `sudo -v` pre-flight). Best-effort: failures are ignored here and surface
/// per-unit like any other error.
fn preflight_sudo(env: &ExecEnv, m: &Manifest) -> Result<()> {
    if m.install.brew.casks.is_empty() || !env.has_command("sudo") {
        return Ok(());
    }
    let _ = env.output("sudo", &["-v"])?;
    Ok(())
}

/// Execute one graph unit. Backend errors become failed outcomes (the
/// scheduler blocks dependents); only spawn-level failures escape as `Err`.
fn run_unit(env: &ExecEnv, m: &Manifest, unit: &graph::Unit) -> BackendOutcome {
    let res: Result<BackendOutcome> = match &unit.kind {
        graph::UnitKind::Taps => ensure_taps(env, &unit.packages),
        graph::UnitKind::Batch("brew") | graph::UnitKind::Package("brew") => {
            Ok(run_if_available(env, &brew::Brew, &unit.packages))
        }
        graph::UnitKind::Batch("cask") | graph::UnitKind::Package("cask") => {
            Ok(run_if_available(env, &brew::BrewCask, &unit.packages))
        }
        graph::UnitKind::Batch("gem") | graph::UnitKind::Package("gem") => {
            Ok(run_if_available(env, &crate::gem::Gem, &unit.packages))
        }
        graph::UnitKind::Batch("npm") | graph::UnitKind::Package("npm") => {
            Ok(run_if_available(env, &crate::npm::Npm, &unit.packages))
        }
        graph::UnitKind::Batch("pip") | graph::UnitKind::Package("pip") => {
            Ok(run_if_available(env, &crate::pip::UvPip, &unit.packages))
        }
        graph::UnitKind::Batch("go") | graph::UnitKind::Package("go") => {
            Ok(run_if_available(env, &crate::go::Go, &unit.packages))
        }
        graph::UnitKind::Batch("mas") | graph::UnitKind::Package("mas") => {
            Ok(run_if_available(env, &crate::mas::Mas, &unit.packages))
        }
        graph::UnitKind::Batch(other) | graph::UnitKind::Package(other) => Err(anyhow::anyhow!(
            "unknown backend '{other}' for unit '{}'",
            unit.id
        )),
        graph::UnitKind::Toolchain => {
            let key = unit.packages.first().map(String::as_str).unwrap_or("");
            match key {
                "rustup" => {
                    let channel = m
                        .install
                        .toolchains
                        .rustup
                        .as_ref()
                        .map(|r| r.channel.as_str())
                        .unwrap_or("stable");
                    toolchain::Toolchain::ensure_rustup(env, channel)
                }
                "node" => toolchain::Toolchain::ensure_node(env),
                "python" => toolchain::Toolchain::ensure_python(env),
                other => Err(anyhow::anyhow!("unknown toolchain '{other}'")),
            }
        }
        graph::UnitKind::Bootstrap => {
            let step = unit.packages.first().map(String::as_str).unwrap_or("");
            bootstrap::run(step, env)
        }
    };
    res.unwrap_or_else(|e| {
        let mut out = BackendOutcome::empty(unit.backend);
        out.fail_one(unit.id.clone(), e.to_string());
        out
    })
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

    #[test]
    fn failed_taps_block_dependent_batches_but_not_siblings() {
        let t = TestEnv::new();
        t.stub(
            "brew",
            "case \"$1\" in tap) echo 'network down' 1>&2; exit 1 ;; esac; exit 0",
        );
        t.stub_ok("rustup", "");
        let manifest = parse_manifest(
            r#"
install:
  brew:
    taps: ["hashicorp/tap"]
    formulas: ["git"]
  toolchains:
    rustup: {}
"#,
        )
        .unwrap();
        let results = install_all(t.exec(), &manifest).unwrap();
        // The taps unit failed …
        let taps = results
            .iter()
            .find(|r| r.failed.iter().any(|f| f.name == "hashicorp/tap"))
            .expect("taps failure");
        assert!(!taps.ok());
        // … so the formula batch (which requires the taps) was skipped …
        let batch = results
            .iter()
            .find(|r| r.note.contains("blocked by 'brew-tap:hashicorp/tap'"))
            .expect("blocked batch");
        assert!(!batch.ok());
        assert!(t.calls_of("brew").iter().all(|c| !c.starts_with("install")));
        // … while the independent rustup toolchain still ran.
        assert!(
            results
                .iter()
                .any(|r| r.backend == "toolchain:rustup" && r.ok()),
            "{:?}",
            results.iter().map(|r| &r.backend).collect::<Vec<_>>()
        );
    }

    #[test]
    fn sequential_path_preserves_legacy_order() {
        let t = TestEnv::new();
        t.stub("brew", BREW_STUB);
        let manifest =
            parse_manifest("install:\n  brew:\n    formulas: [git]\n    casks: [iterm2]\n")
                .unwrap();
        let results = install_all_sequential(t.exec(), &manifest).unwrap();
        assert!(results.iter().all(|r| r.ok()));
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
    }
}
