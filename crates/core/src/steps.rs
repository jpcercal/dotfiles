use crate::report::StepReport;
use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
    Combined,
}

/// Events streamed from the pipeline. `SudoPrompt.respond` carries the channel
/// the password answer is delivered on (empty string = cancel).
#[derive(Debug)]
pub enum PipelineEvent {
    StepStarted { name: String, index: usize, total: usize },
    LogLine { step: String, stream: LogStream, line: String },
    StepFinished { report: StepReport },
    SudoPrompt { command: String, reason: String, respond: mpsc::Sender<String> },
    RunFinished { status: String, report_path: PathBuf },
}

/// Result of a spawned step
pub struct StepOutcome {
    pub report: StepReport,
    pub exit_code: i32,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Heuristic for password prompts on captured output. `sudo -S` prints
/// "[sudo] Password for user:" / "Password:" with no trailing newline, which
/// is why output must be scanned as raw chunks, not buffered lines.
fn looks_like_password_prompt(buf: &str) -> bool {
    let lower = buf.to_lowercase();
    lower.contains("password for") || lower.contains("[sudo] password") || lower.trim_end().ends_with("password:")
}

/// Ask the GUI for a password (SudoPrompt event) and block until answered.
/// Empty string = user cancelled. Times out after 5 min so a headless run
/// without a GUI listener doesn't hang forever.
fn request_password(event_tx: &Option<mpsc::Sender<PipelineEvent>>, command: &str, reason: &str) -> String {
    if event_tx.is_none() {
        return String::new();
    }
    let (respond_tx, respond_rx) = mpsc::channel::<String>();
    if let Some(tx) = event_tx {
        let _ = tx.send(PipelineEvent::SudoPrompt {
            command: command.to_string(),
            reason: reason.to_string(),
            respond: respond_tx,
        });
    }
    respond_rx
        .recv_timeout(std::time::Duration::from_secs(300))
        .unwrap_or_default()
}

/// Deliver the answer to the child. Empty answer (cancel) closes stdin so the
/// prompt gets EOF and the child fails fast instead of hanging invisibly.
fn deliver_answer(stdin_writer: &mut Option<ChildStdin>, answer: &str) {
    if answer.is_empty() {
        *stdin_writer = None;
    } else if let Some(w) = stdin_writer.as_mut() {
        let _ = writeln!(w, "{}", answer);
        let _ = w.flush();
    }
}

/// Run a command with streaming, writing to per-step log and combined log.
/// If SUDO_ASKPASS is set, sudo will invoke it for password.
/// stdin is captured (never inherited from a terminal): password prompts are
/// detected on the captured output, surfaced as SudoPrompt events, and their
/// answers are written to the child's stdin. Cancelling closes stdin (EOF) so
/// the child fails fast instead of hanging on an invisible prompt.
pub fn run_step(
    name: &str,
    program: &str,
    args: &[&str],
    log_dir: &Path,
    run_id: &str,
    combined_log: &Path,
    event_tx: Option<mpsc::Sender<PipelineEvent>>,
    sudo_askpass: Option<&Path>,
) -> StepOutcome {
    let log_path = log_dir.join(format!("{}.{}.log", run_id, name));
    let start = Instant::now();
    let start_secs = now_secs();

    // Prepare combined log file for appending
    let combined_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(combined_log)
        .ok();

    let step_log_file = std::fs::File::create(&log_path).ok();

    // Build command
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    // Capture stdin in GUI mode — never inherit a tty, so sudo can't hang on
    // /dev/tty and instead uses SUDO_ASKPASS or the piped stdin -S path.
    // In headless/terminal mode (event_tx None) inherit stdin so interactive
    // prompts on the real terminal still work.
    if event_tx.is_some() {
        cmd.stdin(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: setsid is async-signal-safe; called in pre_exec in the child.
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }
    }
    // Ensure we have a PATH that includes homebrew etc.
    // Inherit env, but also set SUDO_ASKPASS if provided
    if let Some(askpass) = sudo_askpass {
        cmd.env("SUDO_ASKPASS", askpass);
    }

    // Header for logs
    let header = format!("\n▶ {}  {}\n# {} {}\n", name, chrono::Local::now().format("%H:%M:%S"), program, args.join(" "));
    if let Some(ref f) = combined_file {
        use std::io::Write;
        let _ = writeln!(f.try_clone().unwrap(), "{}", header);
    }
    if let Some(ref f) = step_log_file {
        use std::io::Write;
        let _ = writeln!(f.try_clone().unwrap(), "{}", header);
    }
    // Also emit as LogLine events
    if let Some(tx) = &event_tx {
        for line in header.lines() {
            let _ = tx.send(PipelineEvent::LogLine { step: name.to_string(), stream: LogStream::Combined, line: line.to_string() });
        }
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let note = format!("failed to spawn {}: {}", program, e);
            let report = StepReport {
                name: name.to_string(),
                status: "failed".into(),
                duration_seconds: 0,
                updated: Value::Array(vec![]),
                failed: Value::Array(vec![]),
                note,
                raw_log: format!("{}.{}.log", run_id, name),
            };
            return StepOutcome { report, exit_code: 127 };
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    // Keep the write end so password answers can be delivered to the child
    let mut stdin_writer: Option<ChildStdin> = child.stdin.take();

    // Spawn reader threads — raw byte chunks, not lines: sudo prompts carry no
    // trailing newline, so line buffering would hide them until the next flush.
    let (log_tx, log_rx) = mpsc::channel::<(LogStream, Vec<u8>)>();

    if let Some(out) = stdout {
        let tx = log_tx.clone();
        std::thread::spawn(move || {
            let mut reader = out;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send((LogStream::Stdout, buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                }
            }
        });
    }
    if let Some(err) = stderr {
        let tx = log_tx.clone();
        std::thread::spawn(move || {
            let mut reader = err;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send((LogStream::Stderr, buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                }
            }
        });
    }
    drop(log_tx);

    // Open files for appending in this thread
    let mut step_file = step_log_file;
    let mut combined = combined_file;

    // Partial-line buffers per stream — prompts may arrive without a newline
    let mut line_bufs: [String; 2] = [String::new(), String::new()];
    let mut prompts_answered = 0usize;
    let command_display = format!("{} {}", program, args.join(" "));

    for (stream, chunk) in log_rx {
        let idx = if stream == LogStream::Stdout { 0 } else { 1 };
        line_bufs[idx].push_str(&String::from_utf8_lossy(&chunk));

        // Emit every complete line to the files and the event stream
        while let Some(pos) = line_bufs[idx].find('\n') {
            let raw: String = line_bufs[idx].drain(..=pos).collect();
            let line = raw.trim_end_matches(&['\n', '\r'][..]).to_string();
            if let Some(ref mut f) = step_file {
                let _ = writeln!(f, "{}", line);
            }
            if let Some(ref mut f) = combined {
                let _ = writeln!(f, "{}", line);
            }
            if let Some(tx) = &event_tx {
                let _ = tx.send(PipelineEvent::LogLine { step: name.to_string(), stream: stream.clone(), line: line.clone() });
                let _ = tx.send(PipelineEvent::LogLine { step: name.to_string(), stream: LogStream::Combined, line: line.clone() });
            }
            // Newline-terminated prompts are answered here too
            if prompts_answered < 3 && looks_like_password_prompt(&line) {
                prompts_answered += 1;
                let answer = request_password(&event_tx, &command_display, &format!("step '{}' requested a password", name));
                deliver_answer(&mut stdin_writer, &answer);
            }
        }

        // Password prompt detected mid-line (no trailing newline) — ask the GUI,
        // then answer on stdin
        if prompts_answered < 3 && looks_like_password_prompt(&line_bufs[idx]) {
            prompts_answered += 1;
            line_bufs[idx].clear();
            let answer = request_password(&event_tx, &command_display, &format!("step '{}' requested a password", name));
            deliver_answer(&mut stdin_writer, &answer);
        }
    }

    // Flush trailing partial output (no trailing newline)
    for (idx, buf) in line_bufs.iter_mut().enumerate() {
        let line = buf.trim_end().to_string();
        if !line.is_empty() {
            let stream = if idx == 0 { LogStream::Stdout } else { LogStream::Stderr };
            if let Some(ref mut f) = step_file {
                let _ = writeln!(f, "{}", line);
            }
            if let Some(ref mut f) = combined {
                let _ = writeln!(f, "{}", line);
            }
            if let Some(tx) = &event_tx {
                let _ = tx.send(PipelineEvent::LogLine { step: name.to_string(), stream: stream.clone(), line: line.clone() });
                let _ = tx.send(PipelineEvent::LogLine { step: name.to_string(), stream: LogStream::Combined, line });
            }
        }
        buf.clear();
    }

    let status = child.wait().expect("wait");
    let code = status.code().unwrap_or(1);
    let duration = start.elapsed().as_secs() as i64;

    // For now, caller will convert to StepReport with appropriate updated/failed.
    // We return a base report; caller fills details.
    let _ = start_secs; // unused but kept for parity
    let report = StepReport {
        name: name.to_string(),
        status: if code == 0 { "success".into() } else { "failed".into() },
        duration_seconds: duration,
        updated: Value::Array(vec![]),
        failed: Value::Array(vec![]),
        note: if code == 0 { "".into() } else { format!("exited {}", code) },
        raw_log: format!("{}.{}.log", run_id, name),
    };
    StepOutcome { report, exit_code: code }
}

pub fn parse_brew_upgraded(log_path: &Path) -> Value {
    let content = std::fs::read_to_string(log_path).unwrap_or_default();
    let mut items = vec![];
    let mut current_name: Option<String> = None;
    for line in content.lines() {
        if line.starts_with("==> Upgrading ") {
            // "==> Upgrading foo"
            if let Some(name) = line.split_whitespace().nth(2) {
                current_name = Some(name.to_string());
            }
        } else if line.starts_with("  ") && line.contains("->") {
            if let Some(name) = current_name.clone() {
                let s = line.trim();
                // "1.0 -> 1.1"
                let parts: Vec<&str> = s.split(" -> ").collect();
                if parts.len() == 2 {
                    let from = parts[0].trim().to_string();
                    let to = parts[1].trim().to_string();
                    items.push(serde_json::json!({"name": name, "from": from, "to": to}));
                }
            }
        } else if line.trim().is_empty() {
            current_name = None;
        }
    }
    Value::Array(items)
}

pub fn brew_deprecated_json() -> Value {
    // best-effort, slow
    let formulae = Command::new("brew").args(["list", "--formula"]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    let list = formulae.split_whitespace().collect::<Vec<_>>().join(" ");
    if list.is_empty() {
        return Value::Array(vec![]);
    }
    let out = Command::new("brew").args(["info", "--json=v2"]).arg(&list).output();
    if let Ok(o) = out {
        let s = String::from_utf8_lossy(&o.stdout);
        if let Ok(v) = serde_json::from_str::<Value>(&s) {
            if let Some(arr) = v.get("formulae").and_then(|x| x.as_array()) {
                let deprecated: Vec<Value> = arr.iter().filter(|f| f.get("deprecated").and_then(|x| x.as_bool()).unwrap_or(false)).filter_map(|f| f.get("name").cloned()).collect();
                return Value::Array(deprecated);
            }
        }
    }
    Value::Array(vec![])
}

pub fn composer_audit_json() -> Value {
    let out = Command::new("composer").args(["global", "audit", "--format=json"]).output();
    if let Ok(o) = out {
        let s = String::from_utf8_lossy(&o.stdout);
        if let Ok(v) = serde_json::from_str::<Value>(&s) {
            return v;
        }
    }
    serde_json::json!({"error":"composer global audit unavailable or failed"})
}

/// Helper to run a bash -c command as a step
pub fn run_bash_step(
    name: &str,
    bash_script: &str,
    log_dir: &Path,
    run_id: &str,
    combined_log: &Path,
    event_tx: Option<mpsc::Sender<PipelineEvent>>,
    sudo_askpass: Option<&Path>,
) -> StepOutcome {
    run_step(name, "bash", &["-c", bash_script], log_dir, run_id, combined_log, event_tx, sudo_askpass)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_prompt_heuristic() {
        assert!(looks_like_password_prompt("[sudo] Password for user:"));
        assert!(looks_like_password_prompt("Password:"));
        assert!(looks_like_password_prompt("password for jpcercal:"));
        assert!(looks_like_password_prompt("Enter password: "));
        assert!(!looks_like_password_prompt("Upgrading 12 packages"));
        assert!(!looks_like_password_prompt(""));
    }

    /// End-to-end: prompt on output → SudoPrompt event → answer written to the
    /// child's captured stdin.
    #[test]
    fn stdin_capture_answers_password_prompt() {
        let (tx, rx) = mpsc::channel();
        let answerer = std::thread::spawn(move || {
            while let Ok(ev) = rx.recv() {
                if let PipelineEvent::SudoPrompt { respond, .. } = ev {
                    let _ = respond.send("hunter2".to_string());
                    break;
                }
            }
        });
        let tmp = std::env::temp_dir();
        let outcome = run_step(
            "stdin-test",
            "bash",
            &["-c", "echo 'Password:'; read -r x || exit 3; [ \"$x\" = \"hunter2\" ] && echo MATCH || exit 4"],
            &tmp,
            "t-stdin",
            &tmp.join("t-stdin-combined.log"),
            Some(tx),
            None,
        );
        let _ = answerer.join();
        assert_eq!(outcome.exit_code, 0, "note: {}", outcome.report.note);
    }

    /// Cancel (empty answer) closes the child's stdin → the read gets EOF.
    #[test]
    fn stdin_cancel_closes_child_stdin() {
        let (tx, rx) = mpsc::channel();
        let answerer = std::thread::spawn(move || {
            while let Ok(ev) = rx.recv() {
                if let PipelineEvent::SudoPrompt { respond, .. } = ev {
                    let _ = respond.send(String::new());
                    break;
                }
            }
        });
        let tmp = std::env::temp_dir();
        let outcome = run_step(
            "stdin-cancel-test",
            "bash",
            &["-c", "echo 'Password:'; read -r x || exit 3; exit 0"],
            &tmp,
            "t-cancel",
            &tmp.join("t-cancel-combined.log"),
            Some(tx),
            None,
        );
        let _ = answerer.join();
        assert_eq!(outcome.exit_code, 3);
    }
}
