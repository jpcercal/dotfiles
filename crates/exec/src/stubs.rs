//! Standard stub-suite for `sync --sandbox`: harmless fakes of every external
//! tool the pipeline touches, recording argv to a log. Generated at runtime —
//! no shell scripts live in the repository.

use std::path::Path;

/// (name, extra body after argv recording) pairs.
/// `$SANDBOX` is exported into every stub and points at the sandbox root, so
/// stubs never resolve paths on the real machine (hermeticity).
const STANDARD_STUBS: &[(&str, &str)] = &[
    ("brew", "if [ \"$1\" = --prefix ]; then echo \"$SANDBOX/homebrew\"; fi; exit 0"),
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
    ("dscl", "echo \"UserShell: $SANDBOX/homebrew/bin/zsh\"; exit 0"),
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
/// `sandbox_root` is exported as `$SANDBOX` inside every stub.
pub fn install_standard_stubs(
    bin_dir: &Path,
    log_file: &Path,
    sandbox_root: &Path,
) -> std::io::Result<()> {
    for (name, body) in STANDARD_STUBS {
        let script = format!(
            "#!/bin/sh\nexport SANDBOX='{root}'\necho '{name} '\"$@\" >> '{log}'\n{body}\n",
            root = sandbox_root.display(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_stubs_record_argv_and_stay_sandboxed() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        let log = tmp.path().join("calls.log");
        std::fs::create_dir_all(&bin).unwrap();
        install_standard_stubs(&bin, &log, tmp.path()).unwrap();

        // every expected tool has an executable stub
        for tool in ["brew", "mas", "defaults", "dockutil", "sudo", "launchctl"] {
            let p = bin.join(tool);
            assert!(p.is_file(), "missing stub {}", tool);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_ne!(
                    std::fs::metadata(&p).unwrap().permissions().mode() & 0o111,
                    0
                );
            }
        }

        // brew --prefix resolves INSIDE the sandbox, never /opt/homebrew
        let out = std::process::Command::new(bin.join("brew"))
            .arg("--prefix")
            .output()
            .unwrap();
        let prefix = String::from_utf8(out.stdout).unwrap();
        assert_eq!(prefix.trim(), tmp.path().join("homebrew").to_string_lossy());
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(calls.contains("brew --prefix"), "{}", calls);
    }
}
