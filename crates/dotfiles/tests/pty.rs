//! PTY-level regression test for the progress-UI tty hijack.
//!
//! Reproduces the exact user scenario: a `sudo` password prompt (reading
//! `/dev/tty` directly, like real sudo) fires **while the interactive
//! progress region is active**. The old driver read stdin through crossterm
//! in raw mode, stealing the password bytes and leaving the terminal broken.
//! This test fails (hangs → deadline panic) if the UI ever touches tty
//! input or modes mid-run again.
//!
//! Compiled only with the `tui` feature (headless `--no-default-features`
//! CI never activates the interactive reporter).

#![cfg(feature = "tui")]

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PASSWORD: &str = "topsecret";
/// Whole-test deadline: hang (stolen password) must fail, not block CI.
const DEADLINE: Duration = Duration::from_secs(90);

fn write_exec(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn stub_bin(bin: &Path, name: &str, body: &str) {
    write_exec(&bin.join(name), &format!("#!/bin/sh\n{body}\n"));
}

#[test]
fn progress_ui_never_steals_sudo_password_from_the_tty() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let bin = root.join("bin");
    std::fs::create_dir_all(&home).unwrap();

    // Hermeticity: `ExecEnv::search_path` resolves tools against THIS
    // process's PATH (it builds the child's PATH from it), so the stub dir
    // must come first here — not just in the child's env — or the real
    // Homebrew leaks in and the test gains real-machine effects.
    // (`tests/pty.rs` is its own test binary/process, so this is race-free.)
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.to_string_lossy()));

    // Manifest: one custom step whose post-install hook elevates mid-run,
    // while the progress region is active (the hook runs inside unit
    // execution). `custom:` needs no external package tool, so the fixture
    // is hermetic by construction — `main::ensure_path` prepends real tool
    // dirs (e.g. /opt/homebrew/bin) to PATH, which would otherwise shadow
    // any `brew` stub with the real Homebrew.
    std::fs::write(
        root.join("apps.yaml"),
        "require:\n  - id: \"custom:demotool\"\n    hooks:\n      post-install: \"sudo dscl . -list /\"\n",
    )
    .unwrap();

    stub_bin(&bin, "dscl", "exit 0");
    // Fake sudo: warmups pass silently; real elevations prompt on /dev/tty
    // exactly like sudo does, then record what they received.
    stub_bin(
        &bin,
        "sudo",
        "if [ \"$1\" = \"-v\" ]; then exit 0; fi\n\
         printf 'Password: ' > /dev/tty\n\
         IFS= read -r pw < /dev/tty\n\
         printf '%s' \"$pw\" > \"$HOME/pw.txt\"\n\
         [ \"$pw\" = \"topsecret\" ]",
    );

    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_dotfiles"));
    cmd.arg("install");
    cmd.env("HOME", home.to_string_lossy().as_ref());
    cmd.env("PATH", format!("{}:/usr/bin:/bin", bin.to_string_lossy()));
    cmd.env(
        "DOTFILES_MANIFEST",
        root.join("apps.yaml").to_string_lossy().as_ref(),
    );
    cmd.env("DOTFILES_DIR", root.to_string_lossy().as_ref());
    cmd.env("TERM", "xterm-256color");
    cmd.cwd(root);

    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    let mut writer = pair.master.take_writer().expect("writer");
    let mut reader = pair.master.try_clone_reader().expect("reader");

    let output: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(vec![]));
    let reader_out = output.clone();
    let reader_handle = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => reader_out.lock().unwrap().extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
    });

    let snapshot = || String::from_utf8_lossy(&output.lock().unwrap()).to_string();
    let start = Instant::now();
    let mut sent = false;
    let mut dismissed_review = false;
    let status = loop {
        if start.elapsed() > DEADLINE {
            let _ = child.kill();
            panic!(
                "timed out waiting for install to complete (password stolen?)\n--- pty output ---\n{}",
                snapshot()
            );
        }
        let out = snapshot();
        if !sent && out.contains("Password:") {
            // The hook's sudo prompt is live on the tty: answer it like a user.
            writer
                .write_all(format!("{PASSWORD}\n").as_bytes())
                .expect("write password");
            writer.flush().expect("flush");
            sent = true;
        }
        if !dismissed_review && out.contains("click/Space expand") {
            // Defensive: if the run failed unexpectedly and the failures-only
            // reviewer opened, leave it instead of hanging on the deadline.
            writer.write_all(b"q").expect("dismiss reviewer");
            writer.flush().expect("flush");
            dismissed_review = true;
        }
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None => std::thread::sleep(Duration::from_millis(200)),
        }
    };
    // Release the reader thread, then collect everything.
    drop(writer);
    drop(pair);
    reader_handle.join().expect("reader thread");
    let out = snapshot();

    assert!(sent, "sudo stub never prompted on the tty:\n{out}");
    assert!(
        status.success(),
        "install failed; exit: {}\n--- pty output ---\n{out}",
        status.exit_code()
    );
    let pw_file: PathBuf = home.join("pw.txt");
    let received = std::fs::read_to_string(&pw_file).unwrap_or_default();
    assert_eq!(
        received, PASSWORD,
        "password bytes never reached sudo (stolen mid-flight?):\n{out}"
    );
    assert!(out.contains("▶ install"), "missing section:\n{out}");
    assert!(out.contains("custom"), "missing custom-step recap:\n{out}");
    // No failures → the failures-only reviewer must never open (no keypress
    // needed to leave).
    assert!(
        !out.contains("▶ review"),
        "reviewer opened on a successful run:\n{out}"
    );
}
