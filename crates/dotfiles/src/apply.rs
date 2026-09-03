//! `dotfiles apply` — ports configure-apps.sh: login shell, config dirs,
//! symlinks (with .bkp backups), nvim plugins, Dock layout.

use crate::ctx::Ctx;
use anyhow::{Context, Result};
use clap::Parser;
use dotfiles_manifest::Manifest;
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub struct ApplyArgs {
    /// Only apply one area (default: all)
    #[arg(long, value_parser = ["shell", "dirs", "links", "nvim", "dock"])]
    pub only: Option<String>,

    /// Report drift without changing anything (links/dirs only)
    #[arg(long)]
    pub check: bool,
}

pub fn run(ctx: &Ctx, args: ApplyArgs) -> Result<()> {
    let m = ctx.manifest()?;
    let only = args.only.as_deref();
    let want = |area: &str| only.map(|o| o == area).unwrap_or(true);

    if want("shell") {
        set_login_shell(ctx, args.check)?;
    }
    if want("dirs") {
        create_dirs(ctx, &m, args.check)?;
    }
    if want("links") {
        create_links(ctx, &m, args.check)?;
    }
    if want("nvim") && !args.check {
        install_nvim_plugins(ctx)?;
    }
    if want("dock") && !args.check {
        apply_dock(ctx, &m)?;
    }
    Ok(())
}

/// `sudo dscl . -create /Users/$USER UserShell "$(brew --prefix)/bin/zsh"`,
/// but only when the current shell differs (idempotent).
fn set_login_shell(ctx: &Ctx, check: bool) -> Result<()> {
    let user = std::env::var("USER").context("USER not set")?;
    let prefix = ctx
        .env
        .output("brew", &["--prefix"])?
        .stdout
        .trim()
        .to_string();
    if prefix.is_empty() {
        anyhow::bail!("brew --prefix returned nothing — is Homebrew installed?");
    }
    let zsh = format!("{}/bin/zsh", prefix);
    let current = ctx.env.output(
        "dscl",
        &[".", "-read", &format!("/Users/{}", user), "UserShell"],
    )?;
    let current_shell = current
        .stdout
        .split_whitespace()
        .last()
        .unwrap_or("")
        .to_string();
    println!("login shell: current={}, desired={}", current_shell, zsh);
    if current_shell == zsh {
        println!("  already set");
        return Ok(());
    }
    if check {
        println!("  DRIFT: login shell would be changed");
        return Ok(());
    }
    let res = ctx.env.output(
        "sudo",
        &[
            "dscl",
            ".",
            "-create",
            &format!("/Users/{}", user),
            "UserShell",
            &zsh,
        ],
    )?;
    if !res.ok() {
        anyhow::bail!("dscl failed: {}", res.stderr.trim());
    }
    Ok(())
}

fn create_dirs(ctx: &Ctx, m: &Manifest, check: bool) -> Result<()> {
    for dir in &m.config.mkdir {
        let path = ctx.env.expand(dir);
        if path.is_dir() {
            continue;
        }
        if check {
            println!("DRIFT dir: {}", path.display());
            continue;
        }
        println!("mkdir: {}", path.display());
        std::fs::create_dir_all(&path).with_context(|| format!("mkdir {}", path.display()))?;
    }
    Ok(())
}

fn create_links(ctx: &Ctx, m: &Manifest, check: bool) -> Result<()> {
    for link in &m.config.symbolic_links {
        let src = ctx.dotfiles_dir.join(&link.from.relative_path);
        let dst = ctx.env.expand(&link.to.absolute_path);

        // Already correct?
        if std::fs::read_link(&dst).map(|t| t == src).unwrap_or(false) {
            continue;
        }
        if check {
            println!("DRIFT link: {} -> {}", dst.display(), src.display());
            continue;
        }
        // Back up an existing *real file* before replacing (as the script did:
        // `<dst>.YYYY.MM.DD.bkp`). Symlinks to elsewhere are simply replaced.
        if dst.exists() && !dst.is_symlink() {
            let stamp = chrono::Local::now().format("%Y.%m.%d");
            let backup = PathBuf::from(format!("{}.{}.bkp", dst.display(), stamp));
            println!("backup: {} -> {}", dst.display(), backup.display());
            std::fs::rename(&dst, &backup).with_context(|| format!("backup {}", dst.display()))?;
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(&dst);
        println!("link: {} -> {}", dst.display(), src.display());
        make_symlink(&src, &dst).with_context(|| format!("symlink {}", dst.display()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn make_symlink(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

fn install_nvim_plugins(ctx: &Ctx) -> Result<()> {
    if !ctx.env.has_command("nvim") {
        println!("nvim not installed — skipping PlugInstall");
        return Ok(());
    }
    println!("nvim: PlugInstall");
    let res = ctx
        .env
        .output("nvim", &["--headless", "+PlugInstall", "+qa"])?;
    if !res.ok() {
        anyhow::bail!("nvim PlugInstall failed: {}", res.stderr.trim());
    }
    Ok(())
}

fn apply_dock(ctx: &Ctx, m: &Manifest) -> Result<()> {
    let dock = &m.config.dockutil;
    if dock.add.is_empty() && !dock.before.reset && !dock.before.remove_all {
        return Ok(());
    }
    if !ctx.env.has_command("dockutil") {
        anyhow::bail!("dockutil not installed (formula: dockutil)");
    }
    if dock.before.reset {
        println!("dock: reset (defaults delete + restart)");
        let _ = ctx.env.output("defaults", &["delete", "com.apple.dock"])?;
        let _ = ctx.env.output("killall", &["Dock"])?;
    }
    if dock.before.remove_all {
        println!("dock: remove all");
        let res = ctx.env.output("dockutil", &["--remove", "all"])?;
        if !res.ok() {
            anyhow::bail!("dockutil --remove all failed: {}", res.stderr.trim());
        }
    }
    for entry in &dock.add {
        let mut args = vec!["--add", entry.app.as_str()];
        let after_owned;
        if let Some(after) = &entry.after {
            after_owned = after.clone();
            args.extend(["--after", after_owned.as_str()]);
        }
        let res = ctx.env.output("dockutil", &args)?;
        if !res.ok() {
            anyhow::bail!("dockutil --add {} failed: {}", entry.app, res.stderr.trim());
        }
        println!("dock: + {}", entry.app);
    }
    let _ = ctx.env.output("killall", &["Dock"])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotfiles_manifest::parse_manifest;
    use dotfiles_testkit::TestEnv;

    fn manifest_with_links() -> Manifest {
        parse_manifest(
            r#"
config:
  mkdir: ["~/.config/nvim/"]
  symbolic_links:
    - from: { relative_path: ".gitconfig" }
      to: { absolute_path: "~/.gitconfig" }
"#,
        )
        .unwrap()
    }

    #[test]
    fn dirs_and_links_are_created_and_idempotent() {
        let t = TestEnv::new();
        t.write("dotfiles/.gitconfig", "[user]\nname = j\n");
        let ctx = Ctx::sandbox(t.root(), false).unwrap();
        let m = manifest_with_links();
        create_dirs(&ctx, &m, false).unwrap();
        create_links(&ctx, &m, false).unwrap();
        assert!(t.home().join(".config/nvim").is_dir());
        let link = t.home().join(".gitconfig");
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            t.root().join("dotfiles/.gitconfig")
        );
        // second run: no error, link unchanged, no .bkp created
        create_links(&ctx, &m, false).unwrap();
        assert!(!t.home().join(".gitconfig.bkp").exists());
    }

    #[test]
    fn existing_file_is_backed_up_before_linking() {
        let t = TestEnv::new();
        t.write("dotfiles/.gitconfig", "[user]\n");
        t.write("home/.gitconfig", "pre-existing file\n");
        let ctx = Ctx::sandbox(t.root(), false).unwrap();
        let m = manifest_with_links();
        create_links(&ctx, &m, false).unwrap();
        let backups: Vec<_> = std::fs::read_dir(t.home())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name().to_string_lossy().starts_with(".gitconfig.")
                    && e.file_name().to_string_lossy().ends_with(".bkp")
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(std::fs::read_to_string(backups[0].path())
            .unwrap()
            .contains("pre-existing"));
        assert!(t.home().join(".gitconfig").is_symlink());
    }

    #[test]
    fn check_reports_drift_without_changing() {
        let t = TestEnv::new();
        t.write("dotfiles/.gitconfig", "[user]\n");
        let ctx = Ctx::sandbox(t.root(), false).unwrap();
        let m = manifest_with_links();
        create_links(&ctx, &m, true).unwrap();
        assert!(!t.home().join(".gitconfig").exists());
    }

    #[test]
    fn dock_sequence_matches_script() {
        let t = TestEnv::new();
        t.stub_ok("dockutil", "");
        t.stub_ok("defaults", "");
        t.stub_ok("killall", "");
        let ctx = Ctx::sandbox(t.root(), false).unwrap();
        let m = parse_manifest(
            r#"
config:
  dockutil:
    _before: { reset: true, removeAll: true }
    add:
      - app: "/Applications/Firefox.app"
        after: "Finder"
"#,
        )
        .unwrap();
        apply_dock(&ctx, &m).unwrap();
        assert_eq!(
            t.calls(),
            vec![
                "defaults delete com.apple.dock",
                "killall Dock",
                "dockutil --remove all",
                "dockutil --add /Applications/Firefox.app --after Finder",
                "killall Dock",
            ]
        );
    }
}
