//! Terminal renderer for the exec [`Event`](dotfiles_exec::Event) stream.
//!
//! Layout contract:
//! - `Section` → a job header (`▶ install`); `Subsection` → a backend group.
//! - Long work announces itself live (`→ unit`, `$ command`, `⚠ sudo …`)
//!   while its stdout/stderr accumulates per unit and flushes as one grouped,
//!   indented block on `UnitFinished`. Timing stays live, reading stays grouped.
//! - Everything is shown: successful and failed unit blocks alike (commands
//!   and output), failures additionally marked `✗` in red.
//! - Colors apply only when the sink supports them (`NO_COLOR`/piped output
//!   stays plain) — the glyphs (`▶ ✓ ✗ ⚠ → ! $`) carry the structure alone.

use dotfiles_exec::{Event, Reporter, Stream};
use owo_colors::{OwoColorize, Stream as ColorStream};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::sync::Mutex;

/// One buffered line inside a unit's block.
#[derive(Debug, Clone)]
enum Buffered {
    Command(String),
    Out(String),
    Err(String),
}

struct Inner {
    sink: Box<dyn Write + Send>,
    buffers: BTreeMap<String, Vec<Buffered>>,
    finished: BTreeSet<String>,
}

/// Renders [`Event`]s to a terminal (or any `Write` sink in tests).
pub struct TermReporter {
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for TermReporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TermReporter").finish_non_exhaustive()
    }
}

impl TermReporter {
    /// Report to real stdout/stderr.
    pub fn new() -> Self {
        Self::with_sink(Box::new(std::io::stdout()))
    }

    fn with_sink(sink: Box<dyn Write + Send>) -> Self {
        Self {
            inner: Mutex::new(Inner {
                sink,
                buffers: BTreeMap::new(),
                finished: BTreeSet::new(),
            }),
        }
    }

    fn emit(&self, line: String, stderr: bool) {
        let mut inner = self.inner.lock().unwrap();
        if stderr {
            let _ = writeln!(std::io::stderr(), "{line}");
        } else {
            let _ = writeln!(inner.sink, "{line}");
        }
        let _ = inner.sink.flush();
    }

    /// Flush one unit's buffered block (commands + output, in order).
    fn flush(&self, id: &str) {
        let buffered = self
            .inner
            .lock()
            .unwrap()
            .buffers
            .remove(id)
            .unwrap_or_default();
        for b in buffered {
            let line = match b {
                Buffered::Command(argv) => format!(
                    "    {}",
                    format!("$ {argv}").if_supports_color(ColorStream::Stdout, |t| t.dimmed())
                ),
                Buffered::Out(l) => format!("    {l}"),
                Buffered::Err(l) => format!(
                    "    {}",
                    format!("! {l}").if_supports_color(ColorStream::Stdout, |t| t.red())
                ),
            };
            self.emit(line, false);
        }
    }
}

impl Default for TermReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl Reporter for TermReporter {
    fn report(&self, event: Event) {
        match event {
            Event::Section { title } => {
                let line = format!("▶ {title}")
                    .if_supports_color(ColorStream::Stdout, |t| t.bold())
                    .to_string();
                self.emit(line, false);
            }
            Event::Subsection { title } => {
                let line = format!("  {title}")
                    .if_supports_color(ColorStream::Stdout, |t| t.bold())
                    .to_string();
                self.emit(line, false);
            }
            Event::UnitStarted { id } => {
                self.inner
                    .lock()
                    .unwrap()
                    .buffers
                    .entry(id.clone())
                    .or_default();
                let line = format!("→ {id}")
                    .if_supports_color(ColorStream::Stdout, |t| t.dimmed())
                    .to_string();
                self.emit(line, false);
            }
            Event::UnitLog { id, stream, line } => {
                let buffered = match stream {
                    Stream::Stdout => Buffered::Out(line),
                    Stream::Stderr => Buffered::Err(line),
                };
                let mut inner = self.inner.lock().unwrap();
                if inner.finished.contains(&id) {
                    // Defensive: lines arriving after the finish event (should
                    // not happen — output completes before the runner returns)
                    // print live rather than being dropped.
                    drop(inner);
                    self.emit(format!("  [{id}] {}", render_buffered(&buffered)), false);
                } else {
                    inner.buffers.entry(id).or_default().push(buffered);
                }
            }
            Event::Command {
                argv,
                dry_run,
                unit,
            } => {
                let text = if dry_run {
                    format!("{argv} [dry-run]")
                } else {
                    argv.clone()
                };
                match unit {
                    Some(id) => {
                        let mut inner = self.inner.lock().unwrap();
                        if inner.finished.contains(&id) {
                            drop(inner);
                            self.emit(format!("  [{id}] $ {text}"), false);
                        } else {
                            inner
                                .buffers
                                .entry(id)
                                .or_default()
                                .push(Buffered::Command(text));
                        }
                    }
                    None => self.emit(
                        format!(
                            "{}",
                            format!("$ {text}")
                                .if_supports_color(ColorStream::Stdout, |t| t.dimmed())
                        ),
                        false,
                    ),
                }
            }
            Event::UnitFinished { id, ok, detail } => {
                let mark = if ok { "✓" } else { "✗" };
                let head = if ok {
                    format!("{mark} {id} ({detail})")
                        .if_supports_color(ColorStream::Stdout, |t| t.green())
                        .to_string()
                } else {
                    format!("{mark} {id} ({detail})")
                        .if_supports_color(ColorStream::Stdout, |t| t.red())
                        .to_string()
                };
                self.emit(head, !ok);
                self.inner.lock().unwrap().finished.insert(id.clone());
                self.flush(&id);
            }
            Event::Elevate { command, reason } => {
                // Always live, even mid-parallel-run: every elevation is
                // user-visible at the moment it happens.
                let head = format!("⚠ sudo: {command}")
                    .if_supports_color(ColorStream::Stderr, |t| t.yellow())
                    .to_string();
                self.emit(head, true);
                self.emit(format!("  reason: {reason}"), true);
            }
            Event::Note { msg } => self.emit(msg, false),
            Event::Warn { msg } => {
                let line = if msg.starts_with("✗") {
                    msg.as_str()
                        .if_supports_color(ColorStream::Stderr, |t| t.red())
                        .to_string()
                } else {
                    msg
                };
                self.emit(line, true);
            }
        }
    }
}

fn render_buffered(b: &Buffered) -> String {
    match b {
        Buffered::Command(argv) => format!("$ {argv}"),
        Buffered::Out(l) => l.clone(),
        Buffered::Err(l) => format!("! {l}"),
    }
}

#[cfg(test)]
pub(crate) mod test_sink {
    use super::*;
    use std::sync::Arc;

    /// A `Write` sink backed by shared memory, for asserting on rendering.
    /// Build a reporter capturing stdout-bound lines; returns the reporter
    /// plus a reader for the captured text.
    pub fn capture() -> (TermReporter, Arc<Mutex<Vec<u8>>>) {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let writer = SharedWriter(buf.clone());
        (TermReporter::with_sink(Box::new(writer)), buf)
    }

    #[derive(Clone, Debug)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    pub fn text(buf: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(buf.lock().unwrap().clone()).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::test_sink::{capture, text};
    use super::*;

    #[test]
    fn sections_units_and_blocks_render_in_order() {
        let (r, buf) = capture();
        r.report(Event::Section {
            title: "install".into(),
        });
        r.report(Event::UnitStarted {
            id: "brew-formula:git".into(),
        });
        r.report(Event::Command {
            argv: "brew install git".into(),
            dry_run: false,
            unit: Some("brew-formula:git".into()),
        });
        // Nothing but the start line is visible before the finish …
        assert_eq!(text(&buf), "▶ install\n→ brew-formula:git\n");
        r.report(Event::UnitLog {
            id: "brew-formula:git".into(),
            stream: Stream::Stdout,
            line: "already installed".into(),
        });
        r.report(Event::UnitFinished {
            id: "brew-formula:git".into(),
            ok: true,
            detail: "already ok".into(),
        });
        assert_eq!(
            text(&buf),
            "▶ install\n\
             → brew-formula:git\n\
             ✓ brew-formula:git (already ok)\n\
             \x20   $ brew install git\n\
             \x20   already installed\n"
        );
    }

    #[test]
    fn stderr_lines_get_bang_markers_and_failures_go_red() {
        let (r, buf) = capture();
        r.report(Event::UnitStarted { id: "u".into() });
        r.report(Event::UnitLog {
            id: "u".into(),
            stream: Stream::Stderr,
            line: "boom".into(),
        });
        r.report(Event::UnitFinished {
            id: "u".into(),
            ok: false,
            detail: "boom".into(),
        });
        // Captured sink is not a tty → plain glyphs, no ANSI escapes. The
        // `✗ u (boom)` head goes to real stderr (always user-visible); the
        // sink keeps the start line plus the flushed block.
        let out = text(&buf);
        assert!(out.contains("→ u"), "{out}");
        assert!(out.contains("    ! boom"), "{out}");
        assert!(!out.contains("✗"), "{out}");
        assert!(!out.contains('\x1b'), "{out:?}");
    }

    #[test]
    fn elevation_and_unscoped_commands_print_live() {
        let (r, buf) = capture();
        r.report(Event::Elevate {
            command: "sudo defaults write NSGlobalDomain x".into(),
            reason: "test".into(),
        });
        // Elevations go to real stderr (always user-visible) …
        assert_eq!(text(&buf), "");
        r.report(Event::Command {
            argv: "brew tap foo".into(),
            dry_run: false,
            unit: None,
        });
        r.report(Event::Note {
            msg: "plain note".into(),
        });
        let out = text(&buf);
        // … while unscoped commands and notes hit the captured sink live.
        assert!(out.contains("$ brew tap foo"), "{out}");
        assert!(out.contains("plain note"), "{out}");
    }

    #[test]
    fn parallel_units_keep_separate_blocks() {
        let (r, buf) = capture();
        for id in ["a", "b"] {
            r.report(Event::UnitStarted { id: id.into() });
        }
        // Interleaved arrival …
        r.report(Event::UnitLog {
            id: "a".into(),
            stream: Stream::Stdout,
            line: "a1".into(),
        });
        r.report(Event::UnitLog {
            id: "b".into(),
            stream: Stream::Stdout,
            line: "b1".into(),
        });
        r.report(Event::UnitFinished {
            id: "a".into(),
            ok: true,
            detail: "changed".into(),
        });
        r.report(Event::UnitLog {
            id: "b".into(),
            stream: Stream::Stdout,
            line: "b2".into(),
        });
        r.report(Event::UnitFinished {
            id: "b".into(),
            ok: true,
            detail: "changed".into(),
        });
        let out = text(&buf);
        let pa = out.find("✓ a (changed)").unwrap();
        let pb = out.find("✓ b (changed)").unwrap();
        // … grouped per unit at flush time, never interleaved.
        assert!(out[pa..pb].contains("    a1") && !out[pa..pb].contains("b1"));
        assert!(out[pb..].contains("    b1") && out[pb..].contains("    b2"));
    }
}
