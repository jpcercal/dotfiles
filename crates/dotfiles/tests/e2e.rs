//! End-to-end: run the real binary's full pipeline against a stub sandbox.
//! Asserts the whole thing completes with zero real-machine side effects.

use std::process::Command;

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn sync_sandbox_completes_all_jobs() {
    let out = Command::new(env!("CARGO_BIN_EXE_dotfiles"))
        .args(["sync", "--sandbox"])
        .env("DOTFILES_DIR", repo_root())
        .env("ATUIN_SESSION", "e2e-test")
        .env_remove("DOTFILES_MANIFEST")
        .env_remove("DOTFILES_PREFS")
        .env_remove("DOTFILES_COMMANDS")
        .output()
        .expect("spawn dotfiles");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "sync --sandbox failed.\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    assert!(stdout.contains("sync: done"), "{}", stdout);
    // All default jobs ran, in order
    let positions: Vec<Option<usize>> = ["bootstrap", "install", "apply", "prefs", "history"]
        .iter()
        .map(|j| stdout.find(j))
        .collect();
    assert!(
        positions.iter().all(Option::is_some),
        "missing job in output: {}",
        stdout
    );
    let pos: Vec<usize> = positions.into_iter().map(Option::unwrap).collect();
    assert!(
        pos.windows(2).all(|w| w[0] < w[1]),
        "jobs out of order: {}",
        stdout
    );
}

#[test]
fn sync_sandbox_skip_honored() {
    let out = Command::new(env!("CARGO_BIN_EXE_dotfiles"))
        .args(["sync", "--sandbox", "--skip", "install,apply,prefs"])
        .env("DOTFILES_DIR", repo_root())
        .env("ATUIN_SESSION", "e2e-test")
        .output()
        .expect("spawn dotfiles");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{}", stdout);
    assert!(stdout.contains("jobs = bootstrap, history"), "{}", stdout);
    assert!(!stdout.contains("▶ install"), "{}", stdout);
}

#[test]
fn doctor_is_non_fatal_output() {
    // doctor in the test env: brew missing on this CI scenario is fine —
    // we only assert it runs and prints the check rows.
    let out = Command::new(env!("CARGO_BIN_EXE_dotfiles"))
        .args(["doctor"])
        .env("DOTFILES_DIR", repo_root())
        .output()
        .expect("spawn dotfiles");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("homebrew"), "{}", stdout);
    assert!(stdout.contains("manifest"), "{}", stdout);
}

/// The zero-shell gate: after the migration the repository must contain no
/// shell/JXA automation scripts and no Makefile. Shell scripts generated at
/// runtime (test stubs, askpass wrapper) live outside the repo and are exempt.
/// Dotfile *configs* (.zshrc, .zshenv, .zprofile) are data synced by `apply`,
/// not automation, and are exempt too.
#[test]
fn repo_contains_no_shell_scripts_or_makefile() {
    let out = Command::new("git")
        .args(["ls-files"])
        .current_dir(repo_root())
        .output()
        .expect("git ls-files");
    let files = String::from_utf8(out.stdout).unwrap();
    let offenders: Vec<&str> = files
        .lines()
        .filter(|f| {
            *f == "Makefile"
                || f.starts_with("scripts/")
                || f.starts_with("bin/")
                || f.ends_with(".sh")
                || f.ends_with(".bash")
                || f.ends_with(".zsh")
                || f.ends_with(".jxa")
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "shell scripts still tracked: {:?}",
        offenders
    );
}

#[test]
fn schema_export_matches_committed_file() {
    for kind in ["apps", "prefs"] {
        let out = Command::new(env!("CARGO_BIN_EXE_dotfiles"))
            .args(["schema", "--kind", kind])
            .output()
            .expect("spawn dotfiles");
        assert!(out.status.success());
        let exported = String::from_utf8(out.stdout).unwrap();
        let committed = std::fs::read_to_string(repo_root().join("schema").join(format!("{}.schema.json", kind)))
            .unwrap_or_else(|_| panic!("schema/{}.schema.json must exist and be committed (regen: dotfiles schema --kind {} --write)", kind, kind));
        assert_eq!(
            exported.trim(),
            committed.trim(),
            "schema/{}.schema.json is stale — regenerate with `dotfiles schema --kind {} --write`",
            kind,
            kind
        );
    }
}
