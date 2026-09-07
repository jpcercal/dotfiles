//! Structured user-facing feedback for everything the tool runs.
//!
//! Library crates (`backends`, `prefs`, …) never print directly. Instead they
//! emit [`Event`]s through the [`Reporter`] carried by [`ExecEnv`](crate::ExecEnv)
//! (an `Arc`, so it survives `ExecEnv::clone` and crosses scheduler threads).
//! The CLI installs a terminal renderer; tests install a recorder or nothing
//! ([`NoopReporter`], the default).
//!
//! Layout contract (see the `TermReporter` in the CLI crate):
//! - `Section` = a pipeline job (`▶ install`); `Subsection` = a backend group.
//! - Long work announces itself live (`UnitStarted`, `Command`, `Elevate`)
//!   while its stdout/stderr accumulates as `UnitLog` lines, flushed as one
//!   grouped block on `UnitFinished`. Timing stays live, reading stays grouped.

use std::fmt;
use std::sync::{Arc, Mutex};

/// Which stream a [`Event::UnitLog`] line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// One user-visible fact about what is happening (or happened).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A pipeline job header (`▶ install`).
    Section { title: String },
    /// A backend group inside a job (`brew`, `cask`, …).
    Subsection { title: String },
    /// A schedulable unit started executing (`→ brew-formula:git`).
    UnitStarted { id: String },
    /// One captured stdout/stderr line of unit `id` (buffered by the
    /// renderer, flushed as a block on `UnitFinished`).
    UnitLog {
        id: String,
        stream: Stream,
        line: String,
    },
    /// A unit finished (`✓`/`✗ id (detail)`); `detail` is a one-line
    /// summary (`already ok`, `changed`, or the failure reason).
    UnitFinished {
        id: String,
        ok: bool,
        detail: String,
    },
    /// An external command about to run (`$ brew install git`).
    /// Rendered live when no unit context applies (`unit: None`), buffered
    /// into the unit's block otherwise.
    Command {
        argv: String,
        dry_run: bool,
        unit: Option<String>,
    },
    /// A command is about to run elevated. Emitted on **every** elevated
    /// spawn — even when sudo's timestamp cache means no password prompt
    /// appears — so the user stays conscious of sudo usage.
    Elevate { command: String, reason: String },
    /// An informational line (stdout).
    Note { msg: String },
    /// A non-fatal problem (stderr).
    Warn { msg: String },
}

/// Sink for [`Event`]s. Implementations must be cheap, non-blocking and
/// thread-safe: events arrive from parallel scheduler workers.
pub trait Reporter: Send + Sync + fmt::Debug {
    fn report(&self, event: Event);
}

/// The default reporter: drops every event. Used when no renderer is
/// installed (library unit tests, quiet contexts).
#[derive(Debug, Default)]
pub struct NoopReporter;

impl Reporter for NoopReporter {
    fn report(&self, _event: Event) {}
}

/// A test/debugging reporter that records every event in order.
#[derive(Debug, Default)]
pub struct RecordingReporter {
    events: Mutex<Vec<Event>>,
}

impl RecordingReporter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }

    pub fn into_events(self) -> Vec<Event> {
        self.events.into_inner().unwrap()
    }
}

impl Reporter for RecordingReporter {
    fn report(&self, event: Event) {
        self.events.lock().unwrap().push(event);
    }
}

impl Reporter for Arc<RecordingReporter> {
    fn report(&self, event: Event) {
        self.events.lock().unwrap().push(event);
    }
}

/// Split `sudo`'s own flags off `args` and return the elevated command as a
/// display string (`defaults write …`), or `None` when `program` is not sudo
/// or carries no inner command (bare `sudo -v` → `Some("-v")`).
pub fn elevated_command(program: &str, args: &[&str]) -> Option<String> {
    if program != "sudo" {
        return None;
    }
    // sudo flags that consume the following argv word.
    const TAKES_VALUE: &[&str] = &[
        "-p",
        "--prompt",
        "-u",
        "--user",
        "-g",
        "--group",
        "-C",
        "--close-from",
        "-r",
        "--role",
        "-t",
        "--type",
        "-U",
        "--other-user",
        "-D",
        "--chdir",
    ];
    let mut rest = args;
    while let Some((head, tail)) = rest.split_first() {
        if *head == "--" {
            rest = tail;
            break;
        }
        if !head.starts_with('-') || *head == "-" {
            break;
        }
        if head.contains('=') || !TAKES_VALUE.contains(head) {
            // Combined short flags (`-AnS`) or unknown long flags: assume
            // boolean; a value-taking flag in combined form is vanishingly
            // rare for our call sites.
            rest = tail;
        } else {
            rest = tail.get(1..).unwrap_or(&[]);
        }
    }
    if rest.is_empty() {
        // Credential warmups (`sudo -v`) elevate nothing yet — still worth
        // announcing, since a password prompt may appear.
        return Some("-v (credential check)".to_string());
    }
    Some(rest.join(" "))
}

/// Render `program` + `args` the way dry-run echo always has (`brew tap foo`).
pub fn display_argv(program: &str, args: &[&str]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{} {}", program, args.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_sudo_has_no_elevated_command() {
        assert_eq!(elevated_command("brew", &["install", "git"]), None);
    }

    #[test]
    fn sudo_prefix_is_stripped() {
        assert_eq!(
            elevated_command("sudo", &["defaults", "write", "com.apple.dock", "x"]),
            Some("defaults write com.apple.dock x".to_string())
        );
    }

    #[test]
    fn sudo_boolean_flags_are_skipped() {
        assert_eq!(
            elevated_command("sudo", &["-A", "-n", "installer", "-pkg", "a.pkg"]),
            Some("installer -pkg a.pkg".to_string())
        );
    }

    #[test]
    fn sudo_value_flags_consume_next_word() {
        assert_eq!(
            elevated_command("sudo", &["-p", "Password:", "-u", "root", "id"]),
            Some("id".to_string())
        );
        assert_eq!(
            elevated_command("sudo", &["--prompt=Password:", "id"]),
            Some("id".to_string())
        );
    }

    #[test]
    fn bare_sudo_still_announces() {
        assert_eq!(
            elevated_command("sudo", &["-v"]),
            Some("-v (credential check)".to_string())
        );
        assert_eq!(
            elevated_command("sudo", &[]),
            Some("-v (credential check)".to_string())
        );
    }

    #[test]
    fn double_dash_ends_flag_parsing() {
        assert_eq!(
            elevated_command("sudo", &["--", "-weird", "cmd"]),
            Some("-weird cmd".to_string())
        );
    }

    #[test]
    fn display_argv_matches_legacy_dry_run_format() {
        assert_eq!(display_argv("brew", &["tap", "foo"]), "brew tap foo");
        assert_eq!(display_argv("false", &[]), "false");
    }

    #[test]
    fn recording_reporter_preserves_order() {
        let r = RecordingReporter::new();
        r.report(Event::Note { msg: "a".into() });
        r.report(Event::Note { msg: "b".into() });
        let events = r.into_events();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], Event::Note { .. }));
    }
}
