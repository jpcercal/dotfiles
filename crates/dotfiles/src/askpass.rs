use clap::Parser;
use std::io::{self, Write};

#[derive(Parser, Debug)]
pub struct AskpassArgs {
    /// Prompt from sudo (ignored, we show our own UI or fallback)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub prompt: Vec<String>,
}

/// Minimal askpass: try to show egui password prompt if DISPLAY available,
/// otherwise fallback to reading from /dev/tty or failing.
/// For now, we implement a simple tty fallback and an osascript fallback on macOS
/// so the binary is self-contained. The full egui sheet integration comes via
/// the running upgrade process's unix socket — this binary is used when no
/// upgrade GUI is running (e.g. manual brew).
pub fn run(args: AskpassArgs) -> anyhow::Result<()> {
    let prompt = args.prompt.join(" ");
    let cmd = std::env::var("SUDO_COMMAND").unwrap_or_else(|_| "sudo operation".into());
    // Derive reason similar to old script
    let reason = derive_reason(&cmd);

    // Try to prompt via osascript (native dialog) as fallback — this keeps
    // the askpass working even without the egui event loop.
    // In the future, we will try to connect to the upgrade's unix socket first.
    let socket_path = crate::upgrade::askpass_socket_path();
    if let Some(pw) = try_socket_askpass(&socket_path, &cmd, &reason) {
        print!("{}", pw);
        io::stdout().flush()?;
        return Ok(());
    }

    // Fallback: osascript dialog
    if let Some(pw) = try_osascript_askpass(&cmd, &reason) {
        print!("{}", pw);
        io::stdout().flush()?;
        return Ok(());
    }

    // Last resort: read from tty
    eprint!("{}: ", prompt);
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    print!("{}", line.trim_end());
    io::stdout().flush()?;
    Ok(())
}

fn derive_reason(cmd: &str) -> String {
    let lower = cmd.to_lowercase();
    if lower.contains("discord") {
        "upgrade the Discord cask (removing launchctl service com.discord.discord.ShipIt and installing to /Applications)".into()
    } else if lower.contains("docker") {
        "upgrade Docker Desktop (managing system services and privileged helpers)".into()
    } else if lower.contains("google-chrome") || lower.contains("chrome") {
        "upgrade Google Chrome (installing to /Applications and managing system services)".into()
    } else if lower.contains("cask") || lower.contains("brew upgrade") {
        "upgrade Homebrew casks that require system modifications".into()
    } else {
        "manage system services and install to /Applications".into()
    }
}

fn try_socket_askpass(socket: &std::path::Path, cmd: &str, reason: &str) -> Option<String> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;
    let mut stream = UnixStream::connect(socket).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(300)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let req = serde_json::json!({"type":"sudo_prompt","command": cmd, "reason": reason});
    let line = serde_json::to_string(&req).ok()? + "\n";
    stream.write_all(line.as_bytes()).ok()?;
    stream.flush().ok()?;
    let mut reader = BufReader::new(&stream);
    let mut resp = String::new();
    reader.read_line(&mut resp).ok()?;
    let v: serde_json::Value = serde_json::from_str(&resp).ok()?;
    if v.get("cancel").and_then(|x| x.as_bool()).unwrap_or(false) {
        std::process::exit(1);
    }
    v.get("password")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

fn try_osascript_askpass(cmd: &str, reason: &str) -> Option<String> {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let cmd_esc = esc(cmd);
    let reason_esc = esc(reason);
    let script = format!(
        r#"tell application "System Events"
  activate
  display dialog "dotfiles needs your password to allow:\n\n\"{}\"\n\nThis is required to {}.\n\nPlease enter your password to proceed:" default answer "" with title "dotfiles — sudo required" with hidden answer buttons {{"Cancel", "OK"}} default button "OK" with icon caution
  return text returned of result
end tell"#,
        cmd_esc, reason_esc
    );
    let out = std::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}
