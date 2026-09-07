//! Interactive docker-pull-style progress UI (top-anchored ANSI region).
//!
//! [`TuiReporter`] implements the exec [`Reporter`](dotfiles_exec::Reporter):
//! scheduler threads push [`Event`](dotfiles_exec::Event)s into a channel and
//! a single driver thread owns rendering. The renderer is a **hand-rolled
//! diffed ANSI painter** — no terminal library in the hot path, and
//! therefore no cursor-position queries (`[6n` DSR), no raw-mode toggling
//! and no stdin reads of any kind mid-run. Those queries are exactly how
//! terminal UIs hijack the tty: they race `sudo` password prompts for input
//! bytes and hang forever on ptys that never answer.
//!
//! The region owns screen rows `0..h-1`; the LAST row stays free and the
//! cursor is parked there after every frame, so a child's interactive
//! prompt (sudo from mas/cask installers or hook snippets — announced or
//! not) lands on that line, fully visible and typeable, never overwritten
//! by a redraw. A one-second forced repaint heals any scroll the password
//! Entry causes.
//!
//! Engaging the region scrolls a full screen (prior output is preserved in
//! the terminal's scrollback), so the region never floats detached from the
//! output. Interactive click-to-expand review happens only after the run
//! completes ([`Driver::review_failed`]), when no child can contend for the
//! tty. Shutdown is synchronous: `finish()` disconnects the channel and
//! joins the driver.

pub mod model;
pub mod render;
pub mod review;

use review::TerminalGuard;

use dotfiles_exec::{Event, Reporter};
use model::{Model, RowState, SettleLine};
use render::{diff_commands, render_lines, UiState};
use std::io::{stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Redraw cadence while the region is active (~30 fps; spinner stays live).
const FRAME_BUDGET: Duration = Duration::from_millis(33);
/// Forced full repaint cadence: heals garbling caused by external tty
/// writes (prompt lines, password-Entry scroll) within a second.
const REPAIR_EVERY: Duration = Duration::from_secs(1);

enum DriverMsg {
    Event(Event),
    /// Reserved for explicit-shutdown signaling (channel disconnect is the
    /// shutdown path today; keep the variant for symmetry).
    #[allow(dead_code)]
    Shutdown,
}

struct Driver {
    rx: mpsc::Receiver<DriverMsg>,
    /// Current job model.
    model: Model,
    /// Settled models of past jobs (kept for the post-run reviewer).
    jobs: Vec<Model>,
    ui: UiState,
    /// Last painted rows (shadow buffer for the diff painter).
    painted: Vec<String>,
    /// Screen geometry while the region is active (`None` = idle).
    region: Option<(u16, u16)>,
    /// Set when the region cannot engage (degenerate size): unit finishes
    /// then print plainly, TermReporter-style.
    fallback: bool,
    shutdown: bool,
    last_draw: Option<Instant>,
    last_repaint: Option<Instant>,
}

impl Driver {
    fn run(mut self) {
        // Events drive redraws; the timeout keeps the spinner advancing
        // while units are in flight. No stdin polling anywhere: the tty
        // belongs to interactive children and the user, always.
        loop {
            match self.rx.recv_timeout(FRAME_BUDGET) {
                Ok(DriverMsg::Event(e)) => self.on_event(e),
                Ok(DriverMsg::Shutdown) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            // Drain everything queued before drawing (no starvation),
            // then draw at most one frame.
            loop {
                match self.rx.try_recv() {
                    Ok(DriverMsg::Event(e)) => self.on_event(e),
                    Ok(DriverMsg::Shutdown) => {
                        self.shutdown = true;
                        break;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => break,
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            if self.shutdown {
                break;
            }
            if self.region.is_some() {
                self.maybe_draw();
            }
        }
        self.teardown();
        if self.review_failed() {
            // Failed units across any job: merged reviewer model.
            let merged = Model::merged_for_review(&self.all_models());
            let _guard = TerminalGuard::enter();
            review::run(merged);
            // `_guard` restores raw mode / alt screen / mouse on drop.
        }
    }

    fn on_event(&mut self, e: Event) {
        match e {
            Event::Section { title } => {
                // Job boundary: drop the region, settle its content into
                // plain scrollback, then start the next job fresh.
                let settle = self.model.settle_lines();
                let mut feed: Vec<SettleLine> = self
                    .model
                    .recent
                    .iter()
                    .map(|f| SettleLine {
                        stderr: f.stderr,
                        text: f.text.clone(),
                    })
                    .collect();
                let print_settle = self.region.is_some() || !self.model.rows.is_empty();
                self.disengage();
                if print_settle {
                    let mut all = settle;
                    all.append(&mut feed);
                    self.print_settled(&all);
                }
                println!("▶ {title}");
                self.finish_job(title);
            }
            // Idle passthrough: identical to the plain reporter, and none of
            // these engage the region (preflight probes, prompts, section
            // markers stay normal terminal output).
            Event::Subsection { title } if self.region.is_none() => println!("  {title}"),
            Event::Note { msg } if self.region.is_none() => println!("{msg}"),
            Event::Warn { msg } if self.region.is_none() => eprintln!("{msg}"),
            Event::Elevate { command, reason } if self.region.is_none() => {
                eprintln!("⚠ sudo: {command}");
                eprintln!("  reason: {reason}");
                let _ = std::io::stderr().flush();
                self.model.apply(Event::Elevate { command, reason });
            }
            Event::Command {
                argv,
                dry_run,
                unit: None,
            } if self.region.is_none() => {
                // Unscoped commands (probes, warmups) echo plainly and never
                // engage the region.
                if dry_run {
                    println!("$ {argv} [dry-run]");
                } else {
                    println!("$ {argv}");
                }
            }
            Event::CommandDone { .. } => {
                // Announcement bookkeeping (balanced with Command); nothing
                // to render.
            }
            other => {
                // Unit-context events (and in-frame non-unit events). Unit
                // events engage the region on first sight.
                let unit_event = matches!(
                    other,
                    Event::UnitStarted { .. }
                        | Event::UnitLog { .. }
                        | Event::UnitFinished { .. }
                        | Event::Command { unit: Some(_), .. }
                );
                // Region impossible (degenerate size): finishes print
                // plainly so nothing is lost.
                let finished = if self.region.is_none() && self.fallback {
                    match &other {
                        Event::UnitFinished {
                            id,
                            detail,
                            outcome:
                                dotfiles_exec::UnitOutcome::Changed | dotfiles_exec::UnitOutcome::Failed,
                            ..
                        } => Some((id.clone(), detail.clone())),
                        _ => None,
                    }
                } else {
                    None
                };
                self.model.apply(other);
                if let Some((id, detail)) = finished {
                    self.print_finished_plainly(&id, &detail);
                }
                if unit_event && self.region.is_none() && !self.fallback {
                    self.engage();
                }
            }
        }
        let _ = stdout().flush();
    }

    /// Stash the completed job and start a fresh model.
    fn finish_job(&mut self, title: String) {
        let done = std::mem::take(&mut self.model);
        self.jobs.push(done);
        self.model.reset_job(title);
    }

    /// Own the screen: scroll a full screen (prior output is preserved in
    /// the terminal's scrollback, never lost), then take rows `0..h-1` as
    /// the region. The last row stays free — see the module docs.
    fn engage(&mut self) {
        let (w, h) = crossterm::terminal::size().unwrap_or((80, 24));
        if h < 10 || w < 20 {
            self.fallback = true;
            return;
        }
        print!("{}", "\n".repeat(h as usize));
        let _ = stdout().flush();
        self.region = Some((w, h));
        self.painted = vec![String::new(); (h - 1) as usize];
        self.ui = UiState::new();
        self.ui.scroll = usize::MAX; // start at the tail
        self.maybe_draw();
    }

    /// Release the region: erase it (its content was ephemeral progress)
    /// and leave the cursor at the top-left for plain output.
    fn disengage(&mut self) {
        if self.region.take().is_some() {
            self.painted.clear();
            let _ = write!(stdout(), "\x1b[1;1H\x1b[0m\x1b[2J\x1b[?25h");
            let _ = stdout().flush();
        }
    }

    /// Final settle: region content becomes plain scrollback lines.
    fn teardown(&mut self) {
        let settle = self.model.settle_lines();
        let mut feed: Vec<SettleLine> = self
            .model
            .recent
            .iter()
            .map(|f| SettleLine {
                stderr: f.stderr,
                text: f.text.clone(),
            })
            .collect();
        let mut all = settle;
        all.append(&mut feed);
        self.disengage();
        self.print_settled(&all);
    }

    /// TermReporter-style block print for the fallback path (region could
    /// not engage): the row is removed from the model after printing.
    fn print_finished_plainly(&mut self, id: &str, detail: &str) {
        let Some(row) = self.model.take_row(id) else {
            return;
        };
        let failed = row.state == RowState::Failed;
        let head = format!("{} {id} ({detail})", if failed { "✗" } else { "✓" });
        if failed {
            eprintln!("{head}");
        } else {
            println!("{head}");
        }
        for b in &row.block {
            let text = if b.command {
                format!("    $ {}", b.text)
            } else if b.stderr {
                format!("    ! {}", b.text)
            } else {
                format!("    {}", b.text)
            };
            println!("{text}");
        }
    }

    /// All job models (past jobs + the current one) for the reviewer.
    fn all_models(&self) -> Vec<&Model> {
        self.jobs
            .iter()
            .chain(std::iter::once(&self.model))
            .collect()
    }

    fn review_failed(&mut self) -> bool {
        self.all_models()
            .iter()
            .any(|m| m.rows.values().any(|r| r.state == RowState::Failed))
    }

    fn print_settled(&self, lines: &[SettleLine]) {
        for l in lines {
            if l.stderr {
                eprintln!("{}", l.text);
            } else {
                println!("{}", l.text);
            }
        }
        let _ = stdout().flush();
    }

    fn maybe_draw(&mut self) {
        let Some((w, h)) = self.region else {
            return;
        };
        let now = Instant::now();
        if self
            .last_draw
            .map(|t| now - t < FRAME_BUDGET)
            .unwrap_or(false)
        {
            return;
        }
        self.last_draw = Some(now);
        // Periodic forced repaint: heals garbling from external tty writes
        // (prompt lines, password-Entry scroll) within a second.
        let force = self
            .last_repaint
            .map(|t| now - t >= REPAIR_EVERY)
            .unwrap_or(true);
        if force {
            self.last_repaint = Some(now);
        }
        // Auto-follow: keep the newest activity visible.
        self.ui.scroll = usize::MAX;
        let height = (h - 1) as usize;
        let frame = render_lines(&self.model, &mut self.ui, height);
        let next: Vec<String> = frame.iter().map(|(l, _)| l.clone()).collect();
        let bytes = diff_commands(&self.painted, &frame, force, h - 1);
        self.painted = next;
        let _ = stdout().write_all(&bytes);
        let _ = stdout().flush();
        let _ = w;
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        // Panic-safe region teardown: drop order guarantees the region is
        // released even if the loop unwinds.
        self.disengage();
    }
}

/// Interactive progress reporter: docker-pull rows while units run, an
/// interactive reviewer afterwards when anything failed. Cheap to
/// construct: the driver thread only renders while unit events flow.
#[derive(Debug)]
pub struct TuiReporter {
    tx: Arc<Mutex<Option<mpsc::Sender<DriverMsg>>>>,
    finished: Arc<AtomicBool>,
    driver: Mutex<Option<JoinHandle<()>>>,
}

impl TuiReporter {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<DriverMsg>();
        let driver = std::thread::Builder::new()
            .name("dotfiles-tui".into())
            .spawn(move || {
                Driver {
                    rx,
                    model: Model::new(),
                    jobs: vec![],
                    ui: UiState::new(),
                    painted: vec![],
                    region: None,
                    fallback: false,
                    shutdown: false,
                    last_draw: None,
                    last_repaint: None,
                }
                .run();
            })
            .expect("spawn TUI driver thread");
        Self {
            tx: Arc::new(Mutex::new(Some(tx))),
            finished: Arc::new(AtomicBool::new(false)),
            driver: Mutex::new(Some(driver)),
        }
    }
}

impl Default for TuiReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl Reporter for TuiReporter {
    fn report(&self, event: Event) {
        if let Some(tx) = self.tx.lock().unwrap().as_ref() {
            // The driver is gone after `finish()`; late events are dropped
            // (process teardown prints nothing more anyway).
            let _ = tx.send(DriverMsg::Event(event));
        }
    }

    /// Settle the region, run the reviewer if needed, and restore the
    /// terminal — synchronously. Idempotent; also runs from the
    /// process-exit guard.
    fn finish(&self) {
        if self.finished.swap(true, Ordering::SeqCst) {
            return;
        }
        // Disconnect the channel: the driver notices immediately even while
        // blocked in `recv_timeout`, settles, reviews, and exits.
        self.tx.lock().unwrap().take();
        if let Some(handle) = self.driver.lock().unwrap().take() {
            // No tty-mode ownership mid-run means the driver cannot hang on
            // terminal reads: join is expected to complete promptly.
            let _ = handle.join();
        }
        // Belt and braces: regardless of how the driver ended, the terminal
        // is restored from the main thread too (idempotent, cheap).
        let _ = write!(
            stdout(),
            "\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?1049l\x1b[?25h"
        );
        let _ = stdout().flush();
    }
}
