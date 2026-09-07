//! Pure state machine behind the interactive progress UI.
//!
//! The [`Model`] folds the [`Event`](dotfiles_exec::Event) stream into
//! docker-pull-style display state: every unit keeps a row — settled rows
//! (already-installed no-ops, changed, failed) listed above, in-flight rows
//! below showing the live command. Detail blocks stay collapsed and expand
//! on toggle. No terminal I/O here — rendering lives in `render.rs`, so
//! this is fully unit-testable.

use dotfiles_exec::{Event, Stream, UnitOutcome};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Max unscoped/feed lines kept (status lines under the rows).
const FEED_CAP: usize = 4;

/// Visibility of a unit row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowState {
    /// Running: shows a spinner plus the current `$ command`.
    InFlight,
    /// Already in the desired state: dim settled `✓` line.
    NoOp,
    /// Finished with changes: settled `✓` line, block expandable.
    Changed,
    /// Failed (or blocked): settled `✗` line, block expandable.
    Failed,
}

impl RowState {
    /// Finished (any state) rows sort above in-flight rows.
    pub fn settled(self) -> bool {
        !matches!(self, RowState::InFlight)
    }
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

#[derive(Debug, Clone)]
pub struct UnitRow {
    pub id: String,
    pub state: RowState,
    pub current_cmd: Option<String>,
    pub detail: String,
    pub block: Vec<BlockLine>,
    pub expanded: bool,
}

#[derive(Debug, Default)]
pub struct Model {
    /// Current job title (last `Section`).
    pub section: String,
    /// Visible row ids, in arrival order.
    pub order: Vec<String>,
    pub rows: BTreeMap<String, UnitRow>,
    /// In-flight units that may open an interactive tty prompt (mas/cask
    /// installers, sudo-carrying hooks). Non-empty ⇒ live regions suspend.
    pub prompt_risk: BTreeSet<String>,
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

    /// True while any prompt-capable unit is in flight.
    pub fn may_prompt(&self) -> bool {
        !self.prompt_risk.is_empty()
    }

    /// Fold one event into display state.
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Section { title } => {
                self.reset_job(title);
            }
            Event::Subsection { .. } => {}
            Event::UnitStarted { id, prompt_capable } => {
                self.started += 1;
                if prompt_capable {
                    self.prompt_risk.insert(id.clone());
                }
                if !self.rows.contains_key(&id) {
                    self.order.push(id.clone());
                    self.rows.insert(
                        id.clone(),
                        UnitRow {
                            id: id.clone(),
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
                self.prompt_risk.remove(&id);
                match outcome {
                    UnitOutcome::NoOp => {
                        // Already installed: keep a settled dim row — every
                        // installed app stays visible in the list, above the
                        // units still being installed.
                        let row = self.row_or_placeholder(&id);
                        row.state = RowState::NoOp;
                        row.detail = detail;
                        row.current_cmd = None;
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
            Event::CommandDone { .. } => {
                // Window-closing signal, consumed by the driver.
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
    /// every unit row once once settled lines, plus sudo usage.
    pub fn settle_lines(&self) -> Vec<SettleLine> {
        let mut out = vec![];
        for id in &self.order {
            if let Some(row) = self.rows.get(id) {
                let mark = match row.state {
                    RowState::InFlight => ("→", false),
                    RowState::NoOp => ("✓", false),
                    RowState::Changed => ("✓", false),
                    RowState::Failed => ("✗", true),
                };
                out.push(SettleLine {
                    stderr: mark.1,
                    text: if row.detail.is_empty() {
                        format!("{} {}", mark.0, row.id)
                    } else {
                        format!("{} {} ({})", mark.0, row.id, row.detail)
                    },
                });
            }
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

    /// No-op row ordering for the reviewer: failed first, then changed,
    /// then in-flight, no-ops last.
    fn review_rank(state: RowState) -> u8 {
        match state {
            RowState::Failed => 0,
            RowState::Changed => 1,
            RowState::InFlight => 2,
            RowState::NoOp => 3,
        }
    }

    /// Remove and return one row (id from `order` and `rows`). Used by the
    /// driver to print a finished unit's block in the plain fallback path
    /// (region could not engage) without the region duplicating it later.
    pub fn take_row(&mut self, id: &str) -> Option<UnitRow> {
        if let Some(pos) = self.order.iter().position(|e| e == id) {
            self.order.remove(pos);
        }
        self.rows.remove(id)
    }

    /// Merge every job's model into a single reviewer model: all visible
    /// rows (failed first), all aggregates, and a `review` section title
    /// summarizing what went wrong.
    pub fn merged_for_review(models: &[&Model]) -> Model {
        let mut merged = Model::new();
        merged.section = "review".to_string();
        // Dedup FIRST, in job order (latest job wins on cross-job id
        // collisions, e.g. taps in bootstrap & install), then sort failed
        // first for display.
        let mut by_id: std::collections::BTreeMap<String, UnitRow> = Default::default();
        let mut fresh_order: Vec<String> = vec![];
        for row in models
            .iter()
            .flat_map(|m| m.order.iter().filter_map(|id| m.rows.get(id)).cloned())
        {
            match by_id.entry(row.id.clone()) {
                std::collections::btree_map::Entry::Vacant(v) => {
                    fresh_order.push(row.id.clone());
                    v.insert(row);
                }
                std::collections::btree_map::Entry::Occupied(mut o) => {
                    let _ = o.insert(row);
                }
            }
        }
        let mut final_rows: Vec<UnitRow> = fresh_order
            .into_iter()
            .map(|id| by_id.remove(&id).expect("survivor"))
            .collect();
        final_rows.sort_by_key(|r| Self::review_rank(r.state));
        for row in final_rows {
            merged.order.push(row.id.clone());
            merged.rows.insert(row.id.clone(), row);
        }
        for m in models {
            merged.started += m.started;
            merged.finished += m.finished;
            merged.changed += m.changed;
            merged.failed += m.failed;
            merged.elevate_count += m.elevate_count;
            if m.last_elevate.is_some() {
                merged.last_elevate = m.last_elevate.clone();
            }
        }
        merged
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
        started_capable(id, false)
    }

    fn started_capable(id: &str, prompt_capable: bool) -> Event {
        Event::UnitStarted {
            id: id.into(),
            prompt_capable,
        }
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
    fn noop_finish_keeps_settled_row_visible() {
        let mut m = Model::new();
        m.apply(started("brew-formula:git"));
        m.apply(Event::UnitLog {
            id: "brew-formula:git".into(),
            stream: Stream::Stdout,
            line: "already there".into(),
        });
        m.apply(finished("brew-formula:git", UnitOutcome::NoOp));
        let row = &m.rows["brew-formula:git"];
        assert_eq!(row.state, RowState::NoOp);
        assert_eq!(row.detail, "already ok");
        assert_eq!(m.order, vec!["brew-formula:git"]);
        assert_eq!(m.finished, 1);
        assert!(m.changed == 0 && m.failed == 0);
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
        assert!(m.rows.is_empty());
        assert_eq!(m.elevate_count, 1);
        assert!(m.last_elevate.is_some());
    }

    #[test]
    fn settle_lines_cover_all_rows_and_sudo() {
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
        // Already-installed units settle as rows too (never hidden).
        assert!(
            texts.iter().any(|t| t == "✓ cask:docker (already ok)"),
            "{texts:?}"
        );
        assert!(texts.iter().any(|t| t.contains("⚠ sudo ×1")), "{texts:?}");
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
    fn merged_review_orders_failed_changed_then_noop() {
        let mut job1 = Model::new();
        job1.apply(Event::Section {
            title: "install".into(),
        });
        job1.apply(started("brew-formula:ok"));
        job1.apply(finished("brew-formula:ok", UnitOutcome::Changed));
        job1.apply(started("mas:1"));
        job1.apply(finished("mas:1", UnitOutcome::Failed));
        job1.apply(started("brew-formula:gone"));
        job1.apply(finished("brew-formula:gone", UnitOutcome::NoOp));

        let mut job2 = Model::new();
        job2.apply(Event::Section {
            title: "prefs".into(),
        });
        job2.apply(started("custom:x"));
        job2.apply(finished("custom:x", UnitOutcome::Failed));

        let refs: Vec<&Model> = vec![&job1, &job2];
        let merged = Model::merged_for_review(&refs);
        // Every row is kept; failed first, then changed, no-ops last.
        assert_eq!(
            merged.order,
            vec!["mas:1", "custom:x", "brew-formula:ok", "brew-formula:gone"]
        );
        assert_eq!(merged.rows["mas:1"].state, RowState::Failed);
        assert_eq!(merged.rows["brew-formula:gone"].state, RowState::NoOp);
        assert_eq!(merged.failed, 2);
        assert_eq!(merged.changed, 1);
    }

    #[test]
    fn merged_review_resolves_cross_job_id_collisions() {
        let mut a = Model::new();
        a.apply(started("brew-taps"));
        a.apply(finished("brew-taps", UnitOutcome::Changed));
        let mut b = Model::new();
        b.apply(started("brew-taps"));
        b.apply(finished("brew-taps", UnitOutcome::Failed));
        assert_eq!(b.rows["brew-taps"].state, RowState::Failed);
        let merged = Model::merged_for_review(&[&a, &b]);
        assert_eq!(merged.order, vec!["brew-taps"]);
        assert_eq!(
            merged.rows["brew-taps"].state,
            RowState::Failed,
            "{:?}",
            merged.rows["brew-taps"]
        );
        // Rows map and order must agree on size.
        assert_eq!(merged.rows.len(), merged.order.len());
    }

    #[test]
    fn prompt_risk_tracks_capable_units_in_flight() {
        let mut m = Model::new();
        m.apply(started_capable("cask:docker", true));
        assert!(m.may_prompt());
        m.apply(started("brew-formula:git"));
        assert!(m.may_prompt(), "other in-flight units do not clear risk");
        m.apply(finished("cask:docker", UnitOutcome::Changed));
        assert!(!m.may_prompt(), "finish clears the risk flag");
        // NoOp and Failed finishes clear it too.
        m.apply(started_capable("mas:1", true));
        m.apply(finished("mas:1", UnitOutcome::NoOp));
        assert!(!m.may_prompt());
    }

    #[test]
    fn take_row_removes_row_and_order_entry() {
        let mut m = Model::new();
        m.apply(started("a"));
        m.apply(started("b"));
        let row = m.take_row("a").expect("row");
        assert_eq!(row.id, "a");
        assert_eq!(m.order, vec!["b"]);
        assert!(!m.rows.contains_key("a"));
        assert!(m.take_row("a").is_none());
    }
}
