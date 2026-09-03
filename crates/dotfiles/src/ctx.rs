//! Shared CLI context: how the binary locates its repo, manifest and execution
//! environment. Tests drive everything through `DOTFILES_DIR` + `DOTFILES_SANDBOX`.

use anyhow::{Context, Result};
use dotfiles_exec::ExecEnv;
use dotfiles_manifest::Manifest;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Ctx {
    pub env: ExecEnv,
    pub dotfiles_dir: PathBuf,
}

impl Ctx {
    /// Real-machine context. `dry_run` forwards `--dry-run` to every spawned command.
    pub fn real(dry_run: bool) -> Self {
        let env = ExecEnv::real().with_dry_run(dry_run);
        let dotfiles_dir = std::env::var_os("DOTFILES_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| env.home.join("dotfiles"));
        Self { env, dotfiles_dir }
    }

    /// Sandboxed context used by `sync --sandbox` and integration tests: HOME
    /// and PATH live under `root`, dotfiles repo is `root/dotfiles`.
    pub fn sandbox(root: &std::path::Path, dry_run: bool) -> Result<Self> {
        let env = ExecEnv::sandbox(root)?.with_dry_run(dry_run);
        Ok(Self {
            env,
            dotfiles_dir: root.join("dotfiles"),
        })
    }

    pub fn manifest_path(&self) -> PathBuf {
        std::env::var_os("DOTFILES_MANIFEST")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.dotfiles_dir.join("apps.yaml"))
    }

    pub fn prefs_path(&self) -> PathBuf {
        std::env::var_os("DOTFILES_PREFS")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.dotfiles_dir.join("prefs.yaml"))
    }

    pub fn commands_path(&self) -> PathBuf {
        std::env::var_os("DOTFILES_COMMANDS")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.dotfiles_dir.join("commands.yaml"))
    }

    pub fn manifest(&self) -> Result<Manifest> {
        let path = self.manifest_path();
        dotfiles_manifest::load_manifest(&path)
            .with_context(|| format!("loading {}", path.display()))
    }

    pub fn commands(&self) -> Result<dotfiles_manifest::CommandsManifest> {
        let path = self.commands_path();
        dotfiles_manifest::load_commands(&path)
            .with_context(|| format!("loading {}", path.display()))
    }
}
