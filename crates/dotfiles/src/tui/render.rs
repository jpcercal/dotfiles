//! ratatui rendering for the progress [`Model`](super::model::Model).
//!
//! Docker-pull layout inside an inline viewport: a header, one row per
//! visible unit (in-flight rows show the live `$ command`), inline-expanded
//! detail blocks, an aggregate line for hidden no-op units, and a footer
//! with the last elevation notice. Pure `draw_frame` + [`UiState`] hit-test
//! mapping keep this unit-testable via `TestBackend`.

use super::model::{Model, RowState};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Per-visual-line hit target, rebuilt on every frame.
#[derive(Debug, Clone, Copy)]
pub struct RowMapEntry {
    /// Index into [`Model::order`], if this line toggles a unit's block.
    pub order_idx: Option<usize>,
}

#[derive(Debug)]
pub struct UiState {
    /// First visible visual line (wheel/keys).
    pub scroll: usize,
    /// Selected row for keyboard toggle.
    pub selected: usize,
    pub row_map: Vec<RowMapEntry>,
    pub viewport_y: u16,
    pub viewport_h: u16,
    pub frame: u64,
    pub color: bool,
}

impl UiState {
    pub fn new() -> Self {
        Self {
            scroll: 0,
            selected: 0,
            row_map: vec![],
            viewport_y: 0,
            viewport_h: 0,
            frame: 0,
            color: std::env::var_os("NO_COLOR").is_none(),
        }
    }

    /// Map a terminal click (global coords) to a unit-row order index.
    pub fn hit_test(&self, _col: u16, row: u16) -> Option<usize> {
        let visual = row.checked_sub(self.viewport_y)? as usize + self.scroll;
        self.row_map.get(visual)?.order_idx
    }

    pub fn scroll_by(&mut self, delta: isize, total_lines: usize) {
        let max = total_lines.saturating_sub(self.viewport_h.max(1) as usize);
        let next = self.scroll as isize + delta;
        self.scroll = next.clamp(0, max as isize) as usize;
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

fn push_line<'a>(
    line: Line<'a>,
    order_idx: Option<usize>,
    lines: &mut Vec<Line<'a>>,
    row_map: &mut Vec<RowMapEntry>,
) {
    lines.push(line);
    row_map.push(RowMapEntry { order_idx });
}

fn styled<'a>(text: impl Into<std::borrow::Cow<'a, str>>, style: Style, color: bool) -> Span<'a> {
    if color {
        Span::styled(text, style)
    } else {
        Span::raw(text)
    }
}

/// Render one frame; advances the spinner (redraws are event-driven, so the
/// spinner moves on activity and freezes when idle — never fighting a
/// `sudo` password prompt).
pub fn draw_frame(frame: &mut Frame, model: &Model, ui: &mut UiState) {
    let area = frame.area();
    ui.viewport_y = area.y;
    ui.viewport_h = area.height;
    ui.frame = ui.frame.wrapping_add(1);

    let color = ui.color;
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);
    let green = Style::default().fg(Color::Green);
    let red = Style::default().fg(Color::Red);
    let yellow = Style::default().fg(Color::Yellow);

    let in_flight = model
        .rows
        .values()
        .filter(|r| r.state == RowState::InFlight)
        .count();
    let spin = SPINNER[(ui.frame as usize) % SPINNER.len()];

    let mut lines: Vec<Line> = vec![];
    let mut row_map: Vec<RowMapEntry> = vec![];

    // Header: `▶ install   87/181` (+ spinner while anything runs).
    let mut head = vec![
        styled("▶ ", bold, color),
        styled(model.section.clone(), bold, color),
    ];
    head.push(Span::raw(format!(
        "   {}/{} done",
        model.finished, model.started
    )));
    if in_flight > 0 {
        head.push(Span::raw(format!("  {spin} {in_flight} running")));
    }
    push_line(Line::from(head), None, &mut lines, &mut row_map);

    // Unit rows (+ inline-expanded blocks).
    for (idx, id) in model.order.iter().enumerate() {
        let Some(row) = model.rows.get(id) else {
            continue;
        };
        let caret = if row.expanded { "▾" } else { "▸" };
        let selected = idx == ui.selected;
        let sel = if selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        let row_line = match row.state {
            RowState::InFlight => {
                let cmd = row.current_cmd.as_deref().unwrap_or("…");
                Line::from(vec![
                    styled(spin, dim, color),
                    Span::raw(" "),
                    styled(caret, dim, color),
                    Span::raw(format!(" {}  $ {cmd}", row.id)),
                ])
            }
            RowState::Changed => Line::from(vec![
                styled("✓", green, color),
                Span::raw(" "),
                styled(caret, dim, color),
                Span::raw(format!(" {} ({})", row.id, row.detail)),
            ]),
            RowState::Failed => Line::from(vec![
                styled("✗", red, color),
                Span::raw(" "),
                styled(caret, dim, color),
                Span::raw(format!(" {} ({})", row.id, row.detail)),
            ]),
        };
        let row_line = if selected {
            row_line.patch_style(sel)
        } else {
            row_line
        };
        push_line(row_line, Some(idx), &mut lines, &mut row_map);
        if row.expanded {
            for b in &row.block {
                let text = if b.command {
                    format!("      $ {}", b.text)
                } else if b.stderr {
                    format!("      ! {}", b.text)
                } else {
                    format!("      {}", b.text)
                };
                let style = if b.command {
                    dim
                } else if b.stderr {
                    red
                } else {
                    Style::default()
                };
                push_line(
                    Line::from(vec![styled(text, style, color)]),
                    Some(idx),
                    &mut lines,
                    &mut row_map,
                );
            }
        }
    }

    // Aggregate line for hidden no-op units.
    if !model.aggregates.is_empty() {
        let mut parts: Vec<(&String, &usize)> = model.aggregates.iter().collect();
        parts.sort();
        let text = parts
            .into_iter()
            .map(|(d, n)| format!("{d}: {n} already installed"))
            .collect::<Vec<_>>()
            .join(" · ");
        push_line(
            Line::from(vec![styled(format!("  · {text}"), dim, color)]),
            None,
            &mut lines,
            &mut row_map,
        );
    }

    // Recent feed (unscoped commands, notes, recap lines).
    for feed in &model.recent {
        let style = if feed.stderr { red } else { dim };
        push_line(
            Line::from(vec![styled(format!("  {}", feed.text), style, color)]),
            None,
            &mut lines,
            &mut row_map,
        );
    }

    // Footer: last elevation (sudo consciousness) + hints.
    if let Some((cmd, reason)) = &model.last_elevate {
        push_line(
            Line::from(vec![
                styled(format!("  ⚠ sudo: {cmd}"), yellow, color),
                styled(format!(" — {reason}"), dim, color),
            ]),
            None,
            &mut lines,
            &mut row_map,
        );
    }
    push_line(
        Line::from(vec![styled(
            "  click/Enter expand · ↑↓ select · wheel scroll",
            dim,
            color,
        )]),
        None,
        &mut lines,
        &mut row_map,
    );

    // Clamp scroll into the visible window, then draw scrolled.
    let visible = (area.height as usize).max(1);
    let max_scroll = lines.len().saturating_sub(visible);
    ui.scroll = ui.scroll.min(max_scroll);
    if ui.selected >= model.order.len() {
        ui.selected = model.order.len().saturating_sub(1);
    }
    ui.row_map = row_map;
    let para = Paragraph::new(Text::from(lines)).scroll((ui.scroll as u16, 0));
    frame.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::super::model::Model;
    use super::*;
    use dotfiles_exec::{Event, Stream, UnitOutcome};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn model_with_rows() -> Model {
        let mut m = Model::new();
        m.apply(Event::Section {
            title: "install".into(),
        });
        m.apply(Event::UnitStarted {
            id: "brew-formula:git".into(),
        });
        m.apply(Event::Command {
            argv: "brew install --formula git".into(),
            dry_run: false,
            unit: Some("brew-formula:git".into()),
        });
        m.apply(Event::UnitLog {
            id: "brew-formula:git".into(),
            stream: Stream::Stderr,
            line: "pouring git".into(),
        });
        m.apply(Event::UnitFinished {
            id: "brew-formula:git".into(),
            ok: true,
            detail: "changed".into(),
            outcome: UnitOutcome::Changed,
        });
        m.apply(Event::UnitStarted {
            id: "brew-formula:fd".into(),
        });
        m.apply(Event::UnitFinished {
            id: "brew-formula:fd".into(),
            ok: true,
            detail: "already ok".into(),
            outcome: UnitOutcome::NoOp,
        });
        m.apply(Event::UnitStarted {
            id: "mas:123".into(),
        });
        m.apply(Event::UnitFinished {
            id: "mas:123".into(),
            ok: false,
            detail: "exit 1".into(),
            outcome: UnitOutcome::Failed,
        });
        m
    }

    fn render_text(m: &Model, ui: &mut UiState, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| draw_frame(f, m, ui)).unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..h {
            let mut row = String::new();
            for x in 0..w {
                row.push_str(buf[(x, y)].symbol());
            }
            out.push_str(row.trim_end());
            out.push('\n');
        }
        out
    }

    #[test]
    fn settled_rows_show_but_noop_rows_vanish() {
        let m = model_with_rows();
        let mut ui = UiState::new();
        let out = render_text(&m, &mut ui, 70, 20);
        // In-flight command, settled changed + failed rows are visible …
        assert!(
            out.contains("$ brew install --formula git") || out.contains("brew-formula:git"),
            "{out}"
        );
        assert!(out.contains("✓"), "{out}");
        assert!(out.contains("✗") && out.contains("mas:123"), "{out}");
        // … but the no-op row is gone, counted only in the aggregate …
        assert!(!out.contains("brew-formula:fd"), "{out}");
        assert!(out.contains("brew-formula: 1 already installed"), "{out}");
        // … and collapsed blocks stay hidden until expanded.
        assert!(!out.contains("pouring git"), "{out}");
    }

    #[test]
    fn expansion_reveals_block_lines_inline() {
        let mut m = model_with_rows();
        m.toggle("brew-formula:git");
        m.toggle("mas:123");
        let mut ui = UiState::new();
        let out = render_text(&m, &mut ui, 70, 20);
        assert!(out.contains("$ brew install --formula git"), "{out}");
        assert!(out.contains("! pouring git"), "{out}");
    }

    #[test]
    fn hit_test_maps_clicks_to_rows() {
        let m = model_with_rows();
        let mut ui = UiState::new();
        let _ = render_text(&m, &mut ui, 70, 20);
        // Header is visual line 0; first row (git) is line 1.
        assert_eq!(ui.hit_test(5, ui.viewport_y + 1), Some(0));
        // Second visible row is mas (fd vanished into the aggregate).
        assert_eq!(ui.hit_test(5, ui.viewport_y + 2), Some(1));
        // Header line toggles nothing.
        assert_eq!(ui.hit_test(5, ui.viewport_y), None);
    }

    #[test]
    fn footer_shows_elevation_and_hints() {
        let mut m = Model::new();
        m.apply(Event::Section {
            title: "install".into(),
        });
        m.apply(Event::Elevate {
            command: "sudo -v".into(),
            reason: "cask warmup".into(),
        });
        let mut ui = UiState::new();
        let out = render_text(&m, &mut ui, 70, 20);
        assert!(out.contains("⚠ sudo: sudo -v"), "{out}");
        assert!(out.contains("click/Enter expand"), "{out}");
    }
}
