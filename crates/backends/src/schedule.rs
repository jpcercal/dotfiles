//! Parallel DAG scheduler for install units.
//!
//! Dynamic ready-queue scheduling over `std::thread::scope` (no async runtime
//! — the whole workspace is blocking std I/O): workers take the first ready
//! unit whose lock class has a free slot, run it, then unblock dependents.
//! A failed unit never aborts the run; its transitive dependents are recorded
//! as `skipped (blocked by <id>)` outcomes. Final outcomes are returned in
//! deterministic unit order regardless of thread timing.

use crate::graph::{Graph, Unit};
use crate::outcome::BackendOutcome;
use dotfiles_exec::ExecEnv;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Condvar, Mutex};

/// Scheduler tuning (defaults come from `install.execution` in apps.yaml;
/// `--jobs` / `--sequential` override on the CLI).
#[derive(Debug, Clone, Default)]
pub struct SchedOpts {
    /// Max worker threads; 0 = number of available CPUs.
    pub max_jobs: usize,
    /// Per lock-class concurrency overrides.
    pub lock_limits: BTreeMap<String, usize>,
}

/// Resolve the worker count (0 = auto).
pub fn effective_jobs(opts: &SchedOpts) -> usize {
    if opts.max_jobs > 0 {
        opts.max_jobs
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    }
}

/// Concurrency limit for a lock class. `brew` is hard-capped at 1
/// (concurrent `brew` invocations are unsupported by Homebrew); everything
/// else defaults to 1 and is tunable via `install.execution.locks`.
pub fn lock_limit(opts: &SchedOpts, class: &str) -> usize {
    if class == "brew" {
        return 1;
    }
    opts.lock_limits
        .get(class)
        .copied()
        .filter(|&n| n > 0)
        .unwrap_or(1)
}

struct State {
    /// Ready unit indices, in graph order (FIFO → deterministic dispatch).
    ready: VecDeque<usize>,
    /// In-degree remaining per unit.
    pending: Vec<usize>,
    /// Dependents per unit.
    dependents: Vec<Vec<usize>>,
    /// In-flight count per lock class.
    locks_used: BTreeMap<String, usize>,
    /// Completed (ok, failed, or skipped) count.
    done: usize,
    outcomes: Vec<Option<BackendOutcome>>,
}

/// Run every unit in `graph`, at most `max_jobs` concurrently and honoring
/// dependency edges + lock classes. `runner` executes one unit (it must be
/// thread-safe; `ExecEnv` is `Send + Sync`).
pub fn run(
    graph: &Graph,
    opts: &SchedOpts,
    env: &ExecEnv,
    runner: &(dyn Fn(&Unit, &ExecEnv) -> BackendOutcome + Sync),
) -> Vec<BackendOutcome> {
    let n = graph.units.len();
    if n == 0 {
        return vec![];
    }
    let index: BTreeMap<&str, usize> = graph
        .units
        .iter()
        .enumerate()
        .map(|(i, u)| (u.id.as_str(), i))
        .collect();

    let mut pending = vec![0usize; n];
    let mut dependents: Vec<Vec<usize>> = vec![vec![]; n];
    for (i, u) in graph.units.iter().enumerate() {
        let mut deps = 0;
        for r in &u.requires {
            if let Some(&j) = index.get(r.as_str()) {
                dependents[j].push(i);
                deps += 1;
            }
            // Unknown requirements cannot happen: graph::build() validates.
        }
        pending[i] = deps;
    }
    let ready: VecDeque<usize> = (0..n).filter(|&i| pending[i] == 0).collect();

    let state = Mutex::new(State {
        ready,
        pending,
        dependents,
        locks_used: BTreeMap::new(),
        done: 0,
        outcomes: (0..n).map(|_| None).collect(),
    });
    let cvar = Condvar::new();

    let workers = effective_jobs(opts).clamp(1, n.max(1));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                // Pick the first ready unit with a free lock slot.
                let task = {
                    let mut st = state.lock().unwrap();
                    loop {
                        if st.done >= n {
                            return;
                        }
                        let pick = st.ready.iter().position(|&i| {
                            let class = graph.units[i].lock.as_str();
                            st.locks_used.get(class).copied().unwrap_or(0) < lock_limit(opts, class)
                        });
                        match pick {
                            Some(pos) => {
                                let i = st.ready.remove(pos).expect("ready position");
                                let class = graph.units[i].lock.clone();
                                *st.locks_used.entry(class).or_insert(0) += 1;
                                break Some(i);
                            }
                            None => {
                                st = cvar.wait(st).unwrap();
                            }
                        }
                    }
                };
                let Some(i) = task else { return };
                let outcome = runner(&graph.units[i], env);
                {
                    let mut st = state.lock().unwrap();
                    let class = graph.units[i].lock.clone();
                    if let Some(used) = st.locks_used.get_mut(&class) {
                        *used = used.saturating_sub(1);
                    }
                    let failed = !outcome.ok();
                    st.outcomes[i] = Some(outcome);
                    st.done += 1;
                    if failed {
                        // Transitive dependents are skipped (blocked), not run.
                        let mut stack: Vec<usize> = st.dependents[i].clone();
                        while let Some(d) = stack.pop() {
                            if st.outcomes[d].is_some() {
                                continue;
                            }
                            let unit = &graph.units[d];
                            let mut skip = BackendOutcome::empty(unit.backend);
                            skip.fail_one(
                                unit.id.clone(),
                                format!("blocked by '{}' (not attempted)", graph.units[i].id),
                            );
                            skip.note = format!("skipped: blocked by '{}'", graph.units[i].id);
                            st.outcomes[d] = Some(skip);
                            st.done += 1;
                            // Remove from ready if already queued.
                            st.ready.retain(|&r| r != d);
                            stack.extend(st.dependents[d].iter().copied());
                        }
                    } else {
                        // Unblock dependents whose in-degree hits zero.
                        let next: Vec<usize> = st.dependents[i].clone();
                        for d in next {
                            if st.outcomes[d].is_some() {
                                continue;
                            }
                            st.pending[d] = st.pending[d].saturating_sub(1);
                            if st.pending[d] == 0 && !st.ready.contains(&d) {
                                st.ready.push_back(d);
                            }
                        }
                    }
                    cvar.notify_all();
                }
            });
        }
    });

    let mut st = state.lock().unwrap();
    st.outcomes
        .iter_mut()
        .map(|o| o.take().expect("every unit completed"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::UnitKind;
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar as StdCondvar, Mutex as StdMutex};
    use std::time::Duration;

    fn unit(id: &str, requires: &[&str], lock: &str) -> Unit {
        Unit {
            id: id.into(),
            kind: UnitKind::Package("test"),
            backend: "test",
            packages: vec![id.into()],
            requires: requires.iter().map(|s| s.to_string()).collect(),
            lock: lock.into(),
        }
    }

    fn graph(units: Vec<Unit>) -> Graph {
        Graph { units }
    }

    fn ok_runner(_: &Unit, _: &ExecEnv) -> BackendOutcome {
        BackendOutcome::empty("test")
    }

    fn opts(jobs: usize) -> SchedOpts {
        SchedOpts {
            max_jobs: jobs,
            lock_limits: BTreeMap::new(),
        }
    }

    #[test]
    fn empty_graph_returns_no_outcomes() {
        let t = dotfiles_testkit::TestEnv::new();
        assert!(run(&graph(vec![]), &opts(4), t.exec(), &ok_runner).is_empty());
    }

    #[test]
    fn independent_units_provably_overlap() {
        // Two workers rendezvous inside the runner via a counting condvar:
        // the test only passes if both units execute concurrently (a lone
        // worker would hit the 10 s timeout and fail).
        let t = dotfiles_testkit::TestEnv::new();
        let pair = Arc::new((StdMutex::new(0usize), StdCondvar::new()));
        let entered = Arc::new(AtomicUsize::new(0));
        let g = graph(vec![unit("a", &[], "lock-a"), unit("b", &[], "lock-b")]);
        let outs = run(&g, &opts(2), t.exec(), &|_: &Unit, _: &ExecEnv| {
            entered.fetch_add(1, Ordering::SeqCst);
            let (lock, cvar) = &*pair;
            let mut n = lock.lock().unwrap();
            *n += 1;
            if *n < 2 {
                let (guard, res) = cvar.wait_timeout(n, Duration::from_secs(10)).unwrap();
                assert!(!res.timed_out(), "units did not overlap");
                drop(guard);
            } else {
                cvar.notify_all();
            }
            BackendOutcome::empty("test")
        });
        assert_eq!(entered.load(Ordering::SeqCst), 2);
        assert_eq!(outs.len(), 2);
    }

    #[test]
    fn same_lock_serializes() {
        let t = dotfiles_testkit::TestEnv::new();
        let live = Arc::new(AtomicUsize::new(0));
        let max_live = Arc::new(AtomicUsize::new(0));
        let g = graph(vec![
            unit("a", &[], "brew"),
            unit("b", &[], "brew"),
            unit("c", &[], "brew"),
        ]);
        run(&g, &opts(8), t.exec(), &|_: &Unit, _: &ExecEnv| {
            let n = live.fetch_add(1, Ordering::SeqCst) + 1;
            max_live.fetch_max(n, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(5));
            live.fetch_sub(1, Ordering::SeqCst);
            BackendOutcome::empty("test")
        });
        assert_eq!(max_live.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn brew_lock_capped_at_one_despite_override() {
        let mut limits = BTreeMap::new();
        limits.insert("brew".to_string(), 16);
        assert_eq!(
            lock_limit(
                &SchedOpts {
                    max_jobs: 0,
                    lock_limits: limits
                },
                "brew"
            ),
            1
        );
    }

    #[test]
    fn failure_blocks_dependents_only() {
        let t = dotfiles_testkit::TestEnv::new();
        let ran = Arc::new(std::sync::Mutex::new(BTreeSet::new()));
        let g = graph(vec![
            unit("fail", &[], "l1"),
            unit("child", &["fail"], "l2"),
            unit("grandchild", &["child"], "l3"),
            unit("sibling", &[], "l4"),
        ]);
        let outs = run(&g, &opts(4), t.exec(), &|u: &Unit, _: &ExecEnv| {
            ran.lock().unwrap().insert(u.id.clone());
            let mut out = BackendOutcome::empty("test");
            if u.id == "fail" {
                out.fail_one("fail", "boom");
            } else {
                out.changed.push(u.id.clone());
            }
            out
        });
        let ran = ran.lock().unwrap();
        assert!(ran.contains("fail"));
        assert!(ran.contains("sibling"));
        assert!(!ran.contains("child"), "blocked unit must not run");
        assert!(!ran.contains("grandchild"), "transitive block");
        assert_eq!(outs.len(), 4);
        let by_name: BTreeMap<String, &BackendOutcome> = outs
            .iter()
            .map(|o| {
                let name = o
                    .changed
                    .first()
                    .cloned()
                    .or_else(|| o.failed.first().map(|f| f.name.clone()))
                    .expect("outcome carries a name");
                (name, o)
            })
            .collect();
        assert!(by_name["fail"].note.is_empty());
        assert!(by_name["sibling"].ok());
        assert_eq!(by_name["child"].note, "skipped: blocked by 'fail'");
        assert!(!by_name["child"].ok());
        assert!(by_name["grandchild"].note.contains("blocked by"));
    }

    #[test]
    fn outcome_order_is_deterministic() {
        let t = dotfiles_testkit::TestEnv::new();
        let g = graph(vec![
            unit("zeta", &[], "l1"),
            unit("alpha", &[], "l2"),
            unit("mid", &["alpha"], "l3"),
        ]);
        for _ in 0..25 {
            let ran = Arc::new(std::sync::Mutex::new(vec![]));
            let outs = run(&g, &opts(4), t.exec(), &|u: &Unit, _: &ExecEnv| {
                ran.lock().unwrap().push(u.id.clone());
                let mut o = BackendOutcome::empty("test");
                o.changed.push(u.id.clone());
                o
            });
            // Outcomes follow graph order even though completion order varies.
            let names: Vec<&str> = outs.iter().map(|o| o.changed[0].as_str()).collect();
            assert_eq!(names, vec!["zeta", "alpha", "mid"]);
            // Execution respected the edge: alpha before mid.
            let order = ran.lock().unwrap().clone();
            assert!(
                order.iter().position(|x| x == "alpha").unwrap()
                    < order.iter().position(|x| x == "mid").unwrap(),
                "{:?}",
                order
            );
        }
    }

    #[test]
    fn single_worker_matches_sequential_semantics() {
        let t = dotfiles_testkit::TestEnv::new();
        let order = Arc::new(std::sync::Mutex::new(vec![]));
        let g = graph(vec![
            unit("a", &[], "l1"),
            unit("b", &["a"], "l2"),
            unit("c", &[], "l3"),
        ]);
        let outs = run(&g, &opts(1), t.exec(), &|u: &Unit, _: &ExecEnv| {
            order.lock().unwrap().push(u.id.clone());
            BackendOutcome::empty("test")
        });
        assert_eq!(outs.len(), 3);
        assert!(outs.iter().all(|o| o.ok()));
        let order = order.lock().unwrap().clone();
        assert!(
            order.iter().position(|x| x == "a").unwrap()
                < order.iter().position(|x| x == "b").unwrap()
        );
    }

    #[test]
    fn effective_jobs_defaults_to_cpus() {
        assert_eq!(effective_jobs(&opts(3)), 3);
        assert!(effective_jobs(&opts(0)) >= 1);
    }
}
