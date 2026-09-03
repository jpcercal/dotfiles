//! `dotfiles prefs` — declarative macOS preferences: validate / apply / diff / show.

use crate::ctx::Ctx;
use anyhow::Result;
use clap::Parser;
use dotfiles_prefs::{engine, PrefStatus};

#[derive(Parser, Debug)]
pub struct PrefsArgs {
    #[command(subcommand)]
    pub command: Option<PrefsCommand>,
}

#[derive(Parser, Debug)]
pub enum PrefsCommand {
    /// Apply all preferences (idempotent)
    Apply,
    /// Show where the machine differs from prefs.yaml (exit 1 on drift)
    Diff {
        /// Machine-readable output
        #[arg(long)]
        json: bool,
    },
    /// Validate prefs.yaml (schema + whitelist + duplicates)
    Validate,
    /// Print the parsed, resolved preference list
    Show {
        /// Machine-readable output
        #[arg(long)]
        json: bool,
    },
}

pub fn run(ctx: &Ctx, args: PrefsArgs) -> Result<()> {
    let path = ctx.prefs_path();
    let file = dotfiles_prefs::load_prefs(&path)?;
    match args.command.unwrap_or(PrefsCommand::Apply) {
        PrefsCommand::Validate => {
            println!("prefs.yaml valid: {} entries", file.prefs.len());
            Ok(())
        }
        PrefsCommand::Show { json } => {
            if json {
                println!("{}", serde_json::to_string_pretty(&file)?);
            } else {
                for e in &file.prefs {
                    println!("{}", e.id());
                }
            }
            Ok(())
        }
        PrefsCommand::Apply => {
            // sudo keep-alive parity (support-require-sudo.sh + support-keep-alive.sh):
            // cache credentials once up front when any entry needs sudo.
            let any_sudo = file.prefs.iter().any(|e| match e {
                dotfiles_prefs::PrefEntry::Defaults { sudo, .. } => *sudo,
                dotfiles_prefs::PrefEntry::Exec { sudo, .. } => *sudo,
                dotfiles_prefs::PrefEntry::Builtin { name, .. } => name == "restart-apps",
            });
            if any_sudo && !ctx.env.dry_run {
                ctx.env.output("sudo", &["-v"])?;
            }
            let report = engine::apply(&ctx.env, &file)?;
            let mut applied = 0;
            let mut unchanged = 0;
            for (id, status) in &report.results {
                match status {
                    PrefStatus::Applied => {
                        applied += 1;
                        println!("+ {}", id);
                    }
                    PrefStatus::Unchanged => unchanged += 1,
                    PrefStatus::Failed(e) => eprintln!("✗ {}: {}", id, e),
                }
            }
            println!(
                "prefs: {} applied, {} already set, {} failed",
                applied,
                unchanged,
                report.failures().len()
            );
            // Parity with apply-preferences.sh (no `set -e`): individual pref
            // failures are reported but never abort the run; `prefs diff` is
            // the drift gate.
            Ok(())
        }
        PrefsCommand::Diff { json } => {
            let entries = engine::diff(&ctx.env, &file)?;
            let drifted: Vec<_> = entries
                .iter()
                .filter(|e| e.status == engine::DiffStatus::Drifted)
                .collect();
            if json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else {
                for e in &entries {
                    let mark = match e.status {
                        engine::DiffStatus::InSync => "=",
                        engine::DiffStatus::Drifted => "≠",
                        engine::DiffStatus::Unreadable => "?",
                    };
                    if e.status == engine::DiffStatus::Drifted {
                        println!(
                            "{} {} (want: {}, have: {})",
                            mark,
                            e.id,
                            e.desired,
                            e.current.as_deref().unwrap_or("<unset>")
                        );
                    } else if std::env::var_os("DOTFILES_VERBOSE").is_some() {
                        println!("{} {}", mark, e.id);
                    }
                }
                println!(
                    "diff: {} in sync, {} drifted",
                    entries.len() - drifted.len(),
                    drifted.len()
                );
            }
            if !drifted.is_empty() {
                anyhow::bail!("{} pref(s) drifted", drifted.len());
            }
            Ok(())
        }
    }
}
