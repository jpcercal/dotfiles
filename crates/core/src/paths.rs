use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    pub state_dir: PathBuf,
    pub state_file: PathBuf,
    pub lock_dir: PathBuf,
    pub log_dir: PathBuf,
    /// Dotfiles root (~/dotfiles) — used for resolving relative resources
    pub dotfiles_dir: PathBuf,
}

impl Paths {
    pub fn from_home(home: &Path) -> Self {
        let state_dir = home.join(".local/state/dotfiles-updater");
        let state_file = state_dir.join("state.json");
        let lock_dir = state_dir.join("lock");
        let log_dir = home.join("dotfiles/logs/dotfiles-updater");
        let dotfiles_dir = home.join("dotfiles");
        Self {
            state_dir,
            state_file,
            lock_dir,
            log_dir,
            dotfiles_dir,
        }
    }

    pub fn detect() -> Self {
        let home = dirs::home_dir().expect("HOME not set");
        Self::from_home(&home)
    }

    /// Ensure state_dir and log_dir exist
    pub fn ensure_dirs(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.state_dir)?;
        std::fs::create_dir_all(&self.log_dir)?;
        Ok(())
    }
}
