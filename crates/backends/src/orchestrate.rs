//! Manifest-driven orchestration: the typed equivalent of the old
//! `install-apps.sh` + brew tap bootstrapping from `install-dependencies.sh`.

use crate::outcome::BackendOutcome;
use crate::{bootstrap, brew, graph, schedule, toolchain, Spec};
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

fn taps_from_manifest(m: &Manifest) -> Vec<String> {
    m.install
        .require
        .iter()
        .filter_map(|e| {
            let (p, n) = dotfiles_manifest::units::split_unit_id(e.id())?;
            if p == "brew-tap" {
                Some(n)
            } else {
                None
            }
        })
        .collect()
}

fn packages_for_backend(m: &Manifest, prefix: &str) -> Vec<String> {
    m.install
        .require
        .iter()
        .filter_map(|e| {
            let (p, n) = dotfiles_manifest::units::split_unit_id(e.id())?;
            if p == prefix {
                Some(n)
            } else {
                None
            }
        })
        .collect()
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

/// Legacy sequential install, kept for `--sequential`. New code should use the graph engine.
pub fn install_all_sequential(env: &ExecEnv, m: &Manifest) -> Result<Vec<BackendOutcome>> {
    let mut results: Vec<BackendOutcome> = vec![];

    results.push(ensure_taps(env, &taps_from_manifest(m))?);

    let brew = brew::Brew;
    let cask = brew::BrewCask;
    results.push(run_if_available(
        env,
        &brew,
        &packages_for_backend(m, "brew-formula"),
    ));
    results.push(run_if_available(
        env,
        &cask,
        &packages_for_backend(m, "brew-cask"),
    ));
    results.push(run_if_available(
        env,
        &crate::gem::Gem,
        &packages_for_backend(m, "gem"),
    ));
    results.push(run_if_available(
        env,
        &crate::npm::Npm,
        &packages_for_backend(m, "npm"),
    ));
    results.push(run_if_available(
        env,
        &crate::pip::UvPip,
        &packages_for_backend(m, "pip"),
    ));
    results.push(run_if_available(
        env,
        &crate::go::Go,
        &packages_for_backend(m, "go"),
    ));

    results.push(run_if_available(
        env,
        &crate::cargo::Cargo,
        &packages_for_backend(m, "cargo"),
    ));
    results.push(run_if_available(
        env,
        &crate::composer::Composer,
        &packages_for_backend(m, "composer"),
    ));

    let mas_ids: Vec<String> = packages_for_backend(m, "mas");
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
    for entry in &m.install.bootstrap {
        results.push(bootstrap::run(entry.id(), env)?);
    }

    Ok(results)
}

/// Cache `sudo` credentials once before the parallel run so concurrent cask
/// installs never race on an interactive password prompt (mirrors the prefs
/// `sudo -v` pre-flight). Best-effort: failures are ignored here and surface
/// per-unit like any other error.
fn preflight_sudo(env: &ExecEnv, m: &Manifest) -> Result<()> {
    let has_cask = m.install.require.iter().any(|e| {
        dotfiles_manifest::units::split_unit_id(e.id()).is_some_and(|(p, _)| p == "brew-cask")
    });
    if !has_cask || !env.has_command("sudo") {
        return Ok(());
    }
    let _ = env.output("sudo", &["-v"])?;
    Ok(())
}

fn run_hook(env: &ExecEnv, snippet: &str) -> Result<bool> {
    let res = env.output("sh", &["-c", snippet])?;
    Ok(res.ok())
}

/// Execute one graph unit. Backend errors become failed outcomes (the
/// scheduler blocks dependents); only spawn-level failures escape as `Err`.
fn run_unit(env: &ExecEnv, m: &Manifest, unit: &graph::Unit) -> BackendOutcome {
    // Pre-install hook (only for package units that have it)
    if let Some(hooks) = &unit.hooks {
        if let Some(snippet) = &hooks.pre_install {
            let hook_env = env.clone().with_env("DOTFILES_PKG_ID", &unit.id);
            match run_hook(&hook_env, snippet) {
                Ok(true) => {}
                Ok(false) => {
                    let mut out = BackendOutcome::empty(unit.backend);
                    out.fail_one(
                        unit.id.clone(),
                        format!(
                            "pre-install hook failed: {}",
                            snippet.lines().next().unwrap_or("")
                        ),
                    );
                    return out;
                }
                Err(e) => {
                    let mut out = BackendOutcome::empty(unit.backend);
                    out.fail_one(unit.id.clone(), format!("pre-install hook error: {}", e));
                    return out;
                }
            }
        }
    }

    let res: Result<BackendOutcome> = match &unit.kind {
        graph::UnitKind::Taps => ensure_taps(env, &unit.packages),
        graph::UnitKind::Batch("brew") | graph::UnitKind::Package("brew") => {
            // For brew, reconstruct packages with version suffix if needed? brew-formula pins not supported,
            // so version is None always.
            Ok(run_if_available_with_version(env, &brew::Brew, unit))
        }
        graph::UnitKind::Batch("cask") | graph::UnitKind::Package("cask") => {
            Ok(run_if_available_with_version(env, &brew::BrewCask, unit))
        }
        graph::UnitKind::Batch("gem") | graph::UnitKind::Package("gem") => {
            Ok(run_if_available_with_version(env, &crate::gem::Gem, unit))
        }
        graph::UnitKind::Batch("npm") | graph::UnitKind::Package("npm") => {
            Ok(run_if_available_with_version(env, &crate::npm::Npm, unit))
        }
        graph::UnitKind::Batch("pip") | graph::UnitKind::Package("pip") => {
            Ok(run_if_available_with_version(env, &crate::pip::UvPip, unit))
        }
        graph::UnitKind::Batch("go") | graph::UnitKind::Package("go") => {
            Ok(run_if_available_with_version(env, &crate::go::Go, unit))
        }
        graph::UnitKind::Batch("cargo") | graph::UnitKind::Package("cargo") => Ok(
            run_if_available_with_version(env, &crate::cargo::Cargo, unit),
        ),
        graph::UnitKind::Batch("composer") | graph::UnitKind::Package("composer") => Ok(
            run_if_available_with_version(env, &crate::composer::Composer, unit),
        ),
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
    let mut outcome = res.unwrap_or_else(|e| {
        let mut out = BackendOutcome::empty(unit.backend);
        out.fail_one(unit.id.clone(), e.to_string());
        out
    });

    // Post-install hook: fires whenever the unit ended up present (newly
    // installed or already installed) with no failures. Config hooks must
    // converge on every run — a re-run with the package already present must
    // still (re)apply its filesystem config.
    if let Some(hooks) = &unit.hooks {
        if let Some(snippet) = &hooks.post_install {
            let present = !outcome.changed.is_empty() || !outcome.unchanged.is_empty();
            if present && outcome.failed.is_empty() {
                let hook_env = env.clone().with_env("DOTFILES_PKG_ID", &unit.id);
                match run_hook(&hook_env, snippet) {
                    Ok(true) => {}
                    Ok(false) => {
                        outcome.fail_one(
                            unit.id.clone(),
                            format!(
                                "post-install hook failed: {}",
                                snippet.lines().next().unwrap_or("")
                            ),
                        );
                    }
                    Err(e) => {
                        outcome
                            .fail_one(unit.id.clone(), format!("post-install hook error: {}", e));
                    }
                }
            }
        }
    }

    outcome
}

fn packages_with_version(unit: &graph::Unit) -> Vec<String> {
    if let Some(ver) = &unit.version {
        // For token-style pins (npm, pip, go, composer) we embed version in package string
        match unit.backend {
            "npm" => unit
                .packages
                .iter()
                .map(|p| format!("{}@{}", p, ver))
                .collect(),
            "pip" => unit
                .packages
                .iter()
                .map(|p| format!("{}=={}", p, ver))
                .collect(),
            "go" => unit
                .packages
                .iter()
                .map(|p| format!("{}@{}", p, ver))
                .collect(),
            "composer" => unit
                .packages
                .iter()
                .map(|p| format!("{}:{}", p, ver))
                .collect(),
            _ => unit.packages.clone(),
        }
    } else if unit.backend == "go" {
        // Go always needs a version suffix; default to @latest when not pinned
        unit.packages
            .iter()
            .map(|p| format!("{}@latest", p))
            .collect()
    } else {
        unit.packages.clone()
    }
}

fn run_if_available_with_version(
    env: &ExecEnv,
    backend: &dyn crate::PackageBackend,
    unit: &graph::Unit,
) -> BackendOutcome {
    let pkgs = packages_with_version(unit);
    // For gem and cargo, version is handled via flags, not token
    if unit.backend == "gem" && unit.version.is_some() {
        return run_gem_with_version(env, backend, unit);
    }
    if unit.backend == "cargo" && unit.version.is_some() {
        return run_cargo_with_version(env, backend, unit);
    }
    run_if_available(env, backend, &pkgs)
}

fn run_gem_with_version(
    env: &ExecEnv,
    backend: &dyn crate::PackageBackend,
    unit: &graph::Unit,
) -> BackendOutcome {
    // Gem version pins must be installed with `gem install name -v version`
    // For single-package units, handle specially
    if unit.packages.len() == 1 {
        if let Some(ver) = &unit.version {
            let pkg = &unit.packages[0];
            // Check if already installed via gem list
            if !backend.is_available(env) {
                let mut out = BackendOutcome::unavailable(backend.name());
                out.note = format!("{} not installed — skipping 1 package(s)", backend.tool());
                return out;
            }
            let _installed = backend.list_installed(env).unwrap_or_default();
            // Note: version-aware check would require parsing gem list versions; keep idempotent via name only.
            let res = env.output("gem", &["install", "--no-document", pkg, "-v", ver]);
            let mut out = BackendOutcome::empty(backend.name());
            match res {
                Ok(r) if r.ok() => out.changed.push(pkg.clone()),
                Ok(r) => out.fail_one(
                    pkg.clone(),
                    crate::util::summarize_error(&r.stderr, &r.stdout),
                ),
                Err(e) => out.fail_one(pkg.clone(), e.to_string()),
            }
            return out;
        }
    }
    run_if_available(env, backend, &unit.packages)
}

fn run_cargo_with_version(
    env: &ExecEnv,
    backend: &dyn crate::PackageBackend,
    unit: &graph::Unit,
) -> BackendOutcome {
    if unit.packages.len() == 1 {
        if let Some(ver) = &unit.version {
            let pkg = &unit.packages[0];
            if !backend.is_available(env) {
                let mut out = BackendOutcome::unavailable(backend.name());
                out.note = format!("{} not installed — skipping 1 package(s)", backend.tool());
                return out;
            }
            let res = env.output("cargo", &["install", pkg, "--version", ver]);
            let mut out = BackendOutcome::empty(backend.name());
            match res {
                Ok(r) if r.ok() => out.changed.push(pkg.clone()),
                Ok(r) => out.fail_one(
                    pkg.clone(),
                    crate::util::summarize_error(&r.stderr, &r.stdout),
                ),
                Err(e) => out.fail_one(pkg.clone(), e.to_string()),
            }
            return out;
        }
    }
    run_if_available(env, backend, &unit.packages)
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
  require:
    - "brew-tap:hashicorp/tap"
    - "brew-formula:git"
    - "brew-cask:iterm2"
    - "gem:neovim"
    - "npm:prettier"
    - "pip:pynvim"
    - "go:example.com/x/tool@latest"
    - id: "mas:123"
      label: "Foo"
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
        let manifest =
            parse_manifest("install:\n  require:\n    - \"brew-formula:git\"\n").unwrap();
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
  require:
    - "brew-tap:hashicorp/tap"
    - "brew-formula:git"
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
        let manifest = parse_manifest(
            "install:\n  require:\n    - \"brew-formula:git\"\n    - \"brew-cask:iterm2\"\n",
        )
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

    #[test]
    fn hooks_fire_via_sh() {
        let t = TestEnv::new();
        t.stub("brew", BREW_STUB);
        t.stub("sh", "exit 0");
        let manifest = parse_manifest(
            r#"
install:
  require:
    - id: "brew-formula:git"
      hooks:
        pre-install: "echo pre"
        post-install: "echo post"
"#,
        )
        .unwrap();
        let results = install_all(t.exec(), &manifest).unwrap();
        assert!(results.iter().all(|r| r.ok()));
        let sh_calls = t.calls_of("sh");
        assert!(
            sh_calls.iter().any(|c| c.contains("echo pre")),
            "{:?}",
            sh_calls
        );
        assert!(
            sh_calls.iter().any(|c| c.contains("echo post")),
            "{:?}",
            sh_calls
        );
    }

    #[test]
    fn every_manifest_hook_executes_when_changed_and_when_present() {
        // Every hook declared in the real manifests must actually execute:
        // on a fresh install (package changed) AND on a re-run where the
        // package is already installed (outcome.unchanged — the convergence
        // case that broke the e2e-machine CI job).
        use dotfiles_manifest::{BootstrapEntry, Install, Manifest, RequireEntry};

        /// One hook snippet declared in a real manifest, with its entry for
        /// building a minimal single-entry install manifest.
        struct HookCase {
            file: &'static str,
            require: Option<RequireEntry>,
            bootstrap: Option<BootstrapEntry>,
            id: String,
            snippet: String,
        }

        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut cases: Vec<HookCase> = vec![];
        let mut non_install_hooks: Vec<String> = vec![];
        for file in ["apps.yaml", "e2e/apps.ci.yaml"] {
            let text = std::fs::read_to_string(root.join(file)).unwrap();
            let m = parse_manifest(&text).unwrap();
            for e in &m.install.require {
                if let Some(h) = e.hooks() {
                    for (kind, snippet) in [
                        ("pre-install", &h.pre_install),
                        ("post-install", &h.post_install),
                    ] {
                        if let Some(s) = snippet {
                            cases.push(HookCase {
                                file,
                                require: Some(e.clone()),
                                bootstrap: None,
                                id: e.id().into(),
                                snippet: s.clone(),
                            });
                            let _ = kind;
                        }
                    }
                    for (kind, opt) in [
                        ("pre-update", &h.pre_update),
                        ("post-update", &h.post_update),
                        ("pre-uninstall", &h.pre_uninstall),
                        ("post-uninstall", &h.post_uninstall),
                    ] {
                        if opt.is_some() {
                            non_install_hooks.push(format!("{file} {} {kind}", e.id()));
                        }
                    }
                }
            }
            for b in &m.install.bootstrap {
                if let Some(h) = b.hooks() {
                    for (kind, snippet) in [
                        ("pre-install", &h.pre_install),
                        ("post-install", &h.post_install),
                    ] {
                        if let Some(s) = snippet {
                            cases.push(HookCase {
                                file,
                                require: None,
                                bootstrap: Some(b.clone()),
                                id: format!("bootstrap:{}", b.id()),
                                snippet: s.clone(),
                            });
                            let _ = kind;
                        }
                    }
                    for (kind, opt) in [
                        ("pre-update", &h.pre_update),
                        ("post-update", &h.post_update),
                        ("pre-uninstall", &h.pre_uninstall),
                        ("post-uninstall", &h.post_uninstall),
                    ] {
                        if opt.is_some() {
                            non_install_hooks.push(format!("{file} bootstrap:{} {kind}", b.id()));
                        }
                    }
                }
            }
        }
        // The install engine only executes install-phase hooks; any other
        // hook kind in the manifests would silently never run.
        assert!(
            non_install_hooks.is_empty(),
            "hooks the install engine never executes: {:?}",
            non_install_hooks
        );
        assert!(!cases.is_empty(), "no hooks found in manifests");

        for case in &cases {
            for present in [false, true] {
                let t = TestEnv::new();
                t.stub("sh", "exit 0");
                t.stub("sudo", "exit 0"); // preflight_sudo; TestEnv PATH sees real /usr/bin/sudo
                if case.bootstrap.is_some() {
                    if present {
                        t.stub_ok("opencode", "1.0");
                    } else {
                        t.stub("curl", "exit 0");
                    }
                } else if let Some((prefix, name)) =
                    dotfiles_manifest::units::split_unit_id(&case.id)
                        .map(|(p, n)| (p.to_string(), n))
                {
                    match prefix.as_str() {
                        "brew-formula" | "brew-cask" => {
                            let flag = if prefix == "brew-formula" {
                                "--formula"
                            } else {
                                "--cask"
                            };
                            let list = if present {
                                format!("echo '{name}'")
                            } else {
                                "printf ''".to_string()
                            };
                            t.stub(
                                "brew",
                                &format!(
                                    "case \"$*\" in \"list -1 {flag}\") {list} ;; esac\nexit 0"
                                ),
                            );
                        }
                        "mas" => {
                            let list = if present {
                                format!("echo '{name} Label (1.0)'")
                            } else {
                                "printf ''".to_string()
                            };
                            t.stub(
                                "mas",
                                &format!("case \"$*\" in \"list\") {list} ;; esac\nexit 0"),
                            );
                        }
                        other => panic!("hook test: no stubs for backend '{other}' ({})", case.id),
                    }
                }
                let m = Manifest {
                    schema_version: 2,
                    install: Install {
                        require: case.require.clone().into_iter().collect(),
                        bootstrap: case.bootstrap.clone().into_iter().collect(),
                        ..Default::default()
                    },
                };
                let results = install_all(t.exec(), &m).unwrap();
                assert!(
                    results.iter().all(|r| r.ok()),
                    "{} {} (present={present}): install failed: {:?}",
                    case.file,
                    case.id,
                    results
                        .iter()
                        .flat_map(|r| r.failed.clone())
                        .collect::<Vec<_>>()
                );
                // Raw log: multi-line snippets span lines, so match the file,
                // not the line-split helper.
                let log = std::fs::read_to_string(t.root().join("calls.log")).unwrap();
                assert!(
                    log.contains(case.snippet.as_str()),
                    "{} {} (present={present}): hook never executed.\nlog:\n{log}",
                    case.file,
                    case.id
                );
            }
        }
    }

    #[test]
    fn pre_hook_failure_blocks_install() {
        let t = TestEnv::new();
        t.stub("brew", BREW_STUB);
        t.stub("sh", "exit 1");
        let manifest = parse_manifest(
            r#"
install:
  require:
    - id: "brew-formula:git"
      hooks:
        pre-install: "false"
"#,
        )
        .unwrap();
        let results = install_all(t.exec(), &manifest).unwrap();
        assert!(results.iter().any(|r| !r.ok()));
        assert!(t.calls_of("brew").iter().all(|c| !c.starts_with("install")));
    }
}
