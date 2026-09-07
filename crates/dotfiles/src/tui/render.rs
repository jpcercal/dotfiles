//! Pure text rendering for the progress [`Model`](super::model::Model).
//!
//! Docker-pull layout as plain styled strings — no terminal library, no
//! cursor queries, no mode changes. [`render_lines`] produces the visual
//! lines (ANSI-styled when `color`) plus a per-line hit-target map for
//! click routing; the driver diffs consecutive frames itself and writes
//! only changed rows (`MoveTo` + text + erase-to-EOL), so the tty is never
//! read and interactive prompts on the free last row are never disturbed.

use super::model::{Model, RowState};

pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Per-visual-line hit target: index into [`Model::order`] when the line
/// toggles a unit's detail block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowMapEntry {
    pub order_idx: Option<usize>,
}

#[derive(Debug)]
pub struct UiState {
    /// First visible visual line (wheel/keys; mid-run always tails).
    pub scroll: usize,
    /// Selected row for keyboard toggle (reviewer).
    pub selected: usize,
    pub row_map: Vec<RowMapEntry>,
    pub frame: u64,
    pub color: bool,
    /// Post-run reviewer mode: header shows the job title, footer shows
    /// input hints. Mid-run output-only regions leave this false.
    pub interactive: bool,
}

impl UiState {
    pub fn new() -> Self {
        Self {
            scroll: 0,
            selected: 0,
            row_map: vec![],
            frame: 0,
            color: std::env::var_os("NO_COLOR").is_none(),
            interactive: false,
        }
    }

    /// Map a click on visible row `row` to a unit-row order index.
    pub fn hit_test(&self, row: u16) -> Option<usize> {
        self.row_map.get(row as usize)?.order_idx
    }

    pub fn scroll_by(&mut self, delta: isize, total_lines: usize, height: usize) {
        let max = total_lines.saturating_sub(height);
        let next = self.scroll as isize + delta;
        self.scroll = next.clamp(0, max as isize) as usize;
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

fn styled(text: &str, codes: &[&str], color: bool) -> String {
    if color && !codes.is_empty() {
        format!("\x1b[{}m{}\x1b[0m", codes.join(";"), text)
    } else {
        text.to_string()
    }
}

fn wrap(text: String, codes: &[&str], color: bool) -> String {
    if color && !codes.is_empty() {
        format!("\x1b[{}m{}\x1b[0m", codes.join(";"), text)
    } else {
        text
    }
}

const BOLD: &[&str] = &["1"];
const DIM: &[&str] = &["2"];
const GREEN: &[&str] = &["32"];
const RED: &[&str] = &["31"];
const YELLOW: &[&str] = &["33"];
const REVERSED: &[&str] = &["7"];

/// Render the visible window (at most `height` lines) of the frame:
/// header, unit rows with inline-expanded detail blocks, no-op aggregates,
/// recent feed, and the footer. Returns the styled lines padded with empty
/// strings plus the hit-target map for exactly those lines.
pub fn render_lines(model: &Model, ui: &mut UiState, height: usize) -> Vec<(String, RowMapEntry)> {
    ui.frame = ui.frame.wrapping_add(1);
    let color = ui.color;
    let spin = SPINNER[(ui.frame as usize) % SPINNER.len()];

    let in_flight = model
        .rows
        .values()
        .filter(|r| r.state == RowState::InFlight)
        .count();

    let mut lines: Vec<(String, RowMapEntry)> = vec![];
    let mut push = |line: String, order_idx: Option<usize>| {
        lines.push((line, RowMapEntry { order_idx }));
    };

    // Header. The job title is NOT repeated mid-run — the plain `▶ title`
    // line already marks the section; the reviewer (interactive) shows it.
    let mut head = styled(spin, DIM, color);
    head.push_str(&format!(
        " {}/{} done · {} running",
        model.finished, model.started, in_flight
    ));
    if model.failed > 0 {
        head.push_str(&styled(
            &format!(" · ✗ {} failed", model.failed),
            RED,
            color,
        ));
    }
    if ui.interactive {
        head.push_str(&styled("  ▶ ", BOLD, color));
        head.push_str(&styled(&model.section, BOLD, color));
    }
    push(head, None);

    // Unit rows (+ inline-expanded blocks).
    for (idx, id) in model.order.iter().enumerate() {
        let Some(row) = model.rows.get(id) else {
            continue;
        };
        let caret = if row.expanded { "▾" } else { "▸" };
        let selected = idx == ui.selected && ui.interactive;
        let body = match row.state {
            RowState::InFlight => {
                let cmd = row.current_cmd.as_deref().unwrap_or("…");
                format!(
                    "{} {} {}  $ {}",
                    styled(spin, DIM, color),
                    styled(caret, DIM, color),
                    row.id,
                    cmd
                )
            }
            RowState::Changed => format!(
                "{} {} {} ({})",
                styled("✓", GREEN, color),
                styled(caret, DIM, color),
                row.id,
                row.detail
            ),
            RowState::Failed => format!(
                "{} {} {} ({})",
                styled("✗", RED, color),
                styled(caret, DIM, color),
                row.id,
                row.detail
            ),
        };
        let line = if selected {
            wrap(body, REVERSED, color)
        } else {
            body
        };
        push(line, Some(idx));
        if row.expanded {
            for b in &row.block {
                let text = if b.command {
                    format!("      $ {}", b.text)
                } else if b.stderr {
                    let t = format!("      ! {}", b.text);
                    styled(&t, RED, color)
                } else {
                    format!("      {}", b.text)
                };
                push(text, Some(idx));
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
        push(styled(&format!("  · {text}"), DIM, color), None);
    }

    // Recent feed (unscoped commands, notes, recap lines).
    for feed in &model.recent {
        let codes: &[&str] = if feed.stderr { RED } else { DIM };
        push(styled(&format!("  {}", feed.text), codes, color), None);
    }

    // Footer: prompt heads-up while risky units run, last elevation (sudo
    // consciousness), then input hints in the interactive reviewer.
    if model.may_prompt() {
        push(
            styled(
                "  ⌨ some installs may request your password — it appears on the last line",
                YELLOW,
                color,
            ),
            None,
        );
    }
    if let Some((cmd, reason)) = &model.last_elevate {
        push(styled(&format!("  ⚠ sudo: {cmd}"), YELLOW, color), None);
        push(styled(&format!("  {reason}"), DIM, color), None);
    }
    if ui.interactive {
        push(
            styled(
                "  click/Space expand · ↑↓ select · wheel scroll · q exit",
                DIM,
                color,
            ),
            None,
        );
    }

    // Clamp scroll into the visible window, then slice.
    let max_scroll = lines.len().saturating_sub(height);
    ui.scroll = ui.scroll.min(max_scroll);
    if ui.selected >= model.order.len() {
        ui.selected = model.order.len().saturating_sub(1);
    }
    let start = ui.scroll;
    let window: Vec<(String, RowMapEntry)> = lines
        .into_iter()
        .skip(start)
        .take(height)
        .chain(std::iter::repeat((
            String::new(),
            RowMapEntry { order_idx: None },
        )))
        .take(height)
        .collect();
    ui.row_map = window.iter().map(|(_, m)| *m).collect();
    window
}

/// Pure diff between the previously painted rows and the next frame:
/// ANSI commands that repaint only changed rows (and everything, when
/// `force`), then park the cursor on the free bottom row. Never reads
/// anything — the returned bytes go straight to the tty.
pub fn diff_commands(
    last: &[String],
    next: &[(String, RowMapEntry)],
    force: bool,
    prompt_row: u16,
) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, (line, _)) in next.iter().enumerate() {
        let row = i as u16 + 1; // 1-based ANSI rows
        let unchanged = !force && last.get(i).is_some_and(|prev| prev == line);
        if unchanged {
            continue;
        }
        let _ = write!(out, "\x1b[{row};1H\x1b[2K{line}");
    }
    let _ = write!(out, "\x1b[{prompt_row};1H\x1b[?25l");
    out
}

use std::io::Write as _;

#[cfg(test)]
mod tests {
    use super::super::model::Model;
    use super::*;
    use dotfiles_exec::{Event, Stream, UnitOutcome};

    fn model_with_rows() -> Model {
        let mut m = Model::new();
        m.apply(Event::Section {
            title: "install".into(),
        });
        m.apply(Event::UnitStarted {
            id: "brew-formula:git".into(),
            prompt_capable: false,
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
            prompt_capable: false,
        });
        m.apply(Event::UnitFinished {
            id: "brew-formula:fd".into(),
            ok: true,
            detail: "already ok".into(),
            outcome: UnitOutcome::NoOp,
        });
        m.apply(Event::UnitStarted {
            id: "mas:123".into(),
            prompt_capable: false,
        });
        m.apply(Event::UnitFinished {
            id: "mas:123".into(),
            ok: false,
            detail: "exit 1".into(),
            outcome: UnitOutcome::Failed,
        });
        m
    }

    fn render_plain(m: &Model, ui: &mut UiState, h: usize) -> Vec<String> {
        ui.color = false;
        render_lines(m, ui, h).into_iter().map(|(l, _)| l).collect()
    }

    #[test]
    fn settled_rows_show_but_noop_rows_vanish() {
        let m = model_with_rows();
        let mut ui = UiState::new();
        let out = render_plain(&m, &mut ui, 20);
        let text = out.join("\n");
        // Settled changed + failed rows are visible …
        assert!(text.contains("✓ ▸ brew-formula:git (changed)"), "{text}");
        assert!(text.contains("✗ ▸ mas:123 (exit 1)"), "{text}");
        // … but the no-op row is gone, counted only in the aggregate …
        assert!(!text.contains("brew-formula:fd"), "{text}");
        assert!(text.contains("brew-formula: 1 already installed"), "{text}");
        // … collapsed blocks stay hidden, and the header carries counters.
        assert!(!text.contains("pouring git"), "{text}");
        assert!(text.contains("3/3 done · 0 running · ✗ 1 failed"), "{text}");
    }

    #[test]
    fn in_flight_rows_show_live_command() {
        let mut m = Model::new();
        m.apply(Event::UnitStarted {
            id: "brew-formula:git".into(),
            prompt_capable: false,
        });
        m.apply(Event::Command {
            argv: "brew install --formula git".into(),
            dry_run: false,
            unit: Some("brew-formula:git".into()),
        });
        let mut ui = UiState::new();
        let out = render_plain(&m, &mut ui, 20);
        let text = out.join("\n");
        assert!(text.contains("$ brew install --formula git"), "{text}");
        assert!(text.contains("0/1 done · 1 running"), "{text}");
    }

    #[test]
    fn expansion_reveals_block_lines_inline() {
        let mut m = model_with_rows();
        m.toggle("brew-formula:git");
        m.toggle("mas:123");
        let mut ui = UiState::new();
        let out = render_plain(&m, &mut ui, 20);
        let text = out.join("\n");
        assert!(text.contains("$ brew install --formula git"), "{text}");
        assert!(text.contains("! pouring git"), "{text}");
    }

    #[test]
    fn hit_test_maps_visible_rows_to_units() {
        let m = model_with_rows();
        let mut ui = UiState::new();
        render_lines(&m, &mut ui, 20);
        // Row 0 = header (no target); row 1 = git; row 2 = mas.
        assert_eq!(ui.hit_test(0), None);
        assert_eq!(ui.hit_test(1), Some(0));
        assert_eq!(ui.hit_test(2), Some(1));
        assert_eq!(ui.hit_test(9), None);
    }

    #[test]
    fn scrolling_window_keeps_tail_visible() {
        let m = model_with_rows();
        let mut ui = UiState::new();
        ui.scroll = usize::MAX; // auto-follow
        let out = render_plain(&m, &mut ui, 3);
        assert_eq!(out.len(), 3);
        // Tail: the last three lines (mas row, aggregate, …). The header
        // scrolled off.
        assert!(out.iter().any(|l| l.contains("mas:123")), "{out:?}");
        assert!(!out[0].contains("done"), "{out:?}");
    }

    #[test]
    fn footer_shows_prompt_head_up_and_elevation() {
        let mut m = Model::new();
        m.apply(Event::Section {
            title: "install".into(),
        });
        m.apply(Event::UnitStarted {
            id: "cask:docker".into(),
            prompt_capable: true,
        });
        m.apply(Event::Elevate {
            command: "sudo -v".into(),
            reason: "cask warmup".into(),
        });
        let mut ui = UiState::new();
        let out = render_plain(&m, &mut ui, 20);
        let text = out.join("\n");
        assert!(
            text.contains("⌨ some installs may request your password"),
            "{text}"
        );
        assert!(text.contains("⚠ sudo: sudo -v"), "{text}");
        // Hints only in interactive mode.
        assert!(!text.contains("q exit"), "{text}");
        ui.interactive = true;
        let out = render_plain(&m, &mut ui, 20);
        assert!(out.join("\n").contains("q exit"));
    }

    #[test]
    fn diff_commands_repaints_only_changed_rows() {
        let next = vec![
            ("header".to_string(), RowMapEntry { order_idx: None }),
            ("row".to_string(), RowMapEntry { order_idx: None }),
        ];
        let first = diff_commands(&[], &next, false, 10);
        let s = String::from_utf8(first).unwrap();
        assert!(s.contains("\x1b[1;1H\x1b[2Kheader"), "{s:?}");
        assert!(s.contains("\x1b[2;1H\x1b[2Krow"), "{s:?}");
        assert!(s.ends_with("\x1b[10;1H\x1b[?25l"), "{s:?}");
        // Unchanged second frame: only the cursor park remains.
        let second = diff_commands(&["header".to_string(), "row".to_string()], &next, false, 10);
        assert_eq!(String::from_utf8(second).unwrap(), "\x1b[10;1H\x1b[?25l");
        // Forced repaint rewrites everything.
        let forced = diff_commands(&["header".to_string(), "row".to_string()], &next, true, 10);
        assert!(String::from_utf8(forced).unwrap().contains("header"));
    }

    #[test]
    fn color_off_by_default_in_tests_and_plain_when_no_color() {
        let m = model_with_rows();
        let mut ui = UiState::new();
        ui.color = false;
        let out = render_lines(&m, &mut ui, 20);
        assert!(out.iter().all(|(l, _)| !l.contains('\x1b')));
        ui.color = true;
        let out = render_lines(&m, &mut ui, 20);
        assert!(out.iter().any(|(l, _)| l.contains('\x1b')));
    }
}
