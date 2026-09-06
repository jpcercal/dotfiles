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

/// Every known unit-ID prefix (canonical forms).
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

/// Whether this prefix supports `@version` pinning in the id sugar.
fn is_pin_capable(prefix: &str) -> bool {
    matches!(prefix, "npm" | "pip" | "gem" | "cargo" | "go" | "composer")
}

/// Canonicalize a prefix alias to its canonical form.
pub fn canonical_prefix(prefix: &str) -> &str {
    let lower = prefix.trim().to_ascii_lowercase();
    match lower.as_str() {
        "tap" => "brew-tap",
        "formula" | "brew" | "homebrew" => "brew-formula",
        "cask" => "brew-cask",
        _ => {
            // Return original if it matches canonical case-insensitively, else raw
            for &canon in UNIT_PREFIXES {
                if canon.eq_ignore_ascii_case(&lower) {
                    return canon;
                }
            }
            // Unknown — return as-is, validation will reject it
            // Use leaked static? Instead return input's lower? But we need &'static.
            // Fallback: treat as-is (store lower). We handle via owned string helpers.
            // For split_unit_id we return owned canonical via helper below.
            // This function is only used for canonical constants; unknown stays.
            // We can't return dynamic &str as &'static, so we return input prefix
            // when unknown. Caller must map via owned string variant.
            // For simplicity, return the original prefix (unknown case).
            // This arm only hit for known prefixes; unknown returned as original.
            prefix
        }
    }
}

/// Owned canonical prefix — handles the static/dynamic mismatch.
fn canonical_prefix_owned(prefix: &str) -> String {
    let lower = prefix.trim().to_ascii_lowercase();
    match lower.as_str() {
        "tap" => "brew-tap".to_string(),
        "formula" | "brew" | "homebrew" => "brew-formula".to_string(),
        "cask" => "brew-cask".to_string(),
        _ => {
            for &canon in UNIT_PREFIXES {
                if canon.eq_ignore_ascii_case(&lower) {
                    return canon.to_string();
                }
            }
            prefix.to_string()
        }
    }
}

/// Parse an id into (canonical_prefix, bare_name_without_version, Option<version>).
/// For pin-capable drivers the trailing `@version` is stripped (`latest` treated as no pin).
fn parse_id(id: &str) -> Option<(String, String, Option<String>)> {
    let (raw_prefix, raw_name) = id.split_once(':')?;
    if raw_prefix.is_empty() || raw_name.is_empty() {
        return None;
    }
    let prefix = canonical_prefix_owned(raw_prefix);
    if !UNIT_PREFIXES.contains(&prefix.as_str()) {
        return None;
    }
    if raw_name.is_empty() {
        return None;
    }
    if is_pin_capable(&prefix) {
        // Split at last '@' for version sugar.
        if let Some(at) = raw_name.rfind('@') {
            let ver = &raw_name[at + 1..];
            let base = &raw_name[..at];
            if !base.is_empty() && !ver.is_empty() && ver != "latest" {
                return Some((prefix, base.to_string(), Some(ver.to_string())));
            } else if ver == "latest" {
                // `latest` is not a pin; keep bare base for unit identity
                if !base.is_empty() {
                    return Some((prefix, base.to_string(), None));
                }
            }
            // If base empty (e.g. "@scope/pkg" scoped npm) then '@' at 0 is part of name,
            // not a version separator — fall through to no-version.
            if base.is_empty() {
                // e.g. id "npm:@scope/pkg@1.0" -> raw_name "@scope/pkg@1.0"
                // rfind gives at=10? Actually "@scope/pkg@1.0": last @ at 10, base "@scope/pkg", ver "1.0"
                // That's valid: base is "@scope/pkg" not empty, so we handled above.
                // Only case where base empty is id like "npm:@1.0" which is invalid name anyway.
            }
        }
    }
    Some((prefix, raw_name.to_string(), None))
}

/// Split `prefix:name` on the first colon, canonicalizing prefix aliases.
/// Returns `None` for malformed IDs or unknown prefixes.
/// The returned name is bare (version suffix stripped for pin-capable drivers).
pub fn split_unit_id(id: &str) -> Option<(String, String)> {
    parse_id(id).map(|(p, n, _)| (p, n))
}

/// Backward compat for previous API returning &str; prefer `split_unit_id` which now returns owned strings.
/// This wrapper keeps old call sites that matched on &str working via owned conversion.
pub fn split_unit_id_legacy(id: &str) -> Option<(&str, &str)> {
    let (prefix, name) = id.split_once(':')?;
    if prefix.is_empty() || name.is_empty() {
        return None;
    }
    UNIT_PREFIXES.contains(&prefix).then_some((prefix, name))
}

/// Extract version suffix from an id if present (pin-capable drivers only).
/// Handles both `id: "npm:prettier@3"` sugar and explicit `version:` field via caller.
pub fn extract_version_from_id(id: &str) -> Option<String> {
    parse_id(id).and_then(|(_, _, v)| v)
}

/// Normalized unit ID (canonical prefix + bare name, no version).
pub fn normalize_unit_id(id: &str) -> Option<String> {
    parse_id(id).map(|(p, n, _)| format!("{p}:{n}"))
}

/// Bare name without version and prefix.
pub fn bare_name_from_id(id: &str) -> Option<String> {
    parse_id(id).map(|(_, n, _)| n)
}

/// Lock (resource) class for a unit prefix.
pub fn lock_class_for(prefix: &str) -> &'static str {
    let canon = canonical_prefix(prefix);
    match canon {
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

/// Custom lock-class names (`RequireDetail.lock`, `install.execution.locks` keys)
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
    for e in &m.install.require {
        if let Some(norm) = normalize_unit_id(e.id()) {
            ids.insert(norm);
        } else {
            // If normalization fails (invalid prefix), insert raw canonical attempt
            // so validation can report it; but still insert something.
            // Actually for unknown prefix we want validation to catch it, but we
            // still need deterministic set for graph validation. Use raw id.
            if let Some((p, n)) = e.id().split_once(':') {
                let canon = canonical_prefix_owned(p);
                ids.insert(format!("{canon}:{n}"));
            }
        }
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
    for e in &m.install.require {
        let source = match normalize_unit_id(e.id()) {
            Some(n) => n,
            None => {
                // Fallback raw with alias canonicalization for validation path
                if let Some((p, n)) = e.id().split_once(':') {
                    format!("{}:{}", canonical_prefix_owned(p), n)
                } else {
                    e.id().to_string()
                }
            }
        };
        for target in e.requires() {
            // Normalize target aliases as well (requires may use aliases)
            let norm_target = normalize_unit_id(target).unwrap_or_else(|| {
                if let Some((p, n)) = target.split_once(':') {
                    format!("{}:{}", canonical_prefix_owned(p), n)
                } else {
                    target.clone()
                }
            });
            edges.push((source.clone(), norm_target));
        }
    }
    edges
}

fn has_formula(m: &Manifest, name: &str) -> bool {
    m.install.require.iter().any(|e| {
        if let Some((p, n)) = split_unit_id(e.id()) {
            p == "brew-formula" && n == name
        } else {
            false
        }
    })
}

/// Implicit (built-in) requirements for a unit ID, derived from tool
/// realities (fnm/uv/fzf/git/rtk ship via brew, npm needs node, pip needs the
/// uv python, …). Only references *declared* units — anything undeclared is a
/// runtime concern (today's bail/skip behavior), never a graph edge.
/// Explicit `requires:` are unioned with these by callers.
pub fn implicit_requires(id: &str, m: &Manifest) -> Vec<String> {
    let taps: Vec<String> = m
        .install
        .require
        .iter()
        .filter_map(|e| {
            let (p, n) = split_unit_id(e.id())?;
            if p == "brew-tap" {
                Some(format!("brew-tap:{n}"))
            } else {
                None
            }
        })
        .collect();
    let Some((prefix, name)) = split_unit_id(id) else {
        return vec![];
    };
    let prefix_str = prefix.as_str();
    let name_str = name.as_str();
    match prefix_str {
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
        "toolchain" => match name_str {
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
        "bootstrap" => match name_str {
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
