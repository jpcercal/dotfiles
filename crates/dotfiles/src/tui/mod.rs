//! Interactive docker-pull-style progress UI (ratatui, inline viewport).
//!
//! [`TuiReporter`] implements the exec [`Reporter`](dotfiles_exec::Reporter):
//! scheduler threads push [`Event`](dotfiles_exec::Event)s into a channel and
//! a single driver thread owns the terminal (raw mode + mouse capture +
//! inline region), folding events into the [`Model`](model::Model) and
//! redrawing on activity only.
//!
//! Lifecycle: the region activates lazily on the first unit event and tears
//! down on the next `Section` (or at process end), printing settled summary
//! lines as plain scrollback. Between jobs — and whenever no unit is
//! running — events pass through as plain stdout/stderr, so prompts
//! (confirmations, `sudo` passwords) always meet a normal terminal.

pub mod model;
pub mod render;

use dotfiles_exec::{Event, Reporter};
use model::{Model, SettleLine};
use render::{draw_frame, UiState};
use std::io::{stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// Redraw at most this often while events stream in.
const FRAME_BUDGET: Duration = Duration::from_millis(33);
/// How long `finish()` waits for the driver to settle before giving up.
const FINISH_TIMEOUT: Duration = Duration::from_secs(2);

enum DriverMsg {
    Event(Event),
    Shutdown,
}

struct Driver {
    rx: mpsc::Receiver<DriverMsg>,
    model: Model,
    ui: UiState,
    term: Option<Term>,
    term_h: u16,
    done: Arc<AtomicBool>,
    shutdown: bool,
    last_draw: Option<Instant>,
}

type Term = ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>;

impl Driver {
    fn run(mut self) {
        loop {
            // Block briefly for input (mouse/keys/resize), then drain events.
            let mut dirty = false;
            if self.term.is_some() {
                match poll_input_timeout(FRAME_BUDGET) {
                    Some(input) => {
                        self.handle_input(input);
                        dirty = true;
                    }
                    None => dirty = self.drain_events(true) || dirty,
                }
            } else {
                // Idle: block for the next message (plain passthrough below).
                match self.rx.recv() {
                    Ok(DriverMsg::Event(e)) => {
                        self.on_idle_event(e);
                        dirty = true;
                    }
                    Ok(DriverMsg::Shutdown) | Err(_) => break,
                }
                dirty = self.drain_events(false) || dirty;
            }
            if self.should_shutdown() {
                break;
            }
            if dirty {
                self.maybe_draw();
            }
        }
        self.teardown();
        self.done.store(true, Ordering::SeqCst);
    }

    /// Non-blocking drain; returns true when the model changed.
    fn drain_events(&mut self, _active: bool) -> bool {
        let mut dirty = false;
        loop {
            match self.rx.try_recv() {
                Ok(DriverMsg::Event(e)) => {
                    dirty = true;
                    if self.term.is_some() {
                        self.on_active_event(e);
                    } else {
                        self.on_idle_event(e);
                    }
                }
                Ok(DriverMsg::Shutdown) => {
                    self.shutdown_requested();
                    return true;
                }
                Err(mpsc::TryRecvError::Empty) => return dirty,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.shutdown_requested();
                    return true;
                }
            }
        }
    }

    /// Plain passthrough while no unit region is active: notes, sections and
    /// elevations print exactly like the non-interactive reporter, so jobs
    /// without units (`prefs`, `history`) and everything before the first
    /// unit look identical in both modes.
    fn on_idle_event(&mut self, e: Event) {
        match e {
            Event::Section { title } => {
                println!("▶ {title}");
                self.model.reset_job(title);
            }
            Event::Subsection { title } => println!("  {title}"),
            Event::Note { msg } => println!("{msg}"),
            Event::Warn { msg } => eprintln!("{msg}"),
            Event::Elevate { command, reason } => {
                eprintln!("⚠ sudo: {command}");
                eprintln!("  reason: {reason}");
                self.model.apply(Event::Elevate { command, reason });
            }
            unit_event => {
                // First unit activity: engage the inline region.
                self.model.apply(unit_event);
                self.activate();
            }
        }
        let _ = stdout().flush();
    }

    fn on_active_event(&mut self, e: Event) {
        if matches!(e, Event::Section { .. }) {
            // Job boundary: settle this job's region into scrollback, drop
            // back to plain passthrough for whatever comes next.
            let settle = self.model.settle_lines();
            let title = match e {
                Event::Section { title } => title,
                _ => unreachable!(),
            };
            self.print_settled(&settle);
            self.deactivate();
            // The new section header prints plain (matches idle mode).
            println!("▶ {title}");
            self.model.reset_job(title);
            let _ = stdout().flush();
            return;
        }
        self.model.apply(e);
    }

    fn activate(&mut self) {
        if self.term.is_some() {
            return;
        }
        let (w, h) = crossterm_size().unwrap_or((80, 24));
        let _ = w;
        if h < 10 {
            // Degenerate height: stay in plain passthrough.
            return;
        }
        self.term_h = h;
        if crossterm::terminal::enable_raw_mode().is_err() {
            return;
        }
        if crossterm::execute!(stdout(), crossterm::event::EnableMouseCapture).is_err() {
            let _ = crossterm::terminal::disable_raw_mode();
            return;
        }
        let height = (h / 2).clamp(8, 18);
        let backend = ratatui::backend::CrosstermBackend::new(stdout());
        match ratatui::Terminal::with_options(
            backend,
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Inline(height),
            },
        ) {
            Ok(term) => {
                self.term = Some(term);
                self.ui.scroll = 0;
            }
            Err(_) => {
                let _ = crossterm::execute!(stdout(), crossterm::event::DisableMouseCapture);
                let _ = crossterm::terminal::disable_raw_mode();
            }
        }
    }

    fn deactivate(&mut self) {
        if self.term.is_none() {
            return;
        }
        // Final frame stays in scrollback; restore the terminal first so the
        // settled lines print to a normal tty.
        self.term = None;
        let _ = crossterm::execute!(stdout(), crossterm::event::DisableMouseCapture);
        let _ = crossterm::terminal::disable_raw_mode();
    }

    /// Last frame, settled summary below it, terminal restored.
    fn teardown(&mut self) {
        if self.term.is_some() {
            self.maybe_draw();
            let settle = self.model.settle_lines();
            // Feed leftovers (recaps like `brew [changed] …`) persist too.
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
            self.deactivate();
            self.print_settled(&all);
        }
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
        // Throttle: at most one frame per FRAME_BUDGET. Redraws are
        // event-driven anyway, so this only smooths bursts.
        let now = Instant::now();
        if self
            .last_draw
            .map(|t| now - t < FRAME_BUDGET)
            .unwrap_or(false)
        {
            return;
        }
        self.last_draw = Some(now);
        let model = &self.model;
        let ui = &mut self.ui;
        let _ = term.draw(|f| draw_frame(f, model, ui));
    }

    fn handle_input(&mut self, input: Input) {
        match input {
            Input::Click(col, row) => {
                if let Some(idx) = self.ui.hit_test(col, row) {
                    if let Some(id) = self.model.order.get(idx).cloned() {
                        self.model.toggle(&id);
                        self.ui.selected = idx;
                    }
                }
            }
            Input::Scroll(delta) => {
                let total = self.ui.row_map.len();
                self.ui.scroll_by(delta, total);
            }
            Input::Select(delta) => {
                let n = self.model.order.len();
                if n == 0 {
                    return;
                }
                let next = self.ui.selected as isize + delta;
                self.ui.selected = next.clamp(0, n as isize - 1) as usize;
            }
            Input::ToggleSelected => {
                if let Some(id) = self.model.order.get(self.ui.selected).cloned() {
                    self.model.toggle(&id);
                }
            }
            Input::CollapseAll => {
                for row in self.model.rows.values_mut() {
                    row.expanded = false;
                }
            }
            Input::Resize(h) => {
                self.term_h = h;
            }
        }
    }

    fn should_shutdown(&self) -> bool {
        self.shutdown
    }

    fn shutdown_requested(&mut self) {
        self.shutdown = true;
    }
}

enum Input {
    Click(u16, u16),
    Scroll(isize),
    Select(isize),
    ToggleSelected,
    CollapseAll,
    Resize(u16),
}

fn poll_input_timeout(timeout: Duration) -> Option<Input> {
    use crossterm::event::{
        self, Event as CEvent, KeyCode, KeyEventKind, MouseButton, MouseEventKind,
    };
    if !event::poll(timeout).ok()? {
        return None;
    }
    match event::read().ok()? {
        CEvent::Mouse(m) => match m.kind {
            MouseEventKind::Down(MouseButton::Left) => Some(Input::Click(m.column, m.row)),
            MouseEventKind::ScrollUp => Some(Input::Scroll(-3)),
            MouseEventKind::ScrollDown => Some(Input::Scroll(3)),
            _ => None,
        },
        CEvent::Key(k) if k.kind == KeyEventKind::Press => match k.code {
            KeyCode::Up => Some(Input::Select(-1)),
            KeyCode::Down => Some(Input::Select(1)),
            KeyCode::PageUp => Some(Input::Scroll(-10)),
            KeyCode::PageDown => Some(Input::Scroll(10)),
            KeyCode::Enter => Some(Input::ToggleSelected),
            KeyCode::Esc => Some(Input::CollapseAll),
            _ => None,
        },
        CEvent::Resize(_, h) => Some(Input::Resize(h)),
        _ => None,
    }
}

fn crossterm_size() -> Option<(u16, u16)> {
    crossterm::terminal::size().ok()
}

/// Interactive progress reporter: docker-pull rows with click-to-expand
/// detail blocks. Cheap to construct; the driver thread (and the terminal)
/// only exists while unit events flow.
#[derive(Debug)]
pub struct TuiReporter {
    tx: Arc<Mutex<Option<mpsc::Sender<DriverMsg>>>>,
    finished: Arc<AtomicBool>,
    driver_done: Arc<AtomicBool>,
}

impl TuiReporter {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<DriverMsg>();
        let driver_done = Arc::new(AtomicBool::new(false));
        let done_clone = driver_done.clone();
        std::thread::Builder::new()
            .name("dotfiles-tui".into())
            .spawn(move || {
                Driver {
                    rx,
                    model: Model::new(),
                    ui: UiState::new(),
                    term: None,
                    term_h: 24,
                    done: done_clone,
                    shutdown: false,
                    last_draw: None,
                }
                .run();
            })
            .expect("spawn TUI driver thread");
        Self {
            tx: Arc::new(Mutex::new(Some(tx))),
            finished: Arc::new(AtomicBool::new(false)),
            driver_done,
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

    /// Settle the region, restore the terminal, and wait (bounded) for the
    /// driver to land. Idempotent; also runs from the process-exit guard.
    fn finish(&self) {
        if self.finished.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(tx) = self.tx.lock().unwrap().take() {
            let _ = tx.send(DriverMsg::Shutdown);
        }
        let start = Instant::now();
        while !self.driver_done.load(Ordering::SeqCst) && start.elapsed() < FINISH_TIMEOUT {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
