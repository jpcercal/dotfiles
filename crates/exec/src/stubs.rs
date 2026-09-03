//! Standard stub-suite for `sync --sandbox`: harmless fakes of every external
//! tool the pipeline touches, recording argv to a log. Generated at runtime —
//! no shell scripts live in the repository.

use std::path::Path;

/// (name, extra body after argv recording) pairs.
const STANDARD_STUBS: &[(&str, &str)] = &[
    ("brew", "if [ \"$1\" = --prefix ]; then echo /opt/homebrew; fi; exit 0"),
    ("mas", "exit 0"),
    ("gem", "exit 0"),
    ("npm", "if [ \"$1\" = ls ]; then echo '{}'; fi; exit 0"),
    ("uv", "if [ \"$1 $2\" = 'python find' ]; then echo \"$HOME/upy/bin/python3\"; elif [ \"$1 $2\" = 'pip list' ]; then echo '[]'; fi; exit 0"),
    ("go", "if [ \"$1 $2\" = 'env GOPATH' ]; then echo \"$HOME/gopath\"; fi; exit 0"),
    ("cargo", "exit 0"),
    ("composer", "exit 0"),
    ("rustup", "exit 0"),
    ("fnm", "exit 0"),
    ("nvim", "exit 0"),
    ("opencode", "exit 0"),
    ("rtk", "exit 0"),
    ("npx", "exit 0"),
    ("git", "exit 0"),
    ("curl", "exit 0"),
    ("defaults", "exit 0"),
    ("dockutil", "exit 0"),
    ("dscl", "echo 'UserShell: /opt/homebrew/bin/zsh'; exit 0"),
    ("sudo", "exit 0"),
    ("sqlite3", "cat >/dev/null; exit 0"),
    ("launchctl", "exit 0"),
    ("softwareupdate", "exit 0"),
    ("pmset", "exit 0"),
    ("osascript", "exit 0"),
    ("killall", "exit 0"),
    ("caffeinate", "exit 0"),
    ("xcode-select", "echo '/Library/Developer/CommandLineTools'; exit 0"),
    ("hostname", "echo sandbox-host; exit 0"),
    ("mkdir", "exit 0"),
    ("chflags", "exit 0"),
    ("systemsetup", "exit 0"),
    ("touch", "exit 0"),
    ("open", "exit 0"),
];

/// Write the standard stubs into `bin_dir`, recording every call to `log_file`.
pub fn install_standard_stubs(bin_dir: &Path, log_file: &Path) -> std::io::Result<()> {
    for (name, body) in STANDARD_STUBS {
        let script = format!(
            "#!/bin/sh\necho '{name} '\"$@\" >> '{log}'\n{body}\n",
            name = name,
            log = log_file.display(),
            body = body
        );
        let path = bin_dir.join(name);
        std::fs::write(&path, script)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}
