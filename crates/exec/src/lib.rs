use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub mod stubs;

/// Result of running a command through an [`ExecEnv`].
#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ExecOutput {
    pub fn ok(&self) -> bool {
        self.status == 0
    }
}

/// The single seam through which every external command runs.
///
/// `ExecEnv::real()` talks to the actual machine; `ExecEnv::sandbox(root)`
/// redirects `HOME` and `PATH` into a scratch directory populated with stub
/// binaries, which makes full end-to-end tests hermetic and parallel-safe.
#[derive(Debug, Clone)]
pub struct ExecEnv {
    pub home: PathBuf,
    /// Directories prepended to PATH for every spawned command (stub dir first).
    pub path_prefix: Vec<PathBuf>,
    pub dry_run: bool,
    /// Extra environment overlay applied to every spawned command.
    pub env: BTreeMap<String, String>,
    /// When set, replaces the inherited PATH behind `path_prefix` (test
    /// hermeticity: only stub dir + these base dirs are visible).
    pub base_paths: Option<Vec<PathBuf>>,
}

impl ExecEnv {
    /// The real execution environment of the current user.
    pub fn real() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        Self {
            home,
            path_prefix: Vec::new(),
            dry_run: false,
            env: BTreeMap::new(),
            base_paths: None,
        }
    }

    /// A sandboxed environment rooted at `root`:
    /// - `HOME` = `root/home`
    /// - `PATH` = `root/bin` (stubs) + system paths
    pub fn sandbox(root: &Path) -> Result<Self> {
        let home = root.join("home");
        let bin = root.join("bin");
        std::fs::create_dir_all(&home).context("create sandbox home")?;
        std::fs::create_dir_all(&bin).context("create sandbox bin")?;
        Ok(Self {
            home,
            path_prefix: vec![bin],
            dry_run: false,
            env: BTreeMap::new(),
            base_paths: None,
        })
    }

    /// Restrict PATH to `path_prefix` + `dirs` (system tools no longer leak in).
    pub fn with_isolated_base_paths(mut self, dirs: &[&str]) -> Self {
        self.base_paths = Some(dirs.iter().map(PathBuf::from).collect());
        self
    }

    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.insert(key.to_string(), value.to_string());
        self
    }

    /// Expand a path string that may start with `~` or `$HOME` against this env's home.
    pub fn expand(&self, p: &str) -> PathBuf {
        if let Some(rest) = p.strip_prefix("~/") {
            return self.home.join(rest);
        }
        if p == "~" {
            return self.home.clone();
        }
        if let Some(rest) = p.strip_prefix("$HOME/") {
            return self.home.join(rest);
        }
        PathBuf::from(p)
    }

    /// Resolve `program` to an absolute path using this env's PATH prefix first,
    /// then the inherited PATH. Does not spawn a process.
    pub fn which(&self, program: &str) -> Option<PathBuf> {
        if program.contains('/') {
            let p = PathBuf::from(program);
            return if p.is_file() { Some(p) } else { None };
        }
        for dir in self.search_path() {
            let candidate = dir.join(program);
            if candidate.is_file() && is_executable(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    pub fn has_command(&self, program: &str) -> bool {
        self.which(program).is_some()
    }

    fn search_path(&self) -> Vec<PathBuf> {
        let mut dirs = self.path_prefix.clone();
        match &self.base_paths {
            Some(base) => dirs.extend(base.iter().cloned()),
            None => {
                if let Some(path) = std::env::var_os("PATH") {
                    dirs.extend(std::env::split_paths(&path));
                }
            }
        }
        dirs
    }

    /// Build a `Command` with this environment applied (PATH prefix, HOME, env overlay).
    pub fn command(&self, program: &str, args: &[&str]) -> Command {
        let resolved = self
            .which(program)
            .unwrap_or_else(|| PathBuf::from(program));
        let mut cmd = Command::new(resolved);
        cmd.args(args);
        cmd.env("HOME", &self.home);
        let path = std::env::join_paths(self.search_path()).unwrap_or_default();
        cmd.env("PATH", &path);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd
    }

    /// Run a command capturing stdout/stderr. In dry-run mode nothing is spawned;
    /// the command is printed and a success output is returned.
    pub fn output(&self, program: &str, args: &[&str]) -> Result<ExecOutput> {
        if self.dry_run {
            println!("[dry-run] {} {}", program, args.join(" "));
            return Ok(ExecOutput {
                status: 0,
                stdout: String::new(),
                stderr: String::new(),
            });
        }
        let out = self
            .command(program, args)
            .output()
            .with_context(|| format!("failed to spawn {}", program))?;
        Ok(ExecOutput {
            status: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        })
    }

    /// Run a command with `input` piped to its stdin (e.g. answering an
    /// installer's interactive prompt non-interactively).
    pub fn output_stdin(&self, program: &str, args: &[&str], input: &str) -> Result<ExecOutput> {
        use std::io::Write;
        if self.dry_run {
            println!("[dry-run] {} {} < stdin", program, args.join(" "));
            return Ok(ExecOutput {
                status: 0,
                stdout: String::new(),
                stderr: String::new(),
            });
        }
        let mut child = self
            .command(program, args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn {}", program))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input.as_bytes());
        }
        let out = child.wait_with_output()?;
        Ok(ExecOutput {
            status: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        })
    }

    /// Run a command and return its exit status (stdout/stderr inherited).
    pub fn status(&self, program: &str, args: &[&str]) -> Result<i32> {
        if self.dry_run {
            println!("[dry-run] {} {}", program, args.join(" "));
            return Ok(0);
        }
        let status = self
            .command(program, args)
            .status()
            .with_context(|| format!("failed to spawn {}", program))?;
        Ok(status.code().unwrap_or(-1))
    }
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_expansion_uses_env_home() {
        let env = ExecEnv {
            home: PathBuf::from("/fake/home"),
            path_prefix: vec![],
            dry_run: false,
            env: BTreeMap::new(),
            base_paths: None,
        };
        assert_eq!(
            env.expand("~/.config/nvim"),
            PathBuf::from("/fake/home/.config/nvim")
        );
        assert_eq!(
            env.expand("$HOME/.zshrc"),
            PathBuf::from("/fake/home/.zshrc")
        );
        assert_eq!(
            env.expand("/absolute/path"),
            PathBuf::from("/absolute/path")
        );
        assert_eq!(env.expand("~"), PathBuf::from("/fake/home"));
    }

    #[test]
    fn dry_run_spawns_nothing_and_succeeds() {
        let env = ExecEnv::real().with_dry_run(true);
        // "false" exits 1 for real; dry-run must not spawn it.
        let out = env.output("false", &[]).unwrap();
        assert_eq!(out.status, 0);
        let rc = env.status("false", &[]).unwrap();
        assert_eq!(rc, 0);
    }

    #[test]
    fn which_prefers_path_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let env = ExecEnv::sandbox(tmp.path()).unwrap();
        let stub = tmp.path().join("bin/mycmd");
        std::fs::write(&stub, "#!/bin/sh\necho stub\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let found = env.which("mycmd").expect("stub must be found");
        assert_eq!(found, stub);
    }

    #[test]
    fn sandbox_home_overrides_home_for_children() {
        let tmp = tempfile::tempdir().unwrap();
        let env = ExecEnv::sandbox(tmp.path()).unwrap();
        let out = env.output("sh", &["-c", "echo $HOME"]).unwrap();
        assert_eq!(out.stdout.trim(), tmp.path().join("home").to_string_lossy());
    }

    #[test]
    fn output_captures_failure() {
        let env = ExecEnv::real();
        let out = env.output("sh", &["-c", "echo err 1>&2; exit 3"]).unwrap();
        assert_eq!(out.status, 3);
        assert_eq!(out.stderr.trim(), "err");
    }
}
