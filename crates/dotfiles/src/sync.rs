//! `dotfiles sync` — the full pipeline (replacement for `make` / run.sh).
//! Jobs: bootstrap → install → apply → prefs → history (+ software-update, opt-in).

use crate::ctx::Ctx;
use anyhow::Result;
use clap::Parser;

pub const JOBS: &[&str] = &["bootstrap", "install", "apply", "prefs", "history"];
pub const OPT_IN_JOBS: &[&str] = &["software-update"];

#[derive(Parser, Debug)]
pub struct SyncArgs {
    /// Skip jobs (comma-separated, e.g. --skip prefs,history)
    #[arg(long, value_delimiter = ',', num_args = 0..)]
    pub skip: Vec<String>,

    /// Run only these jobs (enables opt-in jobs like software-update)
    #[arg(long, value_delimiter = ',', num_args = 0..)]
    pub only: Vec<String>,

    /// Sandbox mode: HOME/PATH under a temp dir filled with harmless stub
    /// tools — full end-to-end run, zero real-machine effects. Prints the
    /// sandbox root (kept for inspection).
    #[arg(long)]
    pub sandbox: bool,

    /// Max parallel install units for the install job (default: manifest
    /// `install.execution.max_jobs`, 0 = number of CPUs).
    #[arg(long)]
    pub jobs: Option<usize>,

    /// Run the install job with the legacy sequential installer.
    #[arg(long)]
    pub sequential: bool,
}

/// Job selection (pure, unit-tested).
pub fn select_jobs(skip: &[String], only: &[String]) -> Result<Vec<&'static str>> {
    if !only.is_empty() {
        let mut jobs = vec![];
        for j in only {
            if let Some(known) = JOBS.iter().chain(OPT_IN_JOBS.iter()).find(|k| *k == j) {
                jobs.push(*known);
            } else {
                anyhow::bail!(
                    "unknown job '{}' (known: {}, opt-in: {})",
                    j,
                    JOBS.join(", "),
                    OPT_IN_JOBS.join(", ")
                );
            }
        }
        return Ok(jobs);
    }
    let mut jobs = vec![];
    for j in JOBS {
        if skip.iter().any(|s| s == j) {
            continue;
        }
        jobs.push(*j);
    }
    for s in skip {
        if !JOBS.contains(&s.as_str()) {
            anyhow::bail!(
                "unknown job in --skip: '{}' (known: {})",
                s,
                JOBS.join(", ")
            );
        }
    }
    Ok(jobs)
}

pub fn run(ctx: &Ctx, args: SyncArgs) -> Result<()> {
    let jobs = select_jobs(&args.skip, &args.only)?;

    let effective: Ctx;
    let ctx = if args.sandbox {
        let root =
            std::env::temp_dir().join(format!("dotfiles-sync-sandbox-{}", std::process::id()));
        std::fs::create_dir_all(&root)?;
        let mut sbx = Ctx::sandbox(&root, ctx.env.dry_run)?;
        let bin = root.join("bin");
        let calls = root.join("calls.log");
        dotfiles_exec::stubs::install_standard_stubs(&bin, &calls, &root)?;
        // Isolate from real tools for hermeticity
        sbx.env = sbx
            .env
            .clone()
            .with_isolated_base_paths(&["/usr/bin", "/bin"]);
        // Bring the real manifests so the E2E run exercises production data
        for f in ["apps.yaml", "commands.yaml", "prefs.yaml"] {
            let src = ctx.dotfiles_dir.join(f);
            let dst = sbx.dotfiles_dir.join(f);
            std::fs::create_dir_all(&sbx.dotfiles_dir)?;
            if src.exists() {
                std::fs::copy(&src, &dst)?;
            }
        }
        println!("sandbox root: {}", root.display());
        effective = sbx;
        &effective
    } else {
        ctx
    };

    println!("sync: jobs = {}", jobs.join(", "));
    let mut failed: Vec<&str> = vec![];
    for job in jobs {
        println!("▶ {}", job);
        let result: Result<()> = match job {
            "bootstrap" => {
                crate::bootstrap::run(ctx, crate::bootstrap::BootstrapArgs { no_update: true })
            }
            "install" => crate::pkg::install(
                ctx,
                crate::pkg::InstallArgs {
                    specs: vec![],
                    jobs: args.jobs,
                    sequential: args.sequential,
                },
            ),
            "apply" => crate::apply::run(
                ctx,
                crate::apply::ApplyArgs {
                    only: None,
                    check: false,
                },
            ),
            "prefs" => crate::prefs_cmd::run(
                ctx,
                crate::prefs_cmd::PrefsArgs {
                    command: Some(crate::prefs_cmd::PrefsCommand::Apply),
                },
            ),
            "history" => crate::history::run(
                ctx,
                crate::history::HistoryArgs {
                    command: crate::history::HistoryCommand::Seed,
                },
            ),
            "software-update" => crate::software_update::run(
                ctx,
                crate::software_update::SoftwareUpdateArgs {
                    yes: false,
                    list_only: false,
                },
            ),
            other => anyhow::bail!("unknown job {}", other),
        };
        if let Err(e) = result {
            eprintln!("✗ {} failed: {}", job, e);
            failed.push(job);
        }
    }
    if !failed.is_empty() {
        anyhow::bail!("sync: job(s) failed: {}", failed.join(", "));
    }
    println!("sync: done");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn default_selection_is_all_jobs_in_order() {
        assert_eq!(select_jobs(&[], &[]).unwrap(), JOBS);
    }

    #[test]
    fn skip_excludes_named_jobs() {
        let jobs = select_jobs(&sv(&["prefs", "history"]), &[]).unwrap();
        assert_eq!(jobs, vec!["bootstrap", "install", "apply"]);
    }

    #[test]
    fn only_runs_named_jobs_and_allows_opt_in() {
        let jobs = select_jobs(&[], &sv(&["apply", "software-update"])).unwrap();
        assert_eq!(jobs, vec!["apply", "software-update"]);
    }

    #[test]
    fn unknown_rejected() {
        assert!(select_jobs(&sv(&["nope"]), &[]).is_err());
        assert!(select_jobs(&[], &sv(&["nope"])).is_err());
        // opt-in jobs must not appear in the default selection
        assert!(!select_jobs(&[], &[]).unwrap().contains(&"software-update"));
    }
}
