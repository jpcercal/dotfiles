use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

pub mod report;
pub mod stubs;

pub use report::{elevated_command, Event, NoopReporter, RecordingReporter, Reporter, Stream};

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
///
/// Every spawn is announced through the attached [`Reporter`]: the exact
/// command line (`Event::Command`), an elevation notice for `sudo`
/// (`Event::Elevate`), and — when a unit context is set via [`ExecEnv::for_unit`]
/// — per-line stdout/stderr (`Event::UnitLog`). Library crates never print.
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
    /// Where user-facing feedback goes (shared across clones and threads).
    pub reporter: Arc<dyn Reporter>,
    /// Schedulable unit this env executes for (`brew-formula:git`, …).
    /// Captured output lines are attributed to it via `Event::UnitLog`.
    pub unit: Option<String>,
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
            reporter: Arc::new(NoopReporter),
            unit: None,
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
            reporter: Arc::new(NoopReporter),
            unit: None,
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

    /// Attach a [`Reporter`] for user-facing feedback (shared across clones).
    pub fn with_reporter(mut self, reporter: Arc<dyn Reporter>) -> Self {
        self.reporter = reporter;
        self
    }

    /// Scope this env to one schedulable unit: captured output lines are
    /// attributed to `id` via `Event::UnitLog` so the renderer can group
    /// them into that unit's block.
    pub fn for_unit(mut self, id: &str) -> Self {
        self.unit = Some(id.to_string());
        self
    }

    /// Emit one user-facing event through the attached reporter.
    pub fn report(&self, event: Event) {
        self.reporter.report(event);
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

    /// Run a command capturing stdout/stderr. Announces the exact command
    /// line (`Event::Command`), an elevation notice for `sudo`
    /// (`Event::Elevate`), and per-line output attributed to the unit
    /// context (`Event::UnitLog`). In dry-run mode nothing is spawned.
    pub fn output(&self, program: &str, args: &[&str]) -> Result<ExecOutput> {
        let unit = self.unit.clone();
        let reporter = self.reporter.clone();
        // `run_streamed` announces the spawn; the callback attributes every
        // captured line to this env's unit context.
        let mut attribute = |stream, line: String| {
            if let Some(id) = &unit {
                reporter.report(Event::UnitLog {
                    id: id.clone(),
                    stream,
                    line,
                });
            }
        };
        self.run_streamed(program, args, &mut attribute)
    }

    /// Run an elevated command with an explicit reason shown to the user
    /// (`sudo …` + why it needs root). The reasoned announcement replaces
    /// the automatic sniff — the command is announced exactly once.
    pub fn elevate(&self, program: &str, args: &[&str], reason: &str) -> Result<ExecOutput> {
        self.report(Event::Elevate {
            command: report::display_argv(program, args),
            reason: reason.to_string(),
        });
        self.report(Event::Command {
            argv: report::display_argv(program, args),
            dry_run: self.dry_run,
            unit: self.unit.clone(),
        });
        if self.dry_run {
            return Ok(ExecOutput {
                status: 0,
                stdout: String::new(),
                stderr: String::new(),
            });
        }
        self.spawn_capturing(program, args, None, &mut |_, _| {})
    }

    /// Announce a spawn: the exact command line, plus — for `sudo` — what is
    /// being elevated. Called before any spawn (including dry-run echoes).
    fn announce(&self, program: &str, args: &[&str], reason: Option<&str>) {
        if let Some(command) = report::elevated_command(program, args) {
            self.report(Event::Elevate {
                command: report::display_argv(program, args),
                reason: reason.map(str::to_string).unwrap_or_else(|| {
                    format!("elevates to `{command}` — requires administrator privileges")
                }),
            });
        }
        self.report(Event::Command {
            argv: report::display_argv(program, args),
            dry_run: self.dry_run,
            unit: self.unit.clone(),
        });
    }

    /// Run a command with `input` piped to its stdin (e.g. answering an
    /// installer's interactive prompt non-interactively).
    pub fn output_stdin(&self, program: &str, args: &[&str], input: &str) -> Result<ExecOutput> {
        use std::io::Write;
        self.announce(program, args, None);
        if self.dry_run {
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
        let output = ExecOutput {
            status: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        };
        self.emit_captured(&output);
        Ok(output)
    }

    /// Run a command and return its exit status (stdout/stderr inherited).
    pub fn status(&self, program: &str, args: &[&str]) -> Result<i32> {
        self.announce(program, args, None);
        if self.dry_run {
            return Ok(0);
        }
        let status = self
            .command(program, args)
            .status()
            .with_context(|| format!("failed to spawn {}", program))?;
        Ok(status.code().unwrap_or(-1))
    }

    /// Run a command capturing stdout/stderr while delivering every output
    /// line to `on_line` as it arrives (reader threads over both pipes).
    /// The full captured streams are still returned, so error summarizers
    /// keep working. `output()` is this with a unit-attributing callback.
    pub fn run_streamed(
        &self,
        program: &str,
        args: &[&str],
        on_line: &mut dyn FnMut(Stream, String),
    ) -> Result<ExecOutput> {
        self.announce(program, args, None);
        if self.dry_run {
            return Ok(ExecOutput {
                status: 0,
                stdout: String::new(),
                stderr: String::new(),
            });
        }
        self.spawn_capturing(program, args, None, &mut |s, l| on_line(s, l.to_string()))
    }

    /// Spawn with piped stdout/stderr, stream lines to `on_line`, return all
    /// captured output. `stdin_input`, when set, is written then the pipe is
    /// closed before reading output.
    fn spawn_capturing(
        &self,
        program: &str,
        args: &[&str],
        stdin_input: Option<&str>,
        on_line: &mut dyn FnMut(Stream, &str),
    ) -> Result<ExecOutput> {
        use std::io::{BufRead, BufReader, Write};
        let mut child = self
            .command(program, args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn {}", program))?;
        if let Some(input) = stdin_input {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(input.as_bytes());
            }
        } else {
            drop(child.stdin.take());
        }
        let stdout = child.stdout.take().map(BufReader::new);
        let stderr = child.stderr.take().map(BufReader::new);
        // One thread per pipe pushes lines into a channel; the calling thread
        // drains, invokes the callback, and accumulates the full streams.
        // The callback runs on the calling thread; only the channel senders
        // cross threads, so any `FnMut` works.
        let (tx, rx) = std::sync::mpsc::channel::<(Stream, String)>();
        let mut handles = vec![];
        if let Some(r) = stdout {
            let tx = tx.clone();
            handles.push(std::thread::spawn(move || {
                for line in r.lines() {
                    if let Ok(l) = line {
                        let _ = tx.send((Stream::Stdout, l));
                    } else {
                        break;
                    }
                }
            }));
        }
        if let Some(r) = stderr {
            let tx = tx.clone();
            handles.push(std::thread::spawn(move || {
                for line in r.lines() {
                    if let Ok(l) = line {
                        let _ = tx.send((Stream::Stderr, l));
                    } else {
                        break;
                    }
                }
            }));
        }
        drop(tx);
        let mut out_stdout = String::new();
        let mut out_stderr = String::new();
        for (stream, line) in rx {
            on_line(stream, &line);
            match stream {
                Stream::Stdout => {
                    out_stdout.push_str(&line);
                    out_stdout.push('\n');
                }
                Stream::Stderr => {
                    out_stderr.push_str(&line);
                    out_stderr.push('\n');
                }
            }
        }
        for h in handles {
            let _ = h.join();
        }
        let status = child.wait()?;
        Ok(ExecOutput {
            status: status.code().unwrap_or(-1),
            stdout: out_stdout,
            stderr: out_stderr,
        })
    }

    /// Emit already-captured output as per-line unit events (for paths like
    /// `output_stdin` that cannot stream).
    fn emit_captured(&self, output: &ExecOutput) {
        let Some(id) = &self.unit else { return };
        for line in output.stdout.lines() {
            self.report(Event::UnitLog {
                id: id.clone(),
                stream: Stream::Stdout,
                line: line.to_string(),
            });
        }
        for line in output.stderr.lines() {
            self.report(Event::UnitLog {
                id: id.clone(),
                stream: Stream::Stderr,
                line: line.to_string(),
            });
        }
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
            reporter: Arc::new(NoopReporter),
            unit: None,
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

    #[test]
    fn output_announces_command_and_unit_lines() {
        let reporter = Arc::new(RecordingReporter::new());
        let tmp = tempfile::tempdir().unwrap();
        let env = ExecEnv::sandbox(tmp.path())
            .unwrap()
            .with_reporter(reporter.clone())
            .for_unit("brew-formula:git");
        let out = env
            .output("sh", &["-c", "echo out-line; echo err-line 1>&2"])
            .unwrap();
        assert!(out.ok());
        let events = reporter.events();
        // Exact command echo first, then the two attributed lines.
        assert!(
            matches!(&events[0], Event::Command { argv, dry_run: false, unit } if argv == "sh -c echo out-line; echo err-line 1>&2" && unit.as_deref() == Some("brew-formula:git")),
            "{events:?}"
        );
        assert!(
            events.contains(&Event::UnitLog {
                id: "brew-formula:git".into(),
                stream: Stream::Stdout,
                line: "out-line".into(),
            }),
            "{events:?}"
        );
        assert!(
            events.contains(&Event::UnitLog {
                id: "brew-formula:git".into(),
                stream: Stream::Stderr,
                line: "err-line".into(),
            }),
            "{events:?}"
        );
    }

    #[test]
    fn output_without_unit_context_emits_no_unit_logs() {
        let reporter = Arc::new(RecordingReporter::new());
        let env = ExecEnv::real().with_reporter(reporter.clone());
        let out = env.output("sh", &["-c", "echo hi"]).unwrap();
        assert!(out.ok());
        let events = reporter.events();
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], Event::Command { .. }));
    }

    #[test]
    fn sudo_spawn_announces_elevation_with_inner_command() {
        let reporter = Arc::new(RecordingReporter::new());
        let tmp = tempfile::tempdir().unwrap();
        let env = ExecEnv::sandbox(tmp.path())
            .unwrap()
            .with_reporter(reporter.clone());
        // `true` exists on PATH even in the sandbox (system base paths leak
        // through unless isolated — resolve via `sh` to be hermetic).
        let out = env.output("sh", &["-c", "exit 0"]).unwrap();
        assert!(out.ok());
        // Plain commands never elevate …
        assert!(!reporter
            .events()
            .iter()
            .any(|e| matches!(e, Event::Elevate { .. })),);
        // … but sudo does, echoing the full argv and the inner command.
        let out = env.output("sudo", &["-n", "true"]).unwrap();
        let _ = out;
        let elevate = reporter
            .events()
            .iter()
            .find_map(|e| match e {
                Event::Elevate { command, reason } => Some((command.clone(), reason.clone())),
                _ => None,
            })
            .expect("sudo must announce elevation");
        assert_eq!(elevate.0, "sudo -n true");
        assert!(elevate.1.contains("true"), "{}", elevate.1);
    }

    #[test]
    fn elevate_uses_explicit_reason_and_announces_once() {
        let reporter = Arc::new(RecordingReporter::new());
        let tmp = tempfile::tempdir().unwrap();
        let env = ExecEnv::sandbox(tmp.path())
            .unwrap()
            .with_reporter(reporter.clone());
        let stub = tmp.path().join("bin/sudo");
        std::fs::write(&stub, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let out = env
            .elevate("sudo", &["-v"], "casks may need admin rights")
            .unwrap();
        assert!(out.ok());
        let elevates: Vec<_> = reporter
            .events()
            .iter()
            .filter_map(|e| match e {
                Event::Elevate { command, reason } => Some((command.clone(), reason.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            elevates,
            vec![(
                "sudo -v".to_string(),
                "casks may need admin rights".to_string()
            )]
        );
    }

    #[test]
    fn dry_run_announces_but_spawns_nothing() {
        let reporter = Arc::new(RecordingReporter::new());
        let env = ExecEnv::real()
            .with_dry_run(true)
            .with_reporter(reporter.clone());
        let out = env.output("false", &[]).unwrap();
        assert_eq!(out.status, 0);
        assert!(matches!(
            &reporter.events()[..],
            [Event::Command { argv, dry_run: true, .. }] if argv == "false"
        ));
    }

    #[test]
    fn run_streamed_delivers_lines_and_full_capture() {
        let env = ExecEnv::real();
        let mut seen: Vec<(Stream, String)> = vec![];
        let mut collect = |s, l| seen.push((s, l));
        let out = env
            .run_streamed(
                "sh",
                &["-c", "echo one; echo two 1>&2; echo three"],
                &mut collect,
            )
            .unwrap();
        assert!(out.ok());
        assert_eq!(out.stdout, "one\nthree\n");
        assert_eq!(out.stderr, "two\n");
        let stdout: Vec<&str> = seen
            .iter()
            .filter(|(s, _)| *s == Stream::Stdout)
            .map(|(_, l)| l.as_str())
            .collect();
        let stderr: Vec<&str> = seen
            .iter()
            .filter(|(s, _)| *s == Stream::Stderr)
            .map(|(_, l)| l.as_str())
            .collect();
        // Per-stream order is preserved (cross-stream interleave is not).
        assert_eq!(stdout, vec!["one", "three"]);
        assert_eq!(stderr, vec!["two"]);
    }

    #[test]
    fn output_stdin_attributes_captured_lines_to_unit() {
        let reporter = Arc::new(RecordingReporter::new());
        let env = ExecEnv::real()
            .with_reporter(reporter.clone())
            .for_unit("custom:rustup");
        let out = env.output_stdin("cat", &[], "hello\n").unwrap();
        assert!(out.ok());
        assert_eq!(out.stdout, "hello\n");
        assert!(reporter.events().contains(&Event::UnitLog {
            id: "custom:rustup".into(),
            stream: Stream::Stdout,
            line: "hello".into(),
        }));
    }

    #[test]
    fn status_announces_command() {
        let reporter = Arc::new(RecordingReporter::new());
        let env = ExecEnv::real().with_reporter(reporter.clone());
        let rc = env.status("true", &[]).unwrap();
        assert_eq!(rc, 0);
        assert!(matches!(
            &reporter.events()[..],
            [Event::Command { argv, dry_run: false, .. }] if argv == "true"
        ));
    }
}
