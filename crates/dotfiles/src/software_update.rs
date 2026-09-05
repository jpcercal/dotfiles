//! `dotfiles software-update` — ports software-update.sh. Manual-only:
//! installs every macOS update and reboots. Never driven by the LaunchAgent.
//! `--list-only` is the read-only counterpart (safe for CI): it runs just
//! `softwareupdate --list`, which can neither install nor reboot.

use crate::ctx::Ctx;
use anyhow::Result;
use clap::Parser;
use std::io::Write;

#[derive(Parser, Debug)]
pub struct SoftwareUpdateArgs {
    /// Skip the interactive confirmation (still never called by the agent)
    #[arg(long)]
    pub yes: bool,
    /// List pending updates without installing anything (read-only, no sudo,
    /// no prompt). Safe for CI and the sandbox.
    #[arg(long, conflicts_with = "yes")]
    pub list_only: bool,
}

pub fn run(ctx: &Ctx, args: SoftwareUpdateArgs) -> Result<()> {
    if args.list_only {
        return list(ctx);
    }
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

/// Read-only listing: no sudo, no prompt, no sandbox guard (`--list` cannot
/// install or reboot, so hermetic tests can drive it through stubs).
fn list(ctx: &Ctx) -> Result<()> {
    let res = ctx.env.output("softwareupdate", &["--list"])?;
    print!("{}", res.stdout);
    if !res.ok() {
        anyhow::bail!("softwareupdate --list failed: {}", res.stderr.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotfiles_testkit::TestEnv;

    fn args(list_only: bool) -> SoftwareUpdateArgs {
        SoftwareUpdateArgs {
            yes: false,
            list_only,
        }
    }

    #[test]
    fn list_only_runs_softwareupdate_list_without_sudo() {
        let t = TestEnv::new();
        t.stub(
            "softwareupdate",
            "echo 'No new software available.'; exit 0",
        );
        // Sandboxed HOME: allowed here (read-only) — the install path below
        // still refuses it via its safety guard.
        let ctx = Ctx::sandbox(t.root(), false).unwrap();
        run(&ctx, args(true)).unwrap();
        assert_eq!(t.calls_of("softwareupdate"), vec!["--list"]);
        assert!(t.calls_of("sudo").is_empty());
    }

    #[test]
    fn list_only_reports_tool_failure() {
        let t = TestEnv::new();
        t.stub("softwareupdate", "echo 'flaky mirror' 1>&2; exit 1");
        let ctx = Ctx::sandbox(t.root(), false).unwrap();
        assert!(run(&ctx, args(true)).is_err());
    }
}
