//! `dotfiles bootstrap` — ports install-dependencies.sh: installs Homebrew
//! itself (when missing), refreshes it (with retries), then taps.

use crate::ctx::Ctx;
use anyhow::Result;
use clap::Parser;
use dotfiles_backends::orchestrate::ensure_taps;

const BREW_INSTALL_URL: &str = "https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh";
const BREW_UPDATE_RETRIES: usize = 6;

#[derive(Parser, Debug)]
pub struct BootstrapArgs {
    /// Skip the brew update retry loop
    #[arg(long)]
    pub no_update: bool,
}

pub fn run(ctx: &Ctx, args: BootstrapArgs) -> Result<()> {
    ensure_homebrew(ctx)?;
    if !args.no_update {
        brew_update_with_retries(ctx)?;
    }
    let m = ctx.manifest()?;
    let outcome = ensure_taps(&ctx.env, &m.install.brew.taps)?;
    crate::pkg::print_outcome(&outcome);
    if !outcome.ok() {
        anyhow::bail!("tap setup failed");
    }
    Ok(())
}

fn ensure_homebrew(ctx: &Ctx) -> Result<()> {
    if ctx.env.has_command("brew") {
        println!("homebrew: already installed ({})", brew_prefix(ctx)?);
        return Ok(());
    }
    println!("homebrew: installing (NONINTERACTIVE)…");
    let tmp = std::env::temp_dir().join("dotfiles-brew-install.sh");
    let dl = ctx.env.output(
        "curl",
        &["-fsSL", "-o", tmp.to_str().unwrap(), BREW_INSTALL_URL],
    )?;
    if !dl.ok() {
        anyhow::bail!("brew installer download failed: {}", dl.stderr.trim());
    }
    // The vendor installer is interactive; Homebrew honors NONINTERACTIVE=1.
    // (Previously: `echo "y" | bash <(curl …)`.)
    let out = ctx
        .env
        .clone()
        .with_env("NONINTERACTIVE", "1")
        .output_stdin("bash", &[tmp.to_str().unwrap()], "y\n")?;
    let _ = std::fs::remove_file(&tmp);
    if !out.ok() {
        anyhow::bail!("brew installer failed: {}", out.stderr.trim());
    }
    // brew shellenv equivalent: /opt/homebrew/bin is already path-injected by main.
    if !ctx.env.has_command("brew") && !ctx.env.dry_run {
        anyhow::bail!("brew installer finished but `brew` is still not on PATH");
    }
    Ok(())
}

fn brew_prefix(ctx: &Ctx) -> Result<String> {
    Ok(ctx
        .env
        .output("brew", &["--prefix"])?
        .stdout
        .trim()
        .to_string())
}

/// `brew update` with the old script's 6-attempt tolerance (flaky mirrors).
fn brew_update_with_retries(ctx: &Ctx) -> Result<()> {
    for attempt in 1..=BREW_UPDATE_RETRIES {
        let out = ctx.env.output("brew", &["update"])?;
        if out.ok() {
            return Ok(());
        }
        eprintln!(
            "brew update attempt {}/{} failed: {}",
            attempt,
            BREW_UPDATE_RETRIES,
            out.stderr.trim()
        );
        if attempt < BREW_UPDATE_RETRIES {
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }
    anyhow::bail!("brew update failed after {} attempts", BREW_UPDATE_RETRIES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotfiles_testkit::TestEnv;

    #[test]
    fn skips_install_when_brew_present() {
        let t = TestEnv::new();
        t.stub_ok("brew", "/opt/homebrew\n");
        let ctx = Ctx::sandbox(t.root(), false).unwrap();
        ensure_homebrew(&ctx).unwrap();
        assert!(t.calls_of("curl").is_empty());
    }

    #[test]
    fn brew_update_retries_until_success() {
        let t = TestEnv::new();
        // fails twice, succeeds on third
        t.stub(
            "brew",
            "n=$(cat \"$DOTFILES_BREW_N\" 2>/dev/null || echo 0); n=$((n+1)); echo $n > \"$DOTFILES_BREW_N\"; \
             if [ \"$1\" = update ] && [ $n -lt 3 ]; then exit 1; fi; exit 0",
        );
        let ctx = Ctx::sandbox(t.root(), false).unwrap();
        std::env::set_var("DOTFILES_BREW_N", t.root().join("n"));
        brew_update_with_retries(&ctx).unwrap();
        let updates = t.calls_of("brew").iter().filter(|c| *c == "update").count();
        assert_eq!(updates, 3);
        std::env::remove_var("DOTFILES_BREW_N");
    }
}
