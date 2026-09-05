//! Canonical unit-ID namespace for the dependency graph.
//!
//! `apps.yaml` is the source of truth for install dependencies: package
//! entries declare `requires: [...]` edges using these IDs, the schema
//! validates them, and the parallel execution engine (`dotfiles-backends`)
//! schedules them. ID shape is `<prefix>:<name>`, split on the FIRST colon
//! (names may contain `/`, `@`, …).
//!
//! | Kind          | Unit ID example                                    | Lock class |
//! |---------------|----------------------------------------------------|------------|
//! | Tap           | `brew-tap:hashicorp/tap`                            | `brew`     |
//! | Formula       | `brew-formula:php`                                 | `brew`     |
//! | Cask          | `brew-cask:iterm2`                                 | `brew`     |
//! | MAS app       | `mas:1018301773` (numeric id)                      | `mas`      |
//! | Gem           | `gem:neovim`                                       | `gem`      |
//! | npm global    | `npm:prettier`                                     | `npm`      |
//! | pip (uv)      | `pip:pynvim`                                       | `pip`      |
//! | Cargo         | `cargo:ripgrep`                                    | `cargo`    |
//! | Go module     | `go:github.com/oklog/ulid/v2/cmd/ulid@latest`      | `go`       |
//! | Composer      | `composer:vendor/pkg`                              | `composer` |
//! | Toolchain     | `toolchain:rustup` / `node` / `python`             | `toolchain`|
//! | Bootstrap     | `bootstrap:nvim-plug`                              | `bootstrap`|
//!
//! All Homebrew traffic shares the `brew` lock class (limit 1 — concurrent
//! `brew` invocations are unsupported by Homebrew); every other prefix is its
//! own lock class, so cross-ecosystem installs run in parallel.

use crate::apps::Manifest;
use std::collections::BTreeSet;

/// Every known unit-ID prefix.
pub const UNIT_PREFIXES: &[&str] = &[
    "brew-formula",
    "brew-cask",
    "brew-tap",
    "mas",
    "gem",
    "npm",
    "pip",
    "cargo",
    "go",
    "composer",
    "toolchain",
    "bootstrap",
];

/// Lock (resource) classes addressable from `install.execution.locks`.
pub const LOCK_CLASSES: &[&str] = &[
    "brew",
    "mas",
    "gem",
    "npm",
    "pip",
    "cargo",
    "go",
    "composer",
    "toolchain",
    "bootstrap",
];

/// Split `prefix:name` on the first colon. Returns `None` for malformed IDs
/// or unknown prefixes.
pub fn split_unit_id(id: &str) -> Option<(&str, &str)> {
    let (prefix, name) = id.split_once(':')?;
    if prefix.is_empty() || name.is_empty() {
        return None;
    }
    UNIT_PREFIXES.contains(&prefix).then_some((prefix, name))
}

/// Lock (resource) class for a unit prefix.
pub fn lock_class_for(prefix: &str) -> &'static str {
    match prefix {
        "brew-formula" | "brew-cask" | "brew-tap" => "brew",
        "mas" => "mas",
        "gem" => "gem",
        "npm" => "npm",
        "pip" => "pip",
        "cargo" => "cargo",
        "go" => "go",
        "composer" => "composer",
        "toolchain" => "toolchain",
        "bootstrap" => "bootstrap",
        _ => "default",
    }
}

/// Custom lock-class names (`PkgDetail.lock`, `install.execution.locks` keys)
/// must be lowercase slug-shaped.
pub fn is_valid_lock_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Every addressable unit ID declared by the manifest (item-level; backend
/// batch groupings are a scheduler-internal detail resolved in
/// `dotfiles-backends::graph`).
pub fn unit_ids(m: &Manifest) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for tap in &m.install.brew.taps {
        ids.insert(format!("brew-tap:{tap}"));
    }
    for f in &m.install.brew.formulas {
        ids.insert(format!("brew-formula:{}", f.name()));
    }
    for c in &m.install.brew.casks {
        ids.insert(format!("brew-cask:{}", c.name()));
    }
    for g in &m.install.gem.rubygems {
        ids.insert(format!("gem:{}", g.name()));
    }
    for p in &m.install.npm.global.packages {
        ids.insert(format!("npm:{}", p.name()));
    }
    for p in &m.install.pip.packages {
        ids.insert(format!("pip:{}", p.name()));
    }
    for p in &m.install.go.packages {
        ids.insert(format!("go:{}", p.name()));
    }
    for a in &m.install.mas.apps {
        ids.insert(format!("mas:{}", a.id));
    }
    if m.install.toolchains.rustup.is_some() {
        ids.insert("toolchain:rustup".to_string());
    }
    if m.install.toolchains.node.is_some() {
        ids.insert("toolchain:node".to_string());
    }
    if m.install.toolchains.python.is_some() {
        ids.insert("toolchain:python".to_string());
    }
    for step in &m.install.bootstrap {
        ids.insert(format!("bootstrap:{step}"));
    }
    ids
}

/// Explicit (`requires:`) edges as `(source_unit, target_unit)` pairs.
/// Sources are validated to be declared units by the caller.
pub fn explicit_edges(m: &Manifest) -> Vec<(String, String)> {
    let mut edges = vec![];
    let mut push = |source: String, reqs: &[String]| {
        for target in reqs {
            edges.push((source.clone(), target.clone()));
        }
    };
    for f in &m.install.brew.formulas {
        push(format!("brew-formula:{}", f.name()), f.requires());
    }
    for c in &m.install.brew.casks {
        push(format!("brew-cask:{}", c.name()), c.requires());
    }
    for g in &m.install.gem.rubygems {
        push(format!("gem:{}", g.name()), g.requires());
    }
    for p in &m.install.npm.global.packages {
        push(format!("npm:{}", p.name()), p.requires());
    }
    for p in &m.install.pip.packages {
        push(format!("pip:{}", p.name()), p.requires());
    }
    for p in &m.install.go.packages {
        push(format!("go:{}", p.name()), p.requires());
    }
    for a in &m.install.mas.apps {
        push(format!("mas:{}", a.id), &a.requires);
    }
    edges
}

fn has_formula(m: &Manifest, name: &str) -> bool {
    m.install.brew.formulas.iter().any(|f| f.name() == name)
}

/// Implicit (built-in) requirements for a unit ID, derived from tool
/// realities (fnm/uv/fzf/git/rtk ship via brew, npm needs node, pip needs the
/// uv python, …). Only references *declared* units — anything undeclared is a
/// runtime concern (today's bail/skip behavior), never a graph edge.
/// Explicit `requires:` are unioned with these by callers.
pub fn implicit_requires(id: &str, m: &Manifest) -> Vec<String> {
    let taps: Vec<String> = m
        .install
        .brew
        .taps
        .iter()
        .map(|t| format!("brew-tap:{t}"))
        .collect();
    let Some((prefix, name)) = split_unit_id(id) else {
        return vec![];
    };
    match prefix {
        "brew-formula" | "brew-cask" => taps,
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
        "go" => {
            if has_formula(m, "go") {
                vec!["brew-formula:go".to_string()]
            } else {
                vec![]
            }
        }
        "toolchain" => match name {
            // fnm / uv ship via Homebrew; rustup self-downloads.
            "node" => {
                if has_formula(m, "fnm") {
                    vec!["brew-formula:fnm".to_string()]
                } else {
                    vec![]
                }
            }
            "python" => {
                if has_formula(m, "uv") {
                    vec!["brew-formula:uv".to_string()]
                } else {
                    vec![]
                }
            }
            _ => vec![],
        },
        "bootstrap" => match name {
            "fzf-keybindings" => {
                if has_formula(m, "fzf") {
                    vec!["brew-formula:fzf".to_string()]
                } else {
                    vec![]
                }
            }
            "git-lfs" => {
                if has_formula(m, "git") {
                    vec!["brew-formula:git".to_string()]
                } else {
                    vec![]
                }
            }
            "python-links" => {
                if m.install.toolchains.python.is_some() {
                    vec!["toolchain:python".to_string()]
                } else {
                    vec![]
                }
            }
            "rtk-patch" => {
                if has_formula(m, "rtk") {
                    vec!["brew-formula:rtk".to_string()]
                } else {
                    vec![]
                }
            }
            "claude-mem" => {
                if m.install.toolchains.node.is_some() {
                    vec!["toolchain:node".to_string()]
                } else {
                    vec![]
                }
            }
            // nvim-plug (curl) and opencode (remote installer) need no tools.
            _ => vec![],
        },
        _ => vec![],
    }
}

/// Full edge set (explicit ∪ implicit) keyed by source unit, restricted to
/// declared units. Used by validation (cycle detection) and the graph builder.
pub fn all_edges(m: &Manifest) -> BTreeSet<(String, String)> {
    let universe = unit_ids(m);
    let mut edges = BTreeSet::new();
    for (source, target) in explicit_edges(m) {
        if universe.contains(&source)
            && split_unit_id(&target).is_some_and(|_| universe.contains(&target))
        {
            edges.insert((source, target));
        }
    }
    for id in &universe {
        for target in implicit_requires(id, m) {
            if universe.contains(&target) {
                edges.insert((id.clone(), target));
            }
        }
    }
    edges
}
