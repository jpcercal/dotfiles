//! apt/brew-style package verbs: install / remove / search / list / info / update.

use crate::ctx::Ctx;
use anyhow::Result;
use clap::Parser;
use dotfiles_backends::{orchestrate, BackendOutcome, Spec};

#[derive(Parser, Debug)]
pub struct InstallArgs {
    /// Packages as `backend:name` (bare name = brew formula). No args = install
    /// everything declared in the manifest.
    pub specs: Vec<String>,
    /// Max parallel install units (default: manifest `install.execution.max_jobs`,
    /// 0 = number of CPUs).
    #[arg(long)]
    pub jobs: Option<usize>,
    /// Use the legacy sequential installer instead of the parallel
    /// dependency-graph engine.
    #[arg(long)]
    pub sequential: bool,
}

#[derive(Parser, Debug)]
pub struct RemoveArgs {
    /// Packages as `backend:name` (bare name = brew formula).
    #[arg(required = true)]
    pub specs: Vec<String>,
}

#[derive(Parser, Debug)]
pub struct SearchArgs {
    pub query: String,
    /// Restrict to one backend (default: all).
    #[arg(long)]
    pub backend: Option<String>,
}

#[derive(Parser, Debug)]
pub struct ListArgs {
    /// List installed packages per backend.
    #[arg(long, conflicts_with = "outdated")]
    pub installed: bool,
    /// List outdated packages per backend.
    #[arg(long)]
    pub outdated: bool,
    /// Restrict to one backend.
    #[arg(long)]
    pub backend: Option<String>,
}

#[derive(Parser, Debug)]
pub struct InfoArgs {
    /// Package as `backend:name` (bare name = brew formula).
    pub spec: String,
}

#[derive(Parser, Debug)]
pub struct UpdateArgs {
    /// Restrict to one backend (default: all).
    #[arg(long)]
    pub backend: Option<String>,
}

pub fn install(ctx: &Ctx, args: InstallArgs) -> Result<()> {
    let results = if args.specs.is_empty() {
        let m = ctx.manifest()?;
        println!(
            "installing manifest ({} formulas, {} casks, {} mas, {} bootstrap steps)",
            m.install.brew.formulas.len(),
            m.install.brew.casks.len(),
            m.install.mas.apps.len(),
            m.install.bootstrap.len()
        );
        if args.sequential {
            orchestrate::install_all_sequential(&ctx.env, &m)?
        } else {
            let mut opts = orchestrate::sched_opts_from_manifest(&m);
            if let Some(jobs) = args.jobs {
                opts.max_jobs = jobs;
            }
            orchestrate::install_all_with_opts(&ctx.env, &m, &opts)?
        }
    } else {
        let specs: Vec<Spec> = args
            .specs
            .iter()
            .map(|s| Spec::parse(s))
            .collect::<Result<_, _>>()?;
        orchestrate::install_specs(&ctx.env, &specs)?
    };
    print_outcomes(&results);
    // Never fail silently: a failed unit (or a unit skipped because its
    // dependency failed) fails the command, so CI and `sync` go red. Units
    // skipped for missing tools stay non-fatal by design (mid-bootstrap
    // machines): those outcomes carry a note, not failures.
    let failed: Vec<String> = results
        .iter()
        .flat_map(|r| r.failed.iter().map(|f| format!("{}:{}", r.backend, f.name)))
        .collect();
    if !failed.is_empty() {
        anyhow::bail!("install failed: {}", failed.join(", "));
    }
    Ok(())
}

pub fn remove(ctx: &Ctx, args: RemoveArgs) -> Result<()> {
    let mut grouped: Vec<(String, Vec<String>)> = vec![];
    for s in &args.specs {
        let spec = Spec::parse(s)?;
        match grouped.iter_mut().find(|(b, _)| b == &spec.backend) {
            Some((_, v)) => v.push(spec.name),
            None => grouped.push((spec.backend, vec![spec.name])),
        }
    }
    for (backend, pkgs) in grouped {
        let b = dotfiles_backends::by_name(&backend).expect("Spec::parse validates backend");
        let out = b.remove(&ctx.env, &pkgs)?;
        print_outcome(&out);
        if !out.ok() {
            anyhow::bail!("remove failed");
        }
    }
    Ok(())
}

pub fn search(ctx: &Ctx, args: SearchArgs) -> Result<()> {
    for b in selected_backends(args.backend.as_deref())? {
        if !b.is_available(&ctx.env) {
            continue;
        }
        let hits = b.search(&ctx.env, &args.query)?;
        if !hits.is_empty() {
            println!("{}:", b.name());
            for h in hits.iter().take(20) {
                println!("  {}", h);
            }
        }
    }
    Ok(())
}

pub fn list(ctx: &Ctx, args: ListArgs) -> Result<()> {
    for b in selected_backends(args.backend.as_deref())? {
        if !b.is_available(&ctx.env) {
            continue;
        }
        let items = if args.outdated {
            b.outdated(&ctx.env)?
        } else {
            b.list_installed(&ctx.env)?
        };
        println!("{} ({}):", b.name(), items.len());
        for i in items {
            println!("  {}", i);
        }
    }
    Ok(())
}

pub fn info(ctx: &Ctx, args: InfoArgs) -> Result<()> {
    let spec = Spec::parse(&args.spec)?;
    let b = dotfiles_backends::by_name(&spec.backend).expect("Spec::parse validates backend");
    println!("{}", b.info(&ctx.env, &spec.name)?);
    Ok(())
}

pub fn update(ctx: &Ctx, args: UpdateArgs) -> Result<()> {
    for b in selected_backends(args.backend.as_deref())? {
        if !b.is_available(&ctx.env) {
            continue;
        }
        let out = b.update_index(&ctx.env)?;
        print_outcome(&out);
    }
    Ok(())
}

fn selected_backends(
    name: Option<&str>,
) -> Result<Vec<Box<dyn dotfiles_backends::PackageBackend>>> {
    match name {
        Some(n) => Ok(vec![dotfiles_backends::by_name(n).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown backend '{}' (known: {})",
                n,
                dotfiles_backends::known_backend_names().join(", ")
            )
        })?]),
        None => Ok(dotfiles_backends::all_backends()),
    }
}

pub fn print_outcomes(results: &[BackendOutcome]) {
    for r in results {
        print_outcome(r);
    }
}

pub fn print_outcome(r: &BackendOutcome) {
    if r.changed.is_empty() && r.unchanged.is_empty() && r.failed.is_empty() && r.note.is_empty() {
        return;
    }
    let status = if !r.failed.is_empty() {
        "failed"
    } else if r.changed.is_empty() {
        "ok"
    } else {
        "changed"
    };
    println!(
        "{:10} [{}] {} changed, {} already ok, {} failed{}{}",
        r.backend,
        status,
        r.changed.len(),
        r.unchanged.len(),
        r.failed.len(),
        if r.note.is_empty() { "" } else { " — " },
        r.note
    );
    for f in &r.failed {
        println!("  ✗ {}", f);
    }
    if std::env::var_os("DOTFILES_VERBOSE").is_some() {
        for c in &r.changed {
            println!("  + {}", c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotfiles_testkit::TestEnv;

    fn ctx_with_manifest(t: &TestEnv, yaml: &str) -> Ctx {
        let dir = t.dotfiles_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("apps.yaml"), yaml).unwrap();
        let mut ctx = Ctx::sandbox(t.root(), false).unwrap();
        // Isolate from real tools (mirrors verify/smoke tests).
        ctx.env = ctx.env.clone().with_isolated_base_paths(&[]);
        ctx
    }

    fn install_all(ctx: &Ctx) -> anyhow::Result<()> {
        install(
            ctx,
            InstallArgs {
                specs: vec![],
                jobs: Some(1),
                sequential: false,
            },
        )
    }

    #[test]
    fn install_fails_when_a_unit_fails() {
        let t = TestEnv::new();
        t.stub(
            "brew",
            "case \"$1\" in list) echo '' ;; install) echo boom 1>&2; exit 1 ;; esac; exit 0",
        );
        let ctx = ctx_with_manifest(&t, "install:\n  brew:\n    formulas: [git]\n");
        let err = install_all(&ctx).unwrap_err();
        assert!(err.to_string().contains("install failed"), "{err}");
        assert!(err.to_string().contains("git"), "{err}");
    }

    #[test]
    fn install_succeeds_when_units_ok_or_skipped_for_missing_tools() {
        let t = TestEnv::new();
        t.stub("brew", "case \"$1\" in list) echo '' ;; esac; exit 0");
        // gem has no stub: unavailable tools stay non-fatal by design
        // (mid-bootstrap machines), only real failures fail the command.
        let ctx = ctx_with_manifest(
            &t,
            "install:\n  brew:\n    formulas: [git]\n  gem:\n    rubygems: [neovim]\n",
        );
        install_all(&ctx).unwrap();
    }
}
