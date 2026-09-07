//! `dotfiles prefs` — declarative macOS preferences: validate / apply / diff / show.

use crate::ctx::Ctx;
use anyhow::Result;
use clap::Parser;
use dotfiles_exec::Event;
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
            ctx.env.report(Event::Section {
                title: "prefs".to_string(),
            });
            // Necessity scan (sudo keep-alive parity, but precise): cache
            // credentials once up front — but only when an elevated entry
            // would actually change. `defaults` entries are diffed; `exec`
            // entries are not diffable (conservative: always elevate);
            // `restart-apps` always runs `sudo killall`.
            if !ctx.env.dry_run {
                match entries_needing_sudo(&ctx.env, &file) {
                    Ok(needed) if needed.is_empty() => {
                        if file.prefs.iter().any(|e| match e {
                            dotfiles_prefs::PrefEntry::Defaults { sudo, .. } => *sudo,
                            dotfiles_prefs::PrefEntry::Exec { sudo, .. } => *sudo,
                            dotfiles_prefs::PrefEntry::Builtin { name, .. } => {
                                name == "restart-apps"
                            }
                        }) {
                            ctx.env.report(Event::Note {
                                msg: "sudo: every elevated preference already in sync — no elevation needed".to_string(),
                            });
                        }
                    }
                    Ok(needed) => {
                        ctx.env.report(Event::Note {
                            msg: format!(
                                "sudo: {} elevated entr{} will change ({}); caching credentials once",
                                needed.len(),
                                if needed.len() == 1 { "y" } else { "ies" },
                                needed.join(", "),
                            ),
                        });
                        ctx.env.elevate(
                            "sudo",
                            &["-v"],
                            "elevated preferences are out of sync — caching credentials once up front",
                        )?;
                    }
                    // Diff itself failed (e.g. `defaults` missing): fall back
                    // to the old unconditional warmup rather than prompting
                    // once per entry mid-run.
                    Err(_) => {
                        ctx.env.elevate(
                            "sudo",
                            &["-v"],
                            "could not determine preference drift — caching credentials once up front",
                        )?;
                    }
                }
            }
            let report = engine::apply(&ctx.env, &file)?;
            let mut applied = 0;
            let mut unchanged = 0;
            for (id, status) in &report.results {
                match status {
                    PrefStatus::Applied => {
                        applied += 1;
                        ctx.env.report(Event::Note {
                            msg: format!("+ {}", id),
                        });
                    }
                    PrefStatus::Unchanged => unchanged += 1,
                    PrefStatus::Failed(e) => ctx.env.report(Event::Warn {
                        msg: format!("✗ {}: {}", id, e),
                    }),
                }
            }
            ctx.env.report(Event::Note {
                msg: format!(
                    "prefs: {} applied, {} already set, {} failed",
                    applied,
                    unchanged,
                    report.failures().len()
                ),
            });
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
                        ctx.env.report(Event::Note {
                            msg: format!(
                                "{} {} (want: {}, have: {})",
                                mark,
                                e.id,
                                e.desired,
                                e.current.as_deref().unwrap_or("<unset>")
                            ),
                        });
                    } else if std::env::var_os("DOTFILES_VERBOSE").is_some() {
                        ctx.env.report(Event::Note {
                            msg: format!("{} {}", mark, e.id),
                        });
                    }
                }
                ctx.env.report(Event::Note {
                    msg: format!(
                        "diff: {} in sync, {} drifted",
                        entries.len() - drifted.len(),
                        drifted.len()
                    ),
                });
            }
            if !drifted.is_empty() {
                anyhow::bail!("{} pref(s) drifted", drifted.len());
            }
            Ok(())
        }
    }
}

/// Ids of entries whose application may elevate: out-of-sync `sudo: true`
/// `defaults` entries (via `diff`; `add`-mode entries always write),
/// every `sudo: true` exec entry (not diffable), and `restart-apps`.
fn entries_needing_sudo(
    env: &dotfiles_exec::ExecEnv,
    file: &dotfiles_prefs::PrefsFile,
) -> Result<Vec<String>> {
    use dotfiles_prefs::PrefEntry;
    let drifted: std::collections::BTreeSet<String> = engine::diff(env, file)?
        .into_iter()
        .filter(|e| e.status == engine::DiffStatus::Drifted)
        .map(|e| e.id)
        .collect();
    let mut needed = vec![];
    for e in &file.prefs {
        match e {
            PrefEntry::Defaults {
                sudo: true, add, ..
            } if *add || drifted.contains(e.id()) => needed.push(e.id().to_string()),
            PrefEntry::Exec { sudo: true, .. } => needed.push(e.id().to_string()),
            PrefEntry::Builtin { name, .. } if name == "restart-apps" => {
                needed.push(e.id().to_string())
            }
            _ => {}
        }
    }
    Ok(needed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotfiles_testkit::TestEnv;

    fn prefs(yaml: &str) -> dotfiles_prefs::PrefsFile {
        dotfiles_prefs::parse_prefs(yaml).unwrap()
    }

    #[test]
    fn in_sync_sudo_entries_need_nothing() {
        let t = TestEnv::new();
        t.stub("defaults", "echo 1; exit 0");
        let file = prefs(
            "prefs:\n  - { id: a, kind: defaults, domain: D, key: K, type: bool, value: true, sudo: true }\n",
        );
        assert!(entries_needing_sudo(t.exec(), &file).unwrap().is_empty());
    }

    #[test]
    fn drifted_sudo_defaults_exec_and_restart_apps_need_sudo() {
        let t = TestEnv::new();
        // `defaults read` echoes 1: `a` (want true) is in sync, `b` drifted.
        t.stub("defaults", "echo 1; exit 0");
        let file = prefs(
            "prefs:\n\
             \x20 - { id: a, kind: defaults, domain: D, key: K1, type: bool, value: true, sudo: true }\n\
             \x20 - { id: b, kind: defaults, domain: D, key: K2, type: bool, value: false, sudo: true }\n\
             \x20 - { id: c, kind: defaults, domain: D, key: K3, type: bool, value: false }\n\
             \x20 - { id: d, kind: exec, program: pmset, args: [], sudo: true }\n\
             \x20 - { id: e, kind: builtin, name: restart-apps }\n",
        );
        let needed = entries_needing_sudo(t.exec(), &file).unwrap();
        assert_eq!(needed, vec!["b", "d", "e"]);
    }

    #[test]
    fn add_mode_sudo_entries_always_need_sudo() {
        let t = TestEnv::new();
        t.stub("defaults", "echo 1; exit 0");
        let file = prefs(
            "prefs:\n  - { id: m, kind: defaults, domain: D, key: K, type: dict, value: {x: 1}, add: true, sudo: true }\n",
        );
        assert_eq!(entries_needing_sudo(t.exec(), &file).unwrap(), vec!["m"]);
    }
}
