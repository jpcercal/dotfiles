//! Dependency-graph builder: turns the typed manifest into schedulable units.
//!
//! Grouping rules (all deterministic, manifest order preserved):
//! - one unit per tap (`brew-tap:<tap>`), one unit per MAS app (`mas:<id>`),
//!   one unit per Go module (`go:<module>`), one unit per toolchain and per
//!   bootstrap step;
//! - formulas / casks / gems / npm / pip packages **without** explicit
//!   `requires:`/`lock:` coalesce into one batch unit per backend
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
use dotfiles_manifest::{units, Manifest, PkgEntry};
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

struct Entry<'a> {
    prefix: &'static str,
    backend: &'static str,
    entry: &'a PkgEntry,
}

fn collect(m: &Manifest) -> Vec<Entry<'_>> {
    let mut out = vec![];
    for f in &m.install.brew.formulas {
        out.push(Entry {
            prefix: "brew-formula",
            backend: "brew",
            entry: f,
        });
    }
    for c in &m.install.brew.casks {
        out.push(Entry {
            prefix: "brew-cask",
            backend: "cask",
            entry: c,
        });
    }
    for g in &m.install.gem.rubygems {
        out.push(Entry {
            prefix: "gem",
            backend: "gem",
            entry: g,
        });
    }
    for p in &m.install.npm.global.packages {
        out.push(Entry {
            prefix: "npm",
            backend: "npm",
            entry: p,
        });
    }
    for p in &m.install.pip.packages {
        out.push(Entry {
            prefix: "pip",
            backend: "pip",
            entry: p,
        });
    }
    for p in &m.install.go.packages {
        out.push(Entry {
            prefix: "go",
            backend: "go",
            entry: p,
        });
    }
    out
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
    let mut batch_members: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();

    // Taps: one unit per tap (serialized by the shared `brew` lock).
    for tap in &m.install.brew.taps {
        graph.units.push(Unit {
            id: format!("brew-tap:{tap}"),
            kind: UnitKind::Taps,
            backend: "brew",
            packages: vec![tap.clone()],
            requires: vec![],
            lock: "brew".to_string(),
        });
    }

    // Package entries: detailed or referenced items become singles, the rest
    // accumulate into per-backend batches. (MAS apps and Go modules are
    // handled below — always singles.)
    for e in collect(m) {
        let id = format!("{}:{}", e.prefix, e.entry.name());
        let single = e.entry.is_detailed() || referenced.contains(&id);
        if single {
            graph.units.push(Unit {
                id: id.clone(),
                kind: UnitKind::Package(e.backend),
                backend: e.backend,
                packages: vec![e.entry.name().to_string()],
                requires: requires_for(&id, e.entry, m)?,
                lock: e
                    .entry
                    .lock()
                    .unwrap_or(units::lock_class_for(e.prefix))
                    .to_string(),
            });
        } else {
            batch_members
                .entry(e.prefix)
                .or_default()
                .push(e.entry.name().to_string());
        }
    }

    // Batches (omitted when empty — no edge ever targets an empty batch).
    for (prefix, backend, members) in [
        (
            "brew-formula",
            "brew",
            batch_members.remove("brew-formula").unwrap_or_default(),
        ),
        (
            "brew-cask",
            "cask",
            batch_members.remove("brew-cask").unwrap_or_default(),
        ),
        (
            "gem",
            "gem",
            batch_members.remove("gem").unwrap_or_default(),
        ),
        (
            "npm",
            "npm",
            batch_members.remove("npm").unwrap_or_default(),
        ),
        (
            "pip",
            "pip",
            batch_members.remove("pip").unwrap_or_default(),
        ),
    ] {
        if members.is_empty() {
            continue;
        }
        let id = batch_id(&graph, prefix);
        graph.units.push(Unit {
            id,
            kind: UnitKind::Batch(backend),
            backend,
            packages: members,
            requires: batch_requires(prefix, m),
            lock: units::lock_class_for(prefix).to_string(),
        });
    }

    // Go modules: one unit each.
    for p in &m.install.go.packages {
        let id = format!("go:{}", p.name());
        graph.units.push(Unit {
            id: id.clone(),
            kind: UnitKind::Package("go"),
            backend: "go",
            packages: vec![p.name().to_string()],
            requires: requires_for(&id, p, m)?,
            lock: p.lock().unwrap_or("go").to_string(),
        });
    }

    // MAS apps: one unit each.
    for a in &m.install.mas.apps {
        let id = format!("mas:{}", a.id);
        let mut requires: Vec<String> = a.requires.clone();
        for r in units::implicit_requires(&id, m) {
            if !requires.contains(&r) {
                requires.push(r);
            }
        }
        requires.retain(|r| declared.contains(r));
        requires.sort();
        graph.units.push(Unit {
            id,
            kind: UnitKind::Package("mas"),
            backend: "mas",
            packages: vec![a.id.clone()],
            requires,
            lock: "mas".to_string(),
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
        });
    }

    // Bootstrap steps (manifest order).
    for step in &m.install.bootstrap {
        let id = format!("bootstrap:{step}");
        graph.units.push(Unit {
            id,
            kind: UnitKind::Bootstrap,
            backend: "bootstrap",
            packages: vec![step.clone()],
            requires: requires_for_id(&format!("bootstrap:{step}"), m),
            lock: "bootstrap".to_string(),
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
fn requires_for(id: &str, entry: &PkgEntry, m: &Manifest) -> Result<Vec<String>> {
    let declared = units::unit_ids(m);
    let mut requires: Vec<String> = vec![];
    for r in entry
        .requires()
        .iter()
        .chain(units::implicit_requires(id, m).iter())
    {
        if !declared.contains(r) {
            anyhow::bail!("graph: '{id}' requires '{r}': no such package declared");
        }
        if !requires.contains(r) {
            requires.push(r.clone());
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
            .brew
            .taps
            .iter()
            .map(|t| format!("brew-tap:{t}"))
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
    let backend = match prefix {
        "brew-formula" => "brew",
        "brew-cask" => "cask",
        "brew-tap" => return None, // taps are ensured, not installed
        other => other,
    };
    Some((backend.to_string(), name.to_string()))
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
            "install:\n  brew:\n    taps: [a/b]\n    formulas: [git, jq]\n    casks: [iterm2]\n",
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
            "install:\n  brew:\n    formulas:\n      - git\n      - { name: phpstan, requires: [brew-formula:php] }\n      - php\n",
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
            "install:\n  brew:\n    formulas: [git, fnm]\n  toolchains:\n    node: {}\n  npm:\n    global:\n      packages: [prettier]\n",
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
            "install:\n  go:\n    packages: [example.com/x/tool@latest]\n  mas:\n    apps:\n      - { id: \"123\", name: Foo }\n      - { id: \"456\", name: Bar }\n",
        ))
        .unwrap();
        assert!(g.get("go:example.com/x/tool@latest").is_some());
        assert_eq!(g.get("mas:123").unwrap().lock, "mas");
        assert_eq!(g.get("mas:456").unwrap().packages, vec!["456"]);
    }

    #[test]
    fn bootstrap_steps_carry_implicit_requires() {
        let g = build(&manifest(
            "install:\n  brew:\n    formulas: [fzf]\n  toolchains:\n    node: {}\n    python: {}\n  bootstrap: [fzf-keybindings, claude-mem, opencode]\n",
        ))
        .unwrap();
        assert_eq!(
            g.get("bootstrap:fzf-keybindings").unwrap().requires,
            vec!["brew-formula:fzf"]
        );
        assert_eq!(
            g.get("bootstrap:claude-mem").unwrap().requires,
            vec!["toolchain:node"]
        );
        assert!(g.get("bootstrap:opencode").unwrap().requires.is_empty());
    }

    #[test]
    fn lock_override_is_honored() {
        let g = build(&manifest(
            "install:\n  brew:\n    formulas:\n      - { name: git, lock: my-lock }\n",
        ))
        .unwrap();
        assert_eq!(g.get("brew-formula:git").unwrap().lock, "my-lock");
    }

    #[test]
    fn batch_name_collision_gets_suffix() {
        // A real package literally named `batch` that is referenced splits out
        // as `brew-formula:batch`; the leftover batch unit takes a suffix.
        let g = build(&manifest(
            "install:\n  brew:\n    formulas:\n      - git\n      - { name: other, requires: [brew-formula:batch] }\n      - batch\n",
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
        let mut m = manifest("install:\n  brew:\n    formulas: [git]\n");
        m.install
            .brew
            .formulas
            .push(PkgEntry::Detailed(dotfiles_manifest::PkgDetail {
                name: "x".into(),
                requires: vec!["brew-formula:ghost".into()],
                lock: None,
            }));
        assert!(build(&m).is_err());
    }
}
