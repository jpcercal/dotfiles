//! Dependency-graph builder: turns the typed manifest into schedulable units.
//!
//! Grouping rules (all deterministic, manifest order preserved):
//! - one unit per tap (`brew-tap:<tap>`), one unit per MAS app (`mas:<id>`),
//!   one unit per Go module (`go:<module>`), one unit per toolchain and per
//!   bootstrap step;
//! - formulas / casks / gems / npm / pip / cargo / composer packages **without** explicit
//!   `requires:`/`lock:`/`version`/`hooks`/`label` coalesce into one batch unit per backend
//!   (`brew-formula:batch`, …) so today's single batched tool invocation is
//!   preserved;
//! - any package **referenced** by another unit's requirements (explicit or
//!   implicit) is split out of its batch into its own single-package unit so
//!   the edge has a real target and dependents unblock as early as possible.
//!
//! Batch unit IDs use the reserved `<prefix>:batch` form; on the pathological
//! collision with a real package literally named `batch`, the ID gains a
//! numeric suffix (`<prefix>:batch:2`, …).

use anyhow::Result;
use dotfiles_manifest::{units, Manifest, RequireEntry};
use std::collections::{BTreeMap, BTreeSet};

/// What a unit executes. The `&'static str` payloads are backend labels used
/// for dispatch and reporting; toolchain/bootstrap dispatch reads the key
/// from `Unit.packages[0]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitKind {
    /// `brew tap` (+ trust) for a subset of taps.
    Taps,
    /// One batched tool invocation (e.g. `brew install --formula …`).
    Batch(&'static str),
    /// A single package inside a backend's tool invocation.
    Package(&'static str),
    /// `rustup` / `node` / `python` toolchain ensure (key in packages[0]).
    Toolchain,
    /// Bootstrap step (name in packages[0]).
    Bootstrap,
}

/// One schedulable work item.
#[derive(Debug, Clone)]
pub struct Unit {
    /// Canonical unit ID (`brew-formula:git`, `mas:123`, …).
    pub id: String,
    pub kind: UnitKind,
    /// Static backend label for reports (`brew`, `cask`, `mas`, …).
    pub backend: &'static str,
    /// Packages this unit installs (batch units hold many, singles hold one;
    /// taps units hold tap names).
    pub packages: Vec<String>,
    /// Resolved unit IDs that must succeed first (explicit ∪ implicit).
    pub requires: Vec<String>,
    /// Lock (resource) class serializing same-tool work.
    pub lock: String,
    /// Pinned version, if any (for npm/pip/gem/cargo/go).
    pub version: Option<String>,
    /// Lifecycle hooks for this unit (only on single package units).
    pub hooks: Option<dotfiles_manifest::Hooks>,
}

/// The install-phase DAG in deterministic (topo-stable) unit order.
#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub units: Vec<Unit>,
}

impl Graph {
    pub fn get(&self, id: &str) -> Option<&Unit> {
        self.units.iter().find(|u| u.id == id)
    }

    pub fn unit_ids(&self) -> BTreeSet<String> {
        self.units.iter().map(|u| u.id.clone()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    pub fn len(&self) -> usize {
        self.units.len()
    }
}

fn backend_for_prefix(prefix: &str) -> &'static str {
    match prefix {
        "brew-formula" => "brew",
        "brew-cask" => "cask",
        "brew-tap" => "brew",
        "mas" => "mas",
        "gem" => "gem",
        "npm" => "npm",
        "pip" => "pip",
        "cargo" => "cargo",
        "go" => "go",
        "composer" => "composer",
        _ => "default",
    }
}

fn is_single_always(prefix: &str) -> bool {
    matches!(prefix, "brew-tap" | "mas" | "go")
}

/// Build the install-phase DAG. Assumes the manifest already passed
/// `dotfiles_manifest::validate` (unknown `requires:` targets are a hard
/// error here as defense-in-depth).
pub fn build(m: &Manifest) -> Result<Graph> {
    // Declared item IDs, and every ID referenced by any edge: referenced
    // items are pinned to single units so edges have real targets.
    let declared = units::unit_ids(m);
    let mut referenced: BTreeSet<String> = BTreeSet::new();
    for (_, target) in units::all_edges(m) {
        referenced.insert(target);
    }

    let mut graph = Graph::default();
    let mut batch_members: BTreeMap<String, Vec<(String, Option<String>)>> = BTreeMap::new();
    // For batch members we store (bare_name, version) but for simple batched installs
    // version is None. Batch units won't have version.

    // Taps: one unit per tap (serialized by the shared `brew` lock).
    for e in &m.install.require {
        let (prefix, name) = match units::split_unit_id(e.id()) {
            Some((p, n)) => (p, n),
            None => continue,
        };
        if prefix != "brew-tap" {
            continue;
        }
        let id = format!("brew-tap:{name}");
        graph.units.push(Unit {
            id: id.clone(),
            kind: UnitKind::Taps,
            backend: "brew",
            packages: vec![name.clone()],
            requires: vec![],
            lock: "brew".to_string(),
            version: None,
            hooks: None,
        });
    }

    // All non-tap, non-mas, non-go entries: apply batching logic
    for e in &m.install.require {
        let (prefix, bare_name) = match units::split_unit_id(e.id()) {
            Some((p, n)) => (p, n),
            None => continue,
        };
        if is_single_always(&prefix) {
            continue;
        }
        // Toolchain/bootstrap not in require — skip
        if prefix == "toolchain" || prefix == "bootstrap" {
            continue;
        }
        let norm_id = format!("{prefix}:{bare_name}");
        let is_detailed = e.is_detailed()
            || e.has_hooks()
            || e.is_pinned()
            || e.label().is_some()
            || referenced.contains(&norm_id);
        if is_detailed {
            let backend = backend_for_prefix(&prefix);
            graph.units.push(Unit {
                id: norm_id.clone(),
                kind: UnitKind::Package(backend),
                backend,
                packages: vec![bare_name.clone()],
                requires: requires_for(&norm_id, e, m)?,
                lock: e
                    .lock()
                    .unwrap_or(units::lock_class_for(&prefix))
                    .to_string(),
                version: e.effective_version(),
                hooks: e.hooks().cloned(),
            });
        } else {
            batch_members
                .entry(prefix.clone())
                .or_default()
                .push((bare_name.clone(), e.effective_version()));
        }
    }

    // Batches (omitted when empty — no edge ever targets an empty batch).
    // For batch units we ignore version (no pin in batch).
    let batch_defs: Vec<(&str, &str)> = vec![
        ("brew-formula", "brew"),
        ("brew-cask", "cask"),
        ("gem", "gem"),
        ("npm", "npm"),
        ("pip", "pip"),
        ("cargo", "cargo"),
        ("composer", "composer"),
    ];
    for (prefix, backend) in batch_defs {
        let members = batch_members.remove(prefix).unwrap_or_default();
        if members.is_empty() {
            continue;
        }
        let names: Vec<String> = members.into_iter().map(|(n, _)| n).collect();
        let id = batch_id(&graph, prefix);
        graph.units.push(Unit {
            id,
            kind: UnitKind::Batch(backend),
            backend,
            packages: names,
            requires: batch_requires(prefix, m),
            lock: units::lock_class_for(prefix).to_string(),
            version: None,
            hooks: None,
        });
    }

    // Go modules: one unit each (always singles)
    for e in &m.install.require {
        let (prefix, bare_name) = match units::split_unit_id(e.id()) {
            Some((p, n)) => (p, n),
            None => continue,
        };
        if prefix != "go" {
            continue;
        }
        let id = format!("go:{bare_name}");
        graph.units.push(Unit {
            id: id.clone(),
            kind: UnitKind::Package("go"),
            backend: "go",
            packages: vec![bare_name.clone()],
            requires: requires_for(&id, e, m)?,
            lock: e.lock().unwrap_or("go").to_string(),
            version: e.effective_version(),
            hooks: e.hooks().cloned(),
        });
    }

    // MAS apps: one unit each.
    for e in &m.install.require {
        let (prefix, bare_name) = match units::split_unit_id(e.id()) {
            Some((p, n)) => (p, n),
            None => continue,
        };
        if prefix != "mas" {
            continue;
        }
        let id = format!("mas:{bare_name}");
        // MAS requires are stored in entry.requires plus implicit
        let mut requires: Vec<String> = e.requires().to_vec();
        for r in units::implicit_requires(&id, m) {
            if !requires.contains(&r) {
                requires.push(r);
            }
        }
        // Normalize requires aliases
        let mut norm_requires: Vec<String> = requires
            .into_iter()
            .map(|r| units::normalize_unit_id(&r).unwrap_or(r))
            .collect();
        norm_requires.retain(|r| declared.contains(r));
        norm_requires.sort();
        // Deduplicate
        norm_requires.dedup();
        graph.units.push(Unit {
            id: id.clone(),
            kind: UnitKind::Package("mas"),
            backend: "mas",
            packages: vec![bare_name.clone()],
            requires: norm_requires,
            lock: e.lock().unwrap_or("mas").to_string(),
            version: None,
            hooks: e.hooks().cloned(),
        });
    }

    // Toolchains (rustup, node, python — manifest order).
    if let Some(r) = &m.install.toolchains.rustup {
        let _ = r;
        graph.units.push(Unit {
            id: "toolchain:rustup".to_string(),
            kind: UnitKind::Toolchain,
            backend: "toolchain",
            packages: vec!["rustup".to_string()],
            requires: requires_for_id("toolchain:rustup", m),
            lock: "toolchain".to_string(),
            version: None,
            hooks: None,
        });
    }
    if m.install.toolchains.node.is_some() {
        graph.units.push(Unit {
            id: "toolchain:node".to_string(),
            kind: UnitKind::Toolchain,
            backend: "toolchain",
            packages: vec!["node".to_string()],
            requires: requires_for_id("toolchain:node", m),
            lock: "toolchain".to_string(),
            version: None,
            hooks: None,
        });
    }
    if m.install.toolchains.python.is_some() {
        graph.units.push(Unit {
            id: "toolchain:python".to_string(),
            kind: UnitKind::Toolchain,
            backend: "toolchain",
            packages: vec!["python".to_string()],
            requires: requires_for_id("toolchain:python", m),
            lock: "toolchain".to_string(),
            version: None,
            hooks: None,
        });
    }

    // Bootstrap steps (manifest order).
    for entry in &m.install.bootstrap {
        let step = entry.id();
        let id = format!("bootstrap:{step}");
        graph.units.push(Unit {
            id: id.clone(),
            kind: UnitKind::Bootstrap,
            backend: "bootstrap",
            packages: vec![step.to_string()],
            requires: requires_for_id(&id, m),
            lock: "bootstrap".to_string(),
            version: None,
            hooks: entry.hooks().cloned(),
        });
    }

    // Defense-in-depth: every edge target must be a built unit.
    let built = graph.unit_ids();
    for u in &graph.units {
        for r in &u.requires {
            if !built.contains(r) {
                anyhow::bail!("graph: '{}' requires '{}': no such unit built", u.id, r);
            }
        }
    }

    Ok(graph)
}

/// `<prefix>:batch`, with a numeric suffix on collision with a real package
/// literally named `batch` (or a duplicate batch — impossible by construction).
fn batch_id(graph: &Graph, prefix: &str) -> String {
    let base = format!("{prefix}:batch");
    if graph.get(&base).is_none() && !graph.unit_ids().contains(&base) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{prefix}:batch:{n}");
        if graph.get(&candidate).is_none() {
            return candidate;
        }
        n += 1;
    }
}

/// Explicit ∪ implicit requirements for a single-package unit, validated
/// against declared units and sorted for determinism.
fn requires_for(id: &str, entry: &RequireEntry, m: &Manifest) -> Result<Vec<String>> {
    let declared = units::unit_ids(m);
    let mut requires: Vec<String> = vec![];
    for r in entry
        .requires()
        .iter()
        .chain(units::implicit_requires(id, m).iter())
    {
        let norm = units::normalize_unit_id(r).unwrap_or_else(|| r.clone());
        if !declared.contains(&norm) {
            anyhow::bail!("graph: '{id}' requires '{r}': no such package declared");
        }
        if !requires.contains(&norm) {
            requires.push(norm);
        }
    }
    requires.sort();
    Ok(requires)
}

fn requires_for_id(id: &str, m: &Manifest) -> Vec<String> {
    let declared = units::unit_ids(m);
    let mut requires: Vec<String> = units::implicit_requires(id, m)
        .into_iter()
        .filter(|r| declared.contains(r))
        .collect();
    requires.sort();
    requires
}

/// Batch-level implicit requirements (mirrors `units::implicit_requires` at
/// item granularity): brew batches wait for taps, npm/pip batches for their
/// toolchains when declared.
fn batch_requires(prefix: &str, m: &Manifest) -> Vec<String> {
    match prefix {
        "brew-formula" | "brew-cask" => m
            .install
            .require
            .iter()
            .filter_map(|e| {
                let (p, n) = units::split_unit_id(e.id())?;
                if p == "brew-tap" {
                    Some(format!("brew-tap:{n}"))
                } else {
                    None
                }
            })
            .collect(),
        "npm" => {
            if m.install.toolchains.node.is_some() {
                vec!["toolchain:node".to_string()]
            } else {
                vec![]
            }
        }
        "pip" => {
            if m.install.toolchains.python.is_some() {
                vec!["toolchain:python".to_string()]
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}

/// Why a unit exists (for reports): batch membership vs pinned single.
#[allow(dead_code)]
pub fn describe(u: &Unit) -> String {
    match u.kind {
        UnitKind::Taps => format!("taps {}", u.packages.join(", ")),
        UnitKind::Batch(_) => format!("{} ({} pkgs)", u.id, u.packages.len()),
        _ => u.id.clone(),
    }
}

/// Resolve a canonical unit ID back to a CLI-installable `backend:name` spec
/// (used by error messages and `dotfiles verify`). Batch IDs have no single
/// package and resolve to `None`.
pub fn unit_id_to_spec(id: &str) -> Option<(String, String)> {
    let (prefix, name) = units::split_unit_id(id)?;
    if name == "batch" || name.starts_with("batch:") {
        return None;
    }
    let backend = match prefix.as_str() {
        "brew-formula" => "brew".to_string(),
        "brew-cask" => "cask".to_string(),
        "brew-tap" => return None, // taps are ensured, not installed
        other => other.to_string(),
    };
    Some((backend, name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotfiles_manifest::parse_manifest;

    fn manifest(yaml: &str) -> Manifest {
        parse_manifest(yaml).expect("test manifest must validate")
    }

    #[test]
    fn empty_manifest_builds_empty_graph() {
        let g = build(&manifest("---\n")).unwrap();
        assert!(g.is_empty());
    }

    #[test]
    fn simple_entries_coalesce_into_batches() {
        let g = build(&manifest(
            "install:\n  require:\n    - \"brew-tap:a/b\"\n    - \"brew-formula:git\"\n    - \"brew-formula:jq\"\n    - \"brew-cask:iterm2\"\n",
        ))
        .unwrap();
        let ids: Vec<&str> = g.units.iter().map(|u| u.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["brew-tap:a/b", "brew-formula:batch", "brew-cask:batch",]
        );
        let batch = g.get("brew-formula:batch").unwrap();
        assert_eq!(batch.packages, vec!["git", "jq"]);
        assert_eq!(batch.requires, vec!["brew-tap:a/b"]);
        assert_eq!(batch.lock, "brew");
        assert_eq!(g.get("brew-cask:batch").unwrap().lock, "brew");
    }

    #[test]
    fn referenced_packages_split_out_of_batches() {
        let g = build(&manifest(
            "install:\n  require:\n    - \"brew-formula:git\"\n    - id: \"brew-formula:phpstan\"\n      requires: [\"brew-formula:php\"]\n    - \"brew-formula:php\"\n",
        ))
        .unwrap();
        // php is referenced → single; git stays batched; phpstan is detailed → single.
        let php = g.get("brew-formula:php").unwrap();
        assert_eq!(php.packages, vec!["php"]);
        assert!(matches!(php.kind, UnitKind::Package("brew")));
        let phpstan = g.get("brew-formula:phpstan").unwrap();
        assert_eq!(phpstan.requires, vec!["brew-formula:php"]);
        let batch = g.get("brew-formula:batch").unwrap();
        assert_eq!(batch.packages, vec!["git"]);
    }

    #[test]
    fn implicit_toolchain_edges_pin_tool_formulas() {
        let g = build(&manifest(
            "install:\n  require:\n    - \"brew-formula:git\"\n    - \"brew-formula:fnm\"\n    - \"npm:prettier\"\n  toolchains:\n    node: {}\n",
        ))
        .unwrap();
        // fnm is referenced by toolchain:node → single unit.
        assert!(g.get("brew-formula:fnm").is_some());
        let node = g.get("toolchain:node").unwrap();
        assert_eq!(node.requires, vec!["brew-formula:fnm"]);
        // npm batch waits for the node toolchain.
        let npm = g.get("npm:batch").unwrap();
        assert_eq!(npm.requires, vec!["toolchain:node"]);
        // git stays in the batch.
        assert_eq!(g.get("brew-formula:batch").unwrap().packages, vec!["git"]);
    }

    #[test]
    fn mas_and_go_become_single_units() {
        let g = build(&manifest(
            "install:\n  require:\n    - \"go:example.com/x/tool@latest\"\n    - id: \"mas:123\"\n      label: \"Foo\"\n    - id: \"mas:456\"\n      label: \"Bar\"\n",
        ))
        .unwrap();
        assert!(
            g.get("go:example.com/x/tool@latest").is_some()
                || g.get("go:example.com/x/tool").is_some()
        );
        assert_eq!(g.get("mas:123").unwrap().lock, "mas");
        assert_eq!(g.get("mas:456").unwrap().packages, vec!["456"]);
    }

    #[test]
    fn bootstrap_opencode_carries_hooks_and_no_implicit_requires() {
        // opencode is the only remaining typed step; all other setup moved
        // to post-install hooks on owning packages.
        let g = build(&manifest(
            "install:\n  require:\n    - \"brew-formula:fzf\"\n  toolchains:\n    node: {}\n    python: {}\n  bootstrap:\n    - id: \"opencode\"\n      hooks:\n        post-install: \"echo hi\"\n",
        ))
        .unwrap();
        let unit = g.get("bootstrap:opencode").unwrap();
        assert!(unit.requires.is_empty());
        assert_eq!(
            unit.hooks.as_ref().unwrap().post_install.as_deref(),
            Some("echo hi")
        );
    }

    #[test]
    fn lock_override_is_honored() {
        let g = build(&manifest(
            "install:\n  require:\n    - id: \"brew-formula:git\"\n      lock: \"my-lock\"\n",
        ))
        .unwrap();
        assert_eq!(g.get("brew-formula:git").unwrap().lock, "my-lock");
    }

    #[test]
    fn batch_name_collision_gets_suffix() {
        // A real package literally named `batch` that is referenced splits out
        // as `brew-formula:batch`; the leftover batch unit takes a suffix.
        let g = build(&manifest(
            "install:\n  require:\n    - \"brew-formula:git\"\n    - id: \"brew-formula:other\"\n      requires: [\"brew-formula:batch\"]\n    - \"brew-formula:batch\"\n",
        ))
        .unwrap();
        assert!(g.get("brew-formula:batch").is_some()); // the real package
        let leftovers: Vec<&Unit> = g
            .units
            .iter()
            .filter(|u| u.id.starts_with("brew-formula:batch:"))
            .collect();
        assert_eq!(leftovers.len(), 1);
        assert_eq!(leftovers[0].packages, vec!["git"]);
    }

    #[test]
    fn unit_id_to_spec_mapping() {
        assert_eq!(
            unit_id_to_spec("brew-formula:git"),
            Some(("brew".into(), "git".into()))
        );
        assert_eq!(
            unit_id_to_spec("brew-cask:iterm2"),
            Some(("cask".into(), "iterm2".into()))
        );
        assert_eq!(
            unit_id_to_spec("mas:123"),
            Some(("mas".into(), "123".into()))
        );
        assert_eq!(unit_id_to_spec("brew-tap:a/b"), None);
        assert_eq!(unit_id_to_spec("brew-formula:batch"), None);
        assert_eq!(unit_id_to_spec("nope"), None);
    }

    #[test]
    fn unknown_requires_target_bails_in_build() {
        // parse_manifest validates, so craft the invalid state by editing a
        // valid manifest in memory (defense-in-depth path).
        let mut m = manifest("install:\n  require:\n    - \"brew-formula:git\"\n");
        m.install
            .require
            .push(RequireEntry::Detailed(dotfiles_manifest::RequireDetail {
                id: "brew-formula:x".into(),
                label: None,
                requires: vec!["brew-formula:ghost".into()],
                lock: None,
                version: None,
                hooks: None,
            }));
        assert!(build(&m).is_err());
    }

    #[test]
    fn alias_prefixes_normalize() {
        let g = build(&manifest(
            "install:\n  require:\n    - \"tap:a/b\"\n    - \"formula:git\"\n    - \"cask:iterm2\"\n",
        ))
        .unwrap();
        assert!(g.get("brew-tap:a/b").is_some());
        assert!(g.get("brew-formula:git").is_some() || g.get("brew-formula:batch").is_some());
        assert!(g.get("brew-cask:iterm2").is_some() || g.get("brew-cask:batch").is_some());
    }

    #[test]
    fn version_and_hooks_split_to_single() {
        let g = build(&manifest(
            "install:\n  require:\n    - \"brew-formula:git\"\n    - id: \"npm:prettier@3\"\n      hooks:\n        post-install: \"echo hi\"\n",
        ))
        .unwrap();
        // git stays batched, prettier is detailed due to version/hooks → single
        assert!(g.get("npm:prettier").is_some());
        let u = g.get("npm:prettier").unwrap();
        assert_eq!(u.version.as_deref(), Some("3"));
        assert!(u.hooks.is_some());
        assert!(g.get("brew-formula:batch").is_some());
    }
}
