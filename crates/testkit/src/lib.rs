//! Test fixtures for hermetic backend/config tests.
//!
//! Generates stub binaries (shell scripts) on the fly into a temp dir that is
//! prepended to `PATH`, so every command the code-under-test spawns is a fake
//! whose argv is recorded. Stub scripts are **generated at test time** and
//! never committed; the repo itself contains zero shell scripts.

use dotfiles_exec::ExecEnv;
use std::path::{Path, PathBuf};

pub struct TestEnv {
    tmp: tempfile::TempDir,
    exec: ExecEnv,
}

impl TestEnv {
    /// New sandbox: `root/home` (HOME), `root/bin` (PATH prefix), `root/dotfiles` (repo stand-in).
    /// PATH is *isolated* (stub dir + /usr/bin:/bin only) — real tools like
    /// a developer's `brew`/`rtk` never leak into tests.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn exec(&self) -> &ExecEnv {
        &self.exec
    }

    pub fn exec_dry(&self) -> ExecEnv {
        self.exec.clone().with_dry_run(true)
    }

    pub fn root(&self) -> &Path {
        self.tmp.path()
    }

    pub fn home(&self) -> PathBuf {
        self.exec.home.clone()
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.root().join("bin")
    }

    pub fn dotfiles_dir(&self) -> PathBuf {
        self.root().join("dotfiles")
    }

    fn calls_file(&self) -> PathBuf {
        self.root().join("calls.log")
    }

    /// Install a stub binary named `program` whose body (raw shell) runs after
    /// the argv of every call is appended to the calls log.
    pub fn stub(&self, program: &str, body: &str) -> &Self {
        let path = self.bin_dir().join(program);
        let script = format!(
            "#!/bin/sh\necho '{name} '\"$@\" >> '{calls}'\n{body}\n",
            name = program,
            calls = self.calls_file().display(),
            body = body,
        );
        write_exec(&path, &script);
        self
    }

    /// Stub that prints `stdout` and exits 0.
    pub fn stub_ok(&self, program: &str, stdout: &str) -> &Self {
        self.stub(
            program,
            &format!("printf '%s' '{}'\nexit 0", stdout.replace('\'', "'\\''")),
        )
    }

    /// Stub that always fails with `code`.
    pub fn stub_fail(&self, program: &str, code: i32) -> &Self {
        self.stub(program, &format!("exit {}", code))
    }

    /// All recorded calls, one `program arg arg...` line each.
    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.calls_file())
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim_end().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    /// Recorded calls of one program, argv only (program name stripped).
    pub fn calls_of(&self, program: &str) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter_map(|l| {
                l.strip_prefix(&format!("{} ", program))
                    .map(|s| s.to_string())
            })
            .collect()
    }

    /// Write a file inside the sandbox (paths relative to sandbox root; `home/` and
    /// `dotfiles/` are the interesting prefixes), creating parents.
    pub fn write(&self, rel: &str, content: &str) -> PathBuf {
        let path = self.root().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }
}

impl Default for TestEnv {
    fn default() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let exec = ExecEnv::sandbox(tmp.path())
            .expect("sandbox env")
            .with_isolated_base_paths(&["/usr/bin", "/bin"]);
        let _ = std::fs::create_dir_all(tmp.path().join("dotfiles"));
        Self { tmp, exec }
    }
}

fn write_exec(path: &Path, content: &str) {
    std::fs::write(path, content).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_records_argv_and_reports_success() {
        let t = TestEnv::new();
        t.stub_ok("brew", "ok\n");
        let out = t
            .exec()
            .output("brew", &["install", "ripgrep", "--quiet"])
            .unwrap();
        assert!(out.ok());
        assert_eq!(out.stdout.trim(), "ok");
        assert_eq!(t.calls(), vec!["brew install ripgrep --quiet"]);
        assert_eq!(t.calls_of("brew"), vec!["install ripgrep --quiet"]);
    }

    #[test]
    fn failing_stub_propagates_exit_code() {
        let t = TestEnv::new();
        t.stub_fail("mas", 5);
        let out = t.exec().output("mas", &["install", "123"]).unwrap();
        assert_eq!(out.status, 5);
    }

    #[test]
    fn home_is_sandboxed() {
        let t = TestEnv::new();
        let out = t.exec().output("sh", &["-c", "echo $HOME"]).unwrap();
        assert_eq!(out.stdout.trim(), t.home().to_string_lossy());
    }
}
