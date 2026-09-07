//! Post-run interactive reviewer (alternate screen, click-to-expand).
//!
//! Only reachable **after the whole run completed** — no child processes
//! exist, so raw mode, mouse capture and stdin reads are safe here. The
//! reviewer presents every `Changed`/`Failed` unit row (failed first),
//! collapsed by default; details unfold inline on click or Enter.
//! `q`/`Esc`/`Enter`/`Ctrl-C` exits; [`TerminalGuard`] restores modes even
//! on unwind.
//!
//! Rendering goes through the same diffed ANSI painter as the mid-run
//! region (full-screen), so no terminal library is involved on this path
//! either — crossterm is used only for raw mode + event input.

use super::model::Model;
use super::render::{diff_commands, render_lines, UiState};
use std::io::{stdout, Write};
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
        let _ = write!(
            stdout(),
            "\x1b[?1049h\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h"
        );
        let _ = stdout().flush();
        g
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = write!(
            stdout(),
            "\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?1049l\x1b[?25h"
        );
        let _ = stdout().flush();
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// Run the reviewer over `model` until the user exits.
pub fn run(model: Model) {
    let Ok((w, h)) = crossterm::terminal::size() else {
        return;
    };
    if h < 5 || w < 20 {
        return;
    }
    let mut model = model;
    let mut ui = UiState::new();
    ui.interactive = true;
    ui.color = std::env::var_os("NO_COLOR").is_none();
    let mut painted: Vec<String> = vec![String::new(); h as usize];
    let draw = |model: &Model, ui: &mut UiState, painted: &mut Vec<String>, force: bool| {
        let frame = render_lines(model, ui, h as usize);
        let next: Vec<String> = frame.iter().map(|(l, _)| l.clone()).collect();
        let bytes = diff_commands(painted, &frame, force, h - 1);
        *painted = next;
        let _ = stdout().write_all(&bytes);
        let _ = stdout().flush();
    };
    draw(&model, &mut ui, &mut painted, true);
    loop {
        match crossterm::event::poll(Duration::from_millis(200)) {
            Ok(true) => {}
            _ => continue,
        }
        use crossterm::event::{
            self, Event as CEvent, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
        };
        let exit = match event::read() {
            Ok(CEvent::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                KeyCode::Char('q') | KeyCode::Esc => true,
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => true,
                KeyCode::Up => {
                    ui.selected = ui.selected.saturating_sub(1);
                    false
                }
                KeyCode::Down => {
                    let n = model.order.len();
                    if n > 0 {
                        ui.selected = (ui.selected + 1).min(n - 1);
                    }
                    false
                }
                KeyCode::Char(' ') | KeyCode::Tab => {
                    // Toggle the selected row from the keyboard.
                    if let Some(id) = model.order.get(ui.selected).cloned() {
                        model.toggle(&id);
                    }
                    false
                }
                KeyCode::PageUp => {
                    ui.scroll_by(-10, ui.row_map.len(), h as usize);
                    false
                }
                KeyCode::PageDown => {
                    ui.scroll_by(10, ui.row_map.len(), h as usize);
                    false
                }
                _ => false,
            },
            Ok(CEvent::Mouse(m)) => {
                match m.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(idx) = ui.hit_test(m.row) {
                            if let Some(id) = model.order.get(idx).cloned() {
                                model.toggle(&id);
                                ui.selected = idx;
                            }
                        }
                    }
                    MouseEventKind::ScrollUp => ui.scroll_by(-3, ui.row_map.len(), h as usize),
                    MouseEventKind::ScrollDown => ui.scroll_by(3, ui.row_map.len(), h as usize),
                    _ => {}
                }
                false
            }
            Ok(CEvent::Resize(..)) => false,
            Ok(_) => false,
            Err(_) => true,
        };
        draw(&model, &mut ui, &mut painted, false);
        if exit {
            break;
        }
    }
    // Repaint one last time so the final state is on screen before the
    // guard wipes the alternate screen; then a blank line for the shell.
    draw(&model, &mut ui, &mut painted, true);
    let _ = write!(stdout(), "\r\n");
    let _ = stdout().flush();
}
