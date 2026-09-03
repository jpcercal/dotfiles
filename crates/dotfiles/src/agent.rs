//! `dotfiles agent` — LaunchAgent management (ports the two Makefile targets:
//! install_dotfiles_agent + bootout/enabled orchestration).

use crate::ctx::Ctx;
use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

pub const LABEL: &str = "com.jpcercal.dotfiles.updater";

#[derive(Parser, Debug)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub command: AgentCommand,
}

#[derive(Parser, Debug)]
pub enum AgentCommand {
    /// Install/refresh the LaunchAgent (copies the binary to ~/.local/bin)
    Install,
    /// Remove the LaunchAgent
    Uninstall,
    /// Show agent status
    Status,
    /// One gated tick (what the LaunchAgent runs)
    Tick,
}

pub fn run(ctx: &Ctx, args: AgentArgs) -> Result<()> {
    match args.command {
        AgentCommand::Install => install(ctx),
        AgentCommand::Uninstall => uninstall(ctx),
        AgentCommand::Status => status(ctx),
        AgentCommand::Tick => crate::upgrade::run(crate::upgrade::UpgradeArgs {
            gate: true,
            foreground: false,
            dry_run: false,
            headless: false,
        }),
    }
}

fn uid_domain() -> String {
    let uid = unsafe { libc::getuid() };
    format!("gui/{}", uid)
}

fn plist_dst(ctx: &Ctx) -> PathBuf {
    ctx.env
        .home
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", LABEL))
}

fn render_plist(template: &str, home: &std::path::Path) -> String {
    template.replace("__HOME__", &home.to_string_lossy())
}

fn install(ctx: &Ctx) -> Result<()> {
    let domain = uid_domain();
    // Best-effort unload of any previous agent (script parity: `-launchctl bootout …`)
    let _ = ctx
        .env
        .output("launchctl", &["bootout", &format!("{}/{}", domain, LABEL)]);

    // Ensure ~/.local/bin/dotfiles is fresh (Makefile also staged the binary)
    let local_bin = ctx.env.home.join(".local/bin");
    std::fs::create_dir_all(&local_bin)?;
    let target = local_bin.join("dotfiles");
    let exe = std::env::current_exe()?;
    if exe != target {
        let fresh = match (std::fs::metadata(&exe), std::fs::metadata(&target)) {
            (Ok(a), Ok(b)) => a.modified().ok() != b.modified().ok(),
            (Ok(_), Err(_)) => true,
            _ => false,
        };
        if fresh || !target.exists() {
            println!("staging {} -> {}", exe.display(), target.display());
            std::fs::copy(&exe, &target).context("copy binary to ~/.local/bin")?;
        }
    }

    // Render the plist from the repo template (the `sed __HOME__` step)
    let template_path = ctx
        .dotfiles_dir
        .join("launchd")
        .join(format!("{}.rust.plist", LABEL));
    let template = std::fs::read_to_string(&template_path)
        .with_context(|| format!("read {}", template_path.display()))?;
    let rendered = render_plist(&template, &ctx.env.home);
    let dst = plist_dst(ctx);
    std::fs::create_dir_all(dst.parent().unwrap())?;
    if std::fs::read_to_string(&dst).ok().as_deref() == Some(rendered.as_str()) {
        println!("plist unchanged: {}", dst.display());
    } else {
        std::fs::write(&dst, &rendered)?;
        println!("wrote {}", dst.display());
    }

    let boot = ctx
        .env
        .output("launchctl", &["bootstrap", &domain, dst.to_str().unwrap()])?;
    if !boot.ok() {
        anyhow::bail!("launchctl bootstrap failed: {}", boot.stderr.trim());
    }
    let enable = ctx
        .env
        .output("launchctl", &["enable", &format!("{}/{}", domain, LABEL)])?;
    if !enable.ok() {
        anyhow::bail!("launchctl enable failed: {}", enable.stderr.trim());
    }
    println!("agent installed: runs `dotfiles upgrade --gate` every 6h and at login");
    Ok(())
}

fn uninstall(ctx: &Ctx) -> Result<()> {
    let domain = uid_domain();
    let _ = ctx
        .env
        .output("launchctl", &["bootout", &format!("{}/{}", domain, LABEL)]);
    let dst = plist_dst(ctx);
    if dst.exists() {
        std::fs::remove_file(&dst)?;
        println!("removed {}", dst.display());
    }
    println!("agent uninstalled");
    Ok(())
}

fn status(ctx: &Ctx) -> Result<()> {
    let dst = plist_dst(ctx);
    println!(
        "plist: {}",
        if dst.exists() {
            dst.display().to_string()
        } else {
            "not installed".into()
        }
    );
    let out = ctx.env.output(
        "launchctl",
        &["print", &format!("{}/{}", uid_domain(), LABEL)],
    )?;
    print!("{}", out.stdout);
    if !out.ok() {
        println!("(agent not loaded)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_template_substitution() {
        let rendered = render_plist(
            "__HOME__/.local/bin/dotfiles\n__HOME__/dotfiles/logs",
            std::path::Path::new("/Users/x"),
        );
        assert_eq!(
            rendered,
            "/Users/x/.local/bin/dotfiles\n/Users/x/dotfiles/logs"
        );
    }

    #[test]
    fn install_writes_plist_and_bootstraps() {
        let t = dotfiles_testkit::TestEnv::new();
        t.write(
            "dotfiles/launchd/com.jpcercal.dotfiles.updater.rust.plist",
            "<string>__HOME__/.local/bin/dotfiles</string>",
        );
        t.write("dotfiles/apps.yaml", "install: {}\n");
        t.stub_ok("launchctl", "");
        let ctx = Ctx::sandbox(t.root(), false).unwrap();
        install(&ctx).unwrap();
        let plist = std::fs::read_to_string(plist_dst(&ctx)).unwrap();
        assert!(plist.contains(
            &ctx.env
                .home
                .join(".local/bin/dotfiles")
                .to_string_lossy()
                .to_string()
        ));
        let calls = t.calls_of("launchctl");
        assert!(
            calls.iter().any(|c| c.starts_with("bootstrap gui/")),
            "{:?}",
            calls
        );
        assert!(
            calls.iter().any(|c| c.starts_with("enable gui/")),
            "{:?}",
            calls
        );
    }
}
