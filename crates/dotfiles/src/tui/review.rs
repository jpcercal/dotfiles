//! Post-run interactive reviewer (alternate screen, click-to-expand).
//!
//! Only reachable **after the whole run completed** — no child processes
//! exist, so raw mode, mouse capture and stdin reads are safe here. The
//! reviewer presents every `Changed`/`Failed` unit row (failed first),
//! collapsed by default; details unfold inline on click or Enter.
//! `q`/`Esc`/`Enter`/`Ctrl-C` exits; [`TerminalGuard`] restores modes even
//! on unwind.

use super::model::Model;
use super::render::{draw_frame, UiState};
use std::io::stdout;
use std::time::Duration;

/// RAII: whatever happens (panic, early return, wrapper error), the
/// terminal comes back to a normal state.
pub struct TerminalGuard;

impl TerminalGuard {
    /// Enter raw mode + alternate screen + mouse capture. If any step
    /// fails, the previous steps are already rolled back on drop.
    pub fn enter() -> Self {
        let g = TerminalGuard;
        if crossterm::terminal::enable_raw_mode().is_err() {
            return g;
        }
        let _ = crossterm::execute!(
            stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::event::EnableMouseCapture
        );
        g
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = crossterm::execute!(
            stdout(),
            crossterm::event::DisableMouseCapture,
            crossterm::terminal::LeaveAlternateScreen
        );
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// Run the reviewer over `model` until the user exits.
pub fn run(model: Model) {
    let backend = ratatui::backend::CrosstermBackend::new(stdout());
    let mut term = match ratatui::Terminal::new(backend) {
        Ok(t) => t,
        Err(_) => return,
    };
    let mut model = model;
    let mut ui = UiState::new();
    ui.interactive = true;
    loop {
        // Draw first so a resize glitch never eats the initial frame.
        let r = term.draw(|f| draw_frame(f, &model, &mut ui));
        if r.is_err() {
            break;
        }
        match crossterm::event::poll(Duration::from_millis(200)) {
            Ok(true) => {}
            _ => continue,
        }
        use crossterm::event::{
            self, Event as CEvent, KeyCode, KeyEventKind, MouseButton, MouseEventKind,
        };
        let exit = match event::read() {
            Ok(CEvent::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => true,
                KeyCode::Char('c') if k.modifiers.contains(event::KeyModifiers::CONTROL) => true,
                KeyCode::Up => {
                    ui.selected = ui.selected.saturating_sub(1);
                    false
                }
                KeyCode::Char(' ') | KeyCode::Tab => {
                    // Toggle the selected row from the keyboard.
                    if let Some(id) = model.order.get(ui.selected).cloned() {
                        model.toggle(&id);
                    }
                    false
                }
                KeyCode::Down => {
                    let n = model.order.len();
                    if n > 0 {
                        ui.selected = (ui.selected + 1).min(n - 1);
                    }
                    false
                }
                KeyCode::PageUp => {
                    ui.scroll_by(-10, ui.row_map.len());
                    false
                }
                KeyCode::PageDown => {
                    ui.scroll_by(10, ui.row_map.len());
                    false
                }
                _ => false,
            },
            Ok(CEvent::Mouse(m)) => {
                match m.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(idx) = ui.hit_test(m.column, m.row) {
                            if let Some(id) = model.order.get(idx).cloned() {
                                model.toggle(&id);
                                ui.selected = idx;
                            }
                        }
                    }
                    MouseEventKind::ScrollUp => ui.scroll_by(-3, ui.row_map.len()),
                    MouseEventKind::ScrollDown => ui.scroll_by(3, ui.row_map.len()),
                    _ => {}
                }
                false
            }
            Ok(CEvent::Resize(..)) => false,
            Ok(_) => false,
            Err(_) => true,
        };
        if exit {
            break;
        }
    }
}
