//! `dotfiles software-update` — ports software-update.sh. Manual-only:
//! installs every macOS update and reboots. Never driven by the LaunchAgent.

use crate::ctx::Ctx;
use anyhow::Result;
use clap::Parser;
use std::io::Write;

#[derive(Parser, Debug)]
pub struct SoftwareUpdateArgs {
    /// Skip the interactive confirmation (still never called by the agent)
    #[arg(long)]
    pub yes: bool,
}

pub fn run(ctx: &Ctx, args: SoftwareUpdateArgs) -> Result<()> {
    if !args.yes {
        eprint!(
            "This installs ALL pending macOS updates and REBOOTS the machine. Continue? [y/N] "
        );
        std::io::stderr().flush()?;
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if line.trim().to_lowercase() != "y" {
            println!("aborted");
            return Ok(());
        }
    }
    // softwareupdate talks to the real machine even in sandbox mode; refuse to
    // run under a sandboxed HOME so tests can never trigger a reboot.
    if ctx.env.home.starts_with("/var/folders") || ctx.env.home.starts_with("/tmp") {
        anyhow::bail!("refusing `software-update` with a sandboxed HOME (safety guard)");
    }
    let res = ctx.env.output(
        "sudo",
        &[
            "softwareupdate",
            "--install",
            "--restart",
            "--all",
            "--agree-to-license",
            "--verbose",
        ],
    )?;
    if !res.ok() {
        anyhow::bail!("softwareupdate failed: {}", res.stderr.trim());
    }
    Ok(())
}
