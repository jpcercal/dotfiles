//! Pure state machine behind the interactive progress UI.
//!
//! The [`Model`] folds the [`Event`](dotfiles_exec::Event) stream into
//! docker-pull-style display state: in-flight rows show their live command,
//! finished no-op rows vanish into per-driver aggregates, and changed/failed
//! rows keep a settled final line with a collapsed (expandable) detail block.
//! No terminal I/O here — rendering lives in `render.rs`, so this is fully
//! unit-testable.

use dotfiles_exec::{Event, Stream, UnitOutcome};
use std::collections::{BTreeMap, VecDeque};

/// Max unscoped/feed lines kept (status lines under the rows).
const FEED_CAP: usize = 4;

/// Visibility of a unit row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowState {
    /// Running: shows a spinner plus the current `$ command`.
    InFlight,
    /// Finished with changes: settled `✓` line, block expandable.
    Changed,
    /// Failed (or blocked): settled `✗` line, block expandable.
    Failed,
}

/// One buffered line inside a unit's collapsed detail block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockLine {
    pub command: bool,
    pub stderr: bool,
    pub text: String,
}

/// One line of the status feed (unscoped commands, notes, warnings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedLine {
    pub stderr: bool,
    pub text: String,
}

/// One settled scrollback line, produced on job boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettleLine {
    pub stderr: bool,
    pub text: String,
}

#[derive(Debug)]
pub struct UnitRow {
    pub id: String,
    pub driver: String,
    pub state: RowState,
    pub current_cmd: Option<String>,
    pub detail: String,
    pub block: Vec<BlockLine>,
    pub expanded: bool,
}

/// Driver namespace of a unit id (`brew-formula:git` → `brew-formula`;
/// sequential chunk ids like `brew` stay whole).
pub fn driver_of(id: &str) -> &str {
    id.split_once(':').map(|(d, _)| d).unwrap_or(id)
}

#[derive(Debug, Default)]
pub struct Model {
    /// Current job title (last `Section`).
    pub section: String,
    /// Visible row ids, in arrival order.
    pub order: Vec<String>,
    pub rows: BTreeMap<String, UnitRow>,
    /// Hidden no-op finishes per driver (`brew-formula` → 118).
    pub aggregates: BTreeMap<String, usize>,
    pub started: usize,
    pub finished: usize,
    pub changed: usize,
    pub failed: usize,
    pub recent: VecDeque<FeedLine>,
    pub last_elevate: Option<(String, String)>,
    pub elevate_count: usize,
}

impl Model {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one event into display state.
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Section { title } => {
                self.reset_job(title);
            }
            Event::Subsection { .. } => {}
            Event::UnitStarted { id } => {
                self.started += 1;
                if !self.rows.contains_key(&id) {
                    self.order.push(id.clone());
                    self.rows.insert(
                        id.clone(),
                        UnitRow {
                            id: id.clone(),
                            driver: driver_of(&id).to_string(),
                            state: RowState::InFlight,
                            current_cmd: None,
                            detail: String::new(),
                            block: vec![],
                            expanded: false,
                        },
                    );
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
                    argv
                };
                match unit {
                    Some(id) => {
                        let row = self.row_or_placeholder(&id);
                        row.current_cmd = Some(text.clone());
                        row.block.push(BlockLine {
                            command: true,
                            stderr: false,
                            text,
                        });
                    }
                    None => self.push_feed(FeedLine {
                        stderr: false,
                        text: format!("$ {text}"),
                    }),
                }
            }
            Event::UnitLog { id, stream, line } => {
                let row = self.row_or_placeholder(&id);
                row.block.push(BlockLine {
                    command: false,
                    stderr: stream == Stream::Stderr,
                    text: line,
                });
            }
            Event::UnitFinished {
                id,
                detail,
                outcome,
                ..
            } => {
                self.finished += 1;
                match outcome {
                    UnitOutcome::NoOp => {
                        // Vanish into the aggregate (docker's "Already exists"
                        // silence): no row, no block, just a counter.
                        if let Some(row) = self.rows.remove(&id) {
                            *self.aggregates.entry(row.driver).or_insert(0) += 1;
                        } else {
                            *self
                                .aggregates
                                .entry(driver_of(&id).to_string())
                                .or_insert(0) += 1;
                        }
                        self.order.retain(|r| r != &id);
                    }
                    UnitOutcome::Changed => {
                        self.changed += 1;
                        let row = self.row_or_placeholder(&id);
                        row.state = RowState::Changed;
                        row.detail = detail;
                        row.current_cmd = None;
                    }
                    UnitOutcome::Failed => {
                        self.failed += 1;
                        let row = self.row_or_placeholder(&id);
                        row.state = RowState::Failed;
                        row.detail = detail;
                        row.current_cmd = None;
                    }
                }
            }
            Event::Elevate { command, reason } => {
                self.elevate_count += 1;
                self.last_elevate = Some((command, reason));
            }
            Event::Note { msg } => self.push_feed(FeedLine {
                stderr: false,
                text: msg,
            }),
            Event::Warn { msg } => self.push_feed(FeedLine {
                stderr: true,
                text: msg,
            }),
        }
    }

    /// Toggle a row's collapsed detail block. Returns the new state.
    pub fn toggle(&mut self, id: &str) -> bool {
        if let Some(row) = self.rows.get_mut(id) {
            row.expanded = !row.expanded;
            row.expanded
        } else {
            false
        }
    }

    /// Scrollback summary for a job boundary (new `Section`, teardown):
    /// settled row lines, per-driver no-op aggregates, sudo usage.
    pub fn settle_lines(&self) -> Vec<SettleLine> {
        let mut out = vec![];
        for id in &self.order {
            if let Some(row) = self.rows.get(id) {
                let (mark, stderr) = match row.state {
                    RowState::InFlight => ("→", false),
                    RowState::Changed => ("✓", false),
                    RowState::Failed => ("✗", true),
                };
                out.push(SettleLine {
                    stderr,
                    text: format!("{mark} {} ({})", row.id, row.detail),
                });
            }
        }
        let mut drivers: Vec<(&String, &usize)> = self.aggregates.iter().collect();
        drivers.sort();
        for (driver, n) in drivers {
            out.push(SettleLine {
                stderr: false,
                text: format!("· {driver}: {n} already installed"),
            });
        }
        if self.elevate_count > 0 {
            let last = self
                .last_elevate
                .as_ref()
                .map(|(c, _)| c.as_str())
                .unwrap_or("?");
            out.push(SettleLine {
                stderr: false,
                text: format!("· ⚠ sudo ×{} (last: {last})", self.elevate_count),
            });
        }
        out
    }

    /// Start a fresh job under `title`, forgetting rows and counters.
    /// Elevation history is process-scoped and survives.
    pub fn reset_job(&mut self, title: String) {
        *self = Model {
            section: title,
            last_elevate: self.last_elevate.take(),
            elevate_count: self.elevate_count,
            ..Default::default()
        };
    }

    fn push_feed(&mut self, line: FeedLine) {
        self.recent.push_back(line);
        while self.recent.len() > FEED_CAP {
            self.recent.pop_front();
        }
    }

    /// Defensive: events for a row we never saw `UnitStarted` for (e.g. a
    /// blocked unit, which only emits `UnitFinished`) still get a row so
    /// nothing user-relevant is dropped.
    fn row_or_placeholder(&mut self, id: &str) -> &mut UnitRow {
        if !self.rows.contains_key(id) {
            self.order.push(id.to_string());
            self.rows.insert(
                id.to_string(),
                UnitRow {
                    id: id.to_string(),
                    driver: driver_of(id).to_string(),
                    state: RowState::InFlight,
                    current_cmd: None,
                    detail: String::new(),
                    block: vec![],
                    expanded: false,
                },
            );
        }
        self.rows.get_mut(id).expect("row just inserted")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotfiles_exec::UnitOutcome;

    fn started(id: &str) -> Event {
        Event::UnitStarted { id: id.into() }
    }

    fn finished(id: &str, outcome: UnitOutcome) -> Event {
        Event::UnitFinished {
            id: id.into(),
            ok: outcome != UnitOutcome::Failed,
            detail: match outcome {
                UnitOutcome::NoOp => "already ok".into(),
                UnitOutcome::Changed => "changed".into(),
                UnitOutcome::Failed => "boom".into(),
            },
            outcome,
        }
    }

    #[test]
    fn in_flight_rows_show_live_command() {
        let mut m = Model::new();
        m.apply(started("brew-formula:git"));
        m.apply(Event::Command {
            argv: "brew install --formula git".into(),
            dry_run: false,
            unit: Some("brew-formula:git".into()),
        });
        let row = &m.rows["brew-formula:git"];
        assert_eq!(row.state, RowState::InFlight);
        assert_eq!(
            row.current_cmd.as_deref(),
            Some("brew install --formula git")
        );
        assert_eq!(m.order, vec!["brew-formula:git"]);
    }

    #[test]
    fn noop_finish_removes_row_into_aggregate() {
        let mut m = Model::new();
        m.apply(started("brew-formula:git"));
        m.apply(Event::UnitLog {
            id: "brew-formula:git".into(),
            stream: Stream::Stdout,
            line: "already there".into(),
        });
        m.apply(finished("brew-formula:git", UnitOutcome::NoOp));
        assert!(!m.rows.contains_key("brew-formula:git"));
        assert!(m.order.is_empty());
        assert_eq!(m.aggregates.get("brew-formula"), Some(&1));
        assert_eq!(m.finished, 1);
    }

    #[test]
    fn changed_and_failed_rows_settle_in_place() {
        let mut m = Model::new();
        m.apply(started("brew-formula:git"));
        m.apply(finished("brew-formula:git", UnitOutcome::Changed));
        m.apply(started("mas:123"));
        m.apply(finished("mas:123", UnitOutcome::Failed));
        assert_eq!(m.rows["brew-formula:git"].state, RowState::Changed);
        assert_eq!(m.rows["mas:123"].state, RowState::Failed);
        assert_eq!(m.rows["mas:123"].detail, "boom");
        assert_eq!(m.order.len(), 2);
        assert_eq!((m.changed, m.failed), (1, 1));
    }

    #[test]
    fn blocked_unit_without_start_still_renders_failed() {
        let mut m = Model::new();
        m.apply(finished("npm:prettier", UnitOutcome::Failed));
        let row = &m.rows["npm:prettier"];
        assert_eq!(row.state, RowState::Failed);
        assert_eq!(m.order, vec!["npm:prettier"]);
    }

    #[test]
    fn toggle_expands_and_collapses() {
        let mut m = Model::new();
        m.apply(started("a"));
        assert!(m.toggle("a"));
        assert!(m.rows["a"].expanded);
        assert!(!m.toggle("a"));
        assert!(!m.toggle("missing"));
    }

    #[test]
    fn section_resets_job_but_keeps_elevation_history() {
        let mut m = Model::new();
        m.apply(Event::Section {
            title: "install".into(),
        });
        m.apply(started("a"));
        m.apply(finished("a", UnitOutcome::NoOp));
        m.apply(Event::Elevate {
            command: "sudo -v".into(),
            reason: "warmup".into(),
        });
        m.apply(Event::Section {
            title: "prefs".into(),
        });
        assert_eq!(m.section, "prefs");
        assert!(m.rows.is_empty() && m.aggregates.is_empty());
        assert_eq!(m.elevate_count, 1);
        assert!(m.last_elevate.is_some());
    }

    #[test]
    fn settle_lines_cover_rows_aggregates_and_sudo() {
        let mut m = Model::new();
        m.apply(started("brew-formula:git"));
        m.apply(finished("brew-formula:git", UnitOutcome::Changed));
        m.apply(started("cask:docker"));
        m.apply(finished("cask:docker", UnitOutcome::NoOp));
        m.apply(Event::Elevate {
            command: "sudo -v".into(),
            reason: "warmup".into(),
        });
        let texts: Vec<String> = m.settle_lines().into_iter().map(|l| l.text).collect();
        assert!(
            texts.iter().any(|t| t == "✓ brew-formula:git (changed)"),
            "{texts:?}"
        );
        assert!(
            texts.iter().any(|t| t == "· cask: 1 already installed"),
            "{texts:?}"
        );
        assert!(texts.iter().any(|t| t.contains("⚠ sudo ×1")), "{texts:?}");
        // No-op rows never appear as row lines.
        assert!(
            !texts.iter().any(|t| t.contains("cask:docker (")),
            "{texts:?}"
        );
    }

    #[test]
    fn feed_keeps_only_recent_lines() {
        let mut m = Model::new();
        for i in 0..10 {
            m.apply(Event::Note {
                msg: format!("n{i}"),
            });
        }
        assert_eq!(m.recent.len(), FEED_CAP);
        assert_eq!(m.recent.back().unwrap().text, "n9");
        m.apply(Event::Command {
            argv: "brew tap".into(),
            dry_run: false,
            unit: None,
        });
        assert_eq!(m.recent.back().unwrap().text, "$ brew tap");
    }

    #[test]
    fn driver_of_handles_bare_and_namespaced_ids() {
        assert_eq!(driver_of("brew-formula:git"), "brew-formula");
        assert_eq!(driver_of("mas:123"), "mas");
        assert_eq!(driver_of("brew"), "brew");
        assert_eq!(driver_of("custom:rustup"), "custom");
    }
}
