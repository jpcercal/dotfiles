//! Interactive docker-pull-style progress UI (ratatui, inline viewport).
//!
//! [`TuiReporter`] implements the exec [`Reporter`](dotfiles_exec::Reporter):
//! scheduler threads push [`Event`](dotfiles_exec::Event)s into a channel and
//! a single driver thread owns rendering. Mid-run the region is **strictly
//! output-only** — no raw mode, no mouse capture, no stdin reads — so
//! interactive children (`sudo` password prompts, confirmations) and the
//! user's Ctrl-C/selection work on a completely normal terminal. Interactive
//! click-to-expand review happens only after the run completes (see
//! `review.rs`-style flow inside [`Driver::review_failed`]), when no child
//! can contend for the tty.
//!
//! Lifecycle: the region activates lazily on the first unit event, tears
//! down on the next `Section` (settled rows + aggregates print as plain
//! scrollback), and at process end prints its final settle lines. Shutdown
//! is synchronous: `finish()` disconnects the channel and joins the driver.

pub mod model;
pub mod render;
pub mod review;

use review::TerminalGuard;

use dotfiles_exec::{Event, Reporter};
use model::{Model, SettleLine};
use render::{draw_frame, UiState};
use std::io::{stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Redraw cadence while the region is active (~30 fps; spinner stays live).
const FRAME_BUDGET: Duration = Duration::from_millis(33);

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
    term: Option<Term>,
    shutdown: bool,
    last_draw: Option<Instant>,
}

type Term = ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>;

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
            if self.term.is_some() {
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
                // Job boundary: settle this job's region (and its plain
                // passthrough rows) into scrollback, then start fresh.
                let settle = self.model.settle_lines();
                self.print_settled(&settle);
                self.deactivate();
                println!("▶ {title}");
                self.finish_job(title);
                let _ = stdout().flush();
            }
            Event::Subsection { title } if self.term.is_none() => println!("  {title}"),
            Event::Note { msg } if self.term.is_none() => println!("{msg}"),
            Event::Warn { msg } if self.term.is_none() => eprintln!("{msg}"),
            Event::Elevate { command, reason } => {
                // Elevation notices are always plain stderr (never inside
                // the region): sudo may be about to prompt on the tty.
                eprintln!("⚠ sudo: {command}");
                eprintln!("  reason: {reason}");
                self.model.apply(Event::Elevate { command, reason });
                let _ = std::io::stderr().flush();
            }
            Event::Subsection { .. } | Event::Note { .. } | Event::Warn { .. } => {
                // Inside an active region: non-unit messages fold into the
                // model's feed so they render within the frame.
                self.model.apply(e);
            }
            unit_event => {
                // First unit activity: engage the inline region.
                if self.term.is_none() {
                    self.activate();
                }
                self.model.apply(unit_event);
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

    fn activate(&mut self) {
        let (_, h) = crossterm::terminal::size().unwrap_or((80, 24));
        if h < 10 {
            // Degenerate height: stay in plain passthrough.
            return;
        }
        let height = (h / 2).clamp(8, 18);
        // Output-only inline viewport: no raw mode, no mouse capture.
        let backend = ratatui::backend::CrosstermBackend::new(stdout());
        match ratatui::Terminal::with_options(
            backend,
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Inline(height),
            },
        ) {
            Ok(term) => {
                self.term = Some(term);
                self.ui = UiState::new();
            }
            Err(_) => {
                // Could not reserve the region: keep plain passthrough.
                self.term = None;
            }
        }
    }

    /// Drop the region. Safe to call any number of times; terminal modes
    /// were never modified while active.
    fn deactivate(&mut self) {
        self.term = None;
    }

    /// Last frame, settled summary below it.
    fn teardown(&mut self) {
        if self.term.is_some() {
            self.maybe_draw();
            self.deactivate();
        }
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
        self.print_settled(&all);
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
            .any(|m| m.rows.values().any(|r| r.state == model::RowState::Failed))
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
        let Some(term) = self.term.as_mut() else {
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
        // Auto-follow: keep the newest activity visible.
        self.ui.scroll = usize::MAX;
        let model = &self.model;
        let ui = &mut self.ui;
        let _ = term.draw(|f| draw_frame(f, model, ui));
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

impl Drop for Driver {
    fn drop(&mut self) {
        // Panic-safe region teardown: drop order guarantees the terminal
        // viewport is released even if the loop unwinds.
        self.deactivate();
    }
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
                    term: None,
                    shutdown: false,
                    last_draw: None,
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
        let _ = crossterm::execute!(
            stdout(),
            crossterm::event::DisableMouseCapture,
            crossterm::terminal::LeaveAlternateScreen
        );
        let _ = crossterm::terminal::disable_raw_mode();
    }
}
