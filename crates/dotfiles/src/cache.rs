//! `dotfiles cache clean` — port of bin/clean-osx-cache: clears user and
//! system caches with a confirmation prompt (the old script's print API is
//! long dead; this one actually works).

use crate::ctx::Ctx;
use anyhow::Result;
use clap::Parser;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct CacheArgs {
    #[command(subcommand)]
    pub command: CacheCommand,
}

#[derive(Parser, Debug)]
pub enum CacheCommand {
    /// Delete ~/Library/Caches/* and /Library/Caches/*
    Clean {
        /// Skip the interactive confirmation
        #[arg(long)]
        yes: bool,
    },
}

pub fn run(ctx: &Ctx, args: CacheArgs) -> Result<()> {
    match args.command {
        CacheCommand::Clean { yes } => clean(ctx, yes),
    }
}

fn clean(ctx: &Ctx, yes: bool) -> Result<()> {
    let user_caches = ctx.env.home.join("Library/Caches");
    let system_caches = PathBuf::from("/Library/Caches");
    println!("This deletes the contents of:");
    println!("  {} (user)", user_caches.display());
    println!("  {} (system, needs sudo)", system_caches.display());
    if !yes {
        eprint!("Continue? [y/N] ");
        std::io::stderr().flush()?;
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if line.trim().to_lowercase() != "y" {
            println!("aborted");
            return Ok(());
        }
    }
    for (dir, sudo) in [(user_caches, false), (system_caches, true)] {
        if !dir.is_dir() {
            continue;
        }
        if let Ok(du) = ctx.env.output("du", &["-sh", dir.to_str().unwrap()]) {
            println!(
                "  {}: {}",
                dir.display(),
                du.stdout.split_whitespace().next().unwrap_or("?")
            );
        }
        if sudo {
            // Trailing slash: delete contents, keep the dir (script parity).
            let ok = ctx
                .env
                .output("sudo", &["rm", "-rf", &format!("{}/", dir.display())])?;
            if !ok.ok() {
                anyhow::bail!("failed to clean {}: {}", dir.display(), ok.stderr.trim());
            }
        } else {
            for entry in std::fs::read_dir(&dir)?.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let _ = std::fs::remove_dir_all(&p);
                } else {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }
    Ok(())
}
