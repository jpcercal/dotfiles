use crate::outcome::BackendOutcome;
use anyhow::Result;
use dotfiles_exec::ExecEnv;

/// Bootstrap builtins — the typed replacements for the old
/// `install.brew.customCommands` one-liners. Each is idempotent.
/// Only steps without an owning package live here; everything else moved
/// to post-install hooks on its package (fzf, git-lfs, uv, neovim, rtk, fnm).
pub const BOOTSTRAP_STEPS: &[(&str, &str)] =
    &[("opencode", "install or upgrade opencode via its installer")];

pub fn known_bootstrap_names() -> Vec<&'static str> {
    BOOTSTRAP_STEPS.iter().map(|(n, _)| *n).collect()
}

pub fn run(name: &str, env: &ExecEnv) -> Result<BackendOutcome> {
    match name {
        "opencode" => opencode(env),
        other => anyhow::bail!(
            "unknown bootstrap step '{}' (known: {})",
            other,
            known_bootstrap_names().join(", ")
        ),
    }
}

/// opencode installer: the vendor ships a shell installer — downloaded to a
/// temp file and executed; no shell script is kept in this repo.
fn outcome(backend: &'static str) -> BackendOutcome {
    BackendOutcome {
        backend,
        ..Default::default()
    }
}

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
        assert!(err.to_string().contains("opencode"), "{}", err);
    }

    #[test]
    fn opencode_skipped_when_present() {
        let t = TestEnv::new();
        t.stub_ok("opencode", "1.0");
        let out = run("opencode", t.exec()).unwrap();
        assert_eq!(out.unchanged, vec!["opencode"]);
        assert!(t.calls_of("curl").is_empty());
    }
}
