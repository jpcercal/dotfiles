use crate::outcome::BackendOutcome;
use crate::toolchain;
use anyhow::{Context, Result};
use dotfiles_exec::ExecEnv;

/// Bootstrap builtins — the typed replacements for the old
/// `install.brew.customCommands` one-liners. Each is idempotent.
pub const BOOTSTRAP_STEPS: &[(&str, &str)] = &[
    (
        "fzf-keybindings",
        "install fzf shell keybindings (auto-answer y)",
    ),
    ("git-lfs", "git lfs install"),
    ("python-links", "symlink uv python/pip into ~/.local/bin"),
    ("nvim-plug", "download vim-plug into the nvim autoload dir"),
    ("opencode", "install or upgrade opencode via its installer"),
    ("rtk-patch", "rtk init -g --opencode --auto-patch"),
    (
        "claude-mem",
        "npx claude-mem install --ide opencode --provider openrouter",
    ),
];

pub fn known_bootstrap_names() -> Vec<&'static str> {
    BOOTSTRAP_STEPS.iter().map(|(n, _)| *n).collect()
}

pub fn run(name: &str, env: &ExecEnv) -> Result<BackendOutcome> {
    match name {
        "fzf-keybindings" => fzf_keybindings(env),
        "git-lfs" => git_lfs(env),
        "python-links" => toolchain::python_links(env),
        "nvim-plug" => nvim_plug(env),
        "opencode" => opencode(env),
        "rtk-patch" => rtk_patch(env),
        "claude-mem" => claude_mem(env),
        other => anyhow::bail!(
            "unknown bootstrap step '{}' (known: {})",
            other,
            known_bootstrap_names().join(", ")
        ),
    }
}

fn outcome(backend: &'static str) -> BackendOutcome {
    BackendOutcome {
        backend,
        ..Default::default()
    }
}

/// `$(brew --prefix)/opt/fzf/install`, pre-answering "y" — replaces the old
/// `echo "y" | .../install` pipeline. Runs the brew-shipped installer directly
/// (no shell pipeline), feeding answers via stdin.
fn fzf_keybindings(env: &ExecEnv) -> Result<BackendOutcome> {
    let mut out = outcome("bootstrap:fzf-keybindings");
    if env.home.join(".fzf.zsh").exists() {
        out.unchanged.push("~/.fzf.zsh".into());
        return Ok(out);
    }
    let prefix = env.output("brew", &["--prefix"]).context("brew --prefix")?;
    let installer = format!("{}/opt/fzf/install", prefix.stdout.trim());
    if !std::path::Path::new(&installer).exists() {
        // fzf isn't installed (or this is a sandbox) — nothing to configure.
        out.unchanged.push("fzf not installed".into());
        return Ok(out);
    }
    let res = env.output_stdin(&installer, &["--no-update-rc"], "y\ny\ny\n")?;
    if res.ok() {
        out.changed.push("fzf keybindings".into());
    } else {
        anyhow::bail!("fzf install failed: {}", res.stderr.trim());
    }
    Ok(out)
}

fn git_lfs(env: &ExecEnv) -> Result<BackendOutcome> {
    let mut out = outcome("bootstrap:git-lfs");
    if !env.has_command("git-lfs") && !env.has_command("git-lfs") && !env.has_command("git") {
        out.unchanged.push("git-lfs not installed".into());
        return Ok(out);
    }
    let res = env.output("git", &["lfs", "install"])?;
    if res.ok() {
        out.changed.push("git lfs install".into());
    } else {
        anyhow::bail!("git lfs install failed: {}", res.stderr.trim());
    }
    Ok(out)
}

/// Download vim-plug (`curl -fLo ... --create-dirs`).
fn nvim_plug(env: &ExecEnv) -> Result<BackendOutcome> {
    let mut out = outcome("bootstrap:nvim-plug");
    let dest = env.home.join(".local/share/nvim/site/autoload/plug.vim");
    if dest.exists() {
        out.unchanged.push(dest.display().to_string());
        return Ok(out);
    }
    std::fs::create_dir_all(dest.parent().unwrap())?;
    let res = env.output(
        "curl",
        &[
            "-fLo",
            dest.to_str().unwrap(),
            "https://raw.githubusercontent.com/junegunn/vim-plug/master/plug.vim",
        ],
    )?;
    if res.ok() {
        out.changed.push(dest.display().to_string());
    } else {
        anyhow::bail!("vim-plug download failed: {}", res.stderr.trim());
    }
    Ok(out)
}

/// opencode installer: the vendor ships a shell installer — downloaded to a
/// temp file and executed; no shell script is kept in this repo.
fn opencode(env: &ExecEnv) -> Result<BackendOutcome> {
    let mut out = outcome("bootstrap:opencode");
    if env.has_command("opencode") {
        out.unchanged.push("opencode".into());
        return Ok(out);
    }
    run_remote_installer(env, "https://opencode.ai/install")?;
    out.changed.push("opencode".into());
    Ok(out)
}

fn rtk_patch(env: &ExecEnv) -> Result<BackendOutcome> {
    let mut out = outcome("bootstrap:rtk-patch");
    if !env.has_command("rtk") {
        out.unchanged.push("rtk not installed".into());
        return Ok(out);
    }
    let res = env.output("rtk", &["init", "-g", "--opencode", "--auto-patch"])?;
    if res.ok() {
        out.changed.push("rtk opencode patch".into());
    } else {
        anyhow::bail!("rtk init failed: {}", res.stderr.trim());
    }
    Ok(out)
}

fn claude_mem(env: &ExecEnv) -> Result<BackendOutcome> {
    let mut out = outcome("bootstrap:claude-mem");
    if !env.has_command("npx") {
        out.unchanged.push("npx not available".into());
        return Ok(out);
    }
    let res = env.output(
        "npx",
        &[
            "claude-mem",
            "install",
            "--ide",
            "opencode",
            "--provider",
            "openrouter",
        ],
    )?;
    if res.ok() {
        out.changed.push("claude-mem".into());
    } else {
        anyhow::bail!("claude-mem install failed: {}", res.stderr.trim());
    }
    Ok(out)
}

fn run_remote_installer(env: &ExecEnv, url: &str) -> Result<()> {
    let tmp = std::env::temp_dir().join(format!("dotfiles-installer-{}.sh", std::process::id()));
    let dl = env.output("curl", &["-fsSL", "-o", tmp.to_str().unwrap(), url])?;
    if !dl.ok() {
        anyhow::bail!("installer download failed ({url}): {}", dl.stderr.trim());
    }
    let tool = tmp.to_string_lossy().to_string();
    let run = env.output("sh", &[tool.as_str()]);
    let _ = std::fs::remove_file(&tmp);
    let run = run?;
    if !run.ok() {
        anyhow::bail!("installer failed ({url}): {}", run.stderr.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotfiles_testkit::TestEnv;

    #[test]
    fn unknown_step_is_error_listing_known() {
        let t = TestEnv::new();
        let err = run("nope", t.exec()).unwrap_err();
        assert!(err.to_string().contains("git-lfs"), "{}", err);
    }

    #[test]
    fn nvim_plug_downloads_once() {
        let t = TestEnv::new();
        // fake curl "creates" the file
        t.stub("curl", "printf x > \"$2\"; exit 0");
        let out = run("nvim-plug", t.exec()).unwrap();
        assert_eq!(out.changed.len(), 1);
        assert!(t
            .home()
            .join(".local/share/nvim/site/autoload/plug.vim")
            .exists());
        // second run: no curl call
        let before = t.calls_of("curl").len();
        let out2 = run("nvim-plug", t.exec()).unwrap();
        assert_eq!(out2.unchanged.len(), 1);
        assert_eq!(t.calls_of("curl").len(), before);
    }

    #[test]
    fn opencode_skipped_when_present() {
        let t = TestEnv::new();
        t.stub_ok("opencode", "1.0");
        let out = run("opencode", t.exec()).unwrap();
        assert_eq!(out.unchanged, vec!["opencode"]);
        assert!(t.calls_of("curl").is_empty());
    }

    #[test]
    fn fzf_keybindings_skips_when_fzf_not_installed() {
        let t = TestEnv::new();
        // brew --prefix resolves somewhere with no fzf installer beneath it
        t.stub_ok("brew", "/nonexistent/prefix\n");
        let out = run("fzf-keybindings", t.exec()).unwrap();
        assert_eq!(out.unchanged, vec!["fzf not installed"]);
        assert!(out.changed.is_empty());
    }

    #[test]
    fn fzf_keybindings_runs_installer_and_answers_prompts() {
        let t = TestEnv::new();
        let prefix = t.root().join("homebrew");
        let installer = prefix.join("opt/fzf/install");
        std::fs::create_dir_all(installer.parent().unwrap()).unwrap();
        // fake installer records the stdin it receives next to itself
        std::fs::write(&installer, "#!/bin/sh\ncat > \"$0.stdin\"\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&installer, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        t.stub("brew", &format!("echo '{}'; exit 0", prefix.display()));
        let out = run("fzf-keybindings", t.exec()).unwrap();
        assert_eq!(out.changed, vec!["fzf keybindings"]);
        let recorded = installer.parent().unwrap().join("install.stdin");
        let stdin = std::fs::read_to_string(&recorded).unwrap();
        assert!(
            stdin.lines().count() >= 3,
            "installer prompts answered: {:?}",
            stdin
        );
    }

    #[test]
    fn git_lfs_invocation() {
        let t = TestEnv::new();
        t.stub_ok("git", "");
        run("git-lfs", t.exec()).unwrap();
        assert_eq!(t.calls_of("git"), vec!["lfs install"]);
    }

    #[test]
    fn rtk_patch_invocation_and_skip_logic() {
        let t = TestEnv::new();
        // no rtk on path → skip
        let out = run("rtk-patch", t.exec()).unwrap();
        assert_eq!(out.unchanged, vec!["rtk not installed"]);

        t.stub_ok("rtk", "");
        let out = run("rtk-patch", t.exec()).unwrap();
        assert_eq!(out.changed, vec!["rtk opencode patch"]);
        assert_eq!(t.calls_of("rtk"), vec!["init -g --opencode --auto-patch"]);
    }
}
