use crate::apps::Manifest;
use crate::error::ManifestError;
use crate::units;
use std::collections::{BTreeMap, BTreeSet};

/// Semantic validation beyond YAML shape. Errors (not warnings): anything that
/// would make an install run fail ambiguously or silently do the wrong thing.
pub fn validate(m: &Manifest) -> Result<(), ManifestError> {
    let mut errors: Vec<String> = vec![];

    // --- require list validation ---
    let mut seen_ids = BTreeSet::new();
    for entry in &m.install.require {
        let raw_id = entry.id();
        if raw_id.trim().is_empty() {
            errors.push("install.require: empty id".to_string());
            continue;
        }
        // Parse and canonicalize; check prefix validity
        let parsed = units::split_unit_id(raw_id);
        if parsed.is_none() {
            // Try to give better error: check if colon missing vs unknown prefix
            if !raw_id.contains(':') {
                errors.push(format!(
                    "install.require: '{}' is not a valid unit ID (expected 'prefix:name')",
                    raw_id
                ));
            } else {
                let (pref, _) = raw_id.split_once(':').unwrap();
                let canon = units::canonical_prefix(pref);
                if !units::UNIT_PREFIXES.contains(&canon) {
                    errors.push(format!(
                        "install.require: '{}' has unknown unit prefix '{}' (known: {})",
                        raw_id,
                        pref,
                        units::UNIT_PREFIXES.join(", ")
                    ));
                } else {
                    errors.push(format!(
                        "install.require: '{}' is not a valid unit ID",
                        raw_id
                    ));
                }
            }
            continue;
        }
        let (prefix, bare_name) = parsed.unwrap();
        // bare_name checks
        if bare_name.trim().is_empty() {
            errors.push(format!("install.require: '{}' has empty name", raw_id));
        }
        // Normalized duplicate check
        let norm = units::normalize_unit_id(raw_id).unwrap_or_else(|| raw_id.to_string());
        if !seen_ids.insert(norm.clone()) {
            errors.push(format!("install.require: duplicate entry '{}'", norm));
        }

        // MAS specific
        if prefix == "mas" {
            if bare_name.is_empty() || !bare_name.chars().all(|c| c.is_ascii_digit()) {
                errors.push(format!(
                    "install.require: mas id '{}' is not a numeric App Store id",
                    bare_name
                ));
            }
            match entry.label() {
                Some(l) if !l.trim().is_empty() => {}
                _ => errors.push(format!(
                    "install.require: mas:{} missing non-empty 'label'",
                    bare_name
                )),
            }
        }
        // Label if present must be non-empty (allowed on any entry)
        if let Some(l) = entry.label() {
            if l.trim().is_empty() {
                errors.push(format!("install.require: '{}' has empty label", raw_id));
            }
        }

        // brew-tap shape
        if prefix == "brew-tap" {
            let parts: Vec<&str> = bare_name.split('/').collect();
            let well_formed = parts.len() == 2
                && parts.iter().all(|p| {
                    !p.is_empty()
                        && p.chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                });
            if !well_formed {
                errors.push(format!(
                    "install.require: tap '{}' is not in owner/repo form",
                    bare_name
                ));
            }
        }

        // Version pin validation: hard error for non-pin-capable prefixes
        let is_pinned = entry.is_pinned();
        let pin_capable = matches!(
            prefix.as_str(),
            "npm" | "pip" | "gem" | "cargo" | "go" | "composer"
        );
        // brew-formula/cask/mas/brew-tap/toolchain/bootstrap must not be pinned.
        // For brew-formula, `name@version` containing @ is actually part of name (e.g. node@20)
        // — our parse treats brew-formula as not pin-capable, so extract_version returns None.
        // But explicit `version:` field must still be rejected.
        if entry.version().is_some() && !pin_capable {
            errors.push(format!(
                "install.require: '{}' has a 'version' pin but '{}' does not support version pinning (supported: npm, pip, gem, cargo, go)",
                raw_id, prefix
            ));
        } else if is_pinned && !pin_capable {
            // This would be via @ suffix sugar; for non-pin-capable we already treat @ as part of name,
            // so is_pinned will be false. But keep check for explicit mapping.
            errors.push(format!(
                "install.require: '{}' is version-pinned but '{}' does not support pinning",
                raw_id, prefix
            ));
        }
        // For pin-capable, also validate version string shape not empty
        if let Some(v) = entry.version() {
            if v.trim().is_empty() {
                errors.push(format!("install.require: '{}' has empty version", raw_id));
            }
        }
        // Hooks only on package entries, not taps/toolchains
        if entry.has_hooks() && matches!(prefix.as_str(), "brew-tap" | "toolchain") {
            errors.push(format!(
                "install.require: '{}' has 'hooks' but '{}' entries do not support hooks",
                raw_id, prefix
            ));
        }
    }

    for entry in &m.install.bootstrap {
        let step = entry.id();
        if !crate::apps::KNOWN_BOOTSTRAP_STEPS.contains(&step) {
            errors.push(format!(
                "install.bootstrap: unknown step '{}' (known: {})",
                step,
                crate::apps::KNOWN_BOOTSTRAP_STEPS.join(", ")
            ));
        }
    }

    if let Some(node) = &m.install.toolchains.node {
        if node.ensure != "lts" {
            errors.push(format!(
                "install.toolchains.node.ensure: unsupported value '{}' (supported: lts)",
                node.ensure
            ));
        }
    }
    if let Some(python) = &m.install.toolchains.python {
        if python.provider != "uv" {
            errors.push(format!(
                "install.toolchains.python.provider: unsupported value '{}' (supported: uv)",
                python.provider
            ));
        }
    }

    validate_graph(m, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ManifestError::Validation(errors))
    }
}

/// Dependency-graph validation for the parallel execution engine:
/// `requires:` targets must resolve to declared units, the combined
/// explicit+implicit edge set must be acyclic, and `install.execution.locks`
/// must be well-formed (`brew` is capped at 1 — concurrent `brew`
/// invocations are unsupported by Homebrew).
fn validate_graph(m: &Manifest, errors: &mut Vec<String>) {
    let universe = units::unit_ids(m);

    for (source, target) in units::explicit_edges(m) {
        // Normalize target for display? Keep as stored.
        match units::split_unit_id(&target) {
            None => errors.push(format!(
                "graph: '{}' requires '{}': unknown unit prefix (known: {})",
                source,
                target,
                units::UNIT_PREFIXES.join(", ")
            )),
            Some(_) => {
                if !universe.contains(&target) {
                    errors.push(format!(
                        "graph: '{}' requires '{}': no such package declared in apps.yaml",
                        source, target
                    ));
                }
            }
        }
    }

    // Cycle detection (iterative DFS with an explicit stack; deterministic via
    // BTreeMap/BTreeSet ordering) over explicit ∪ implicit edges.
    let mut adjacency: BTreeMap<&String, Vec<&String>> = BTreeMap::new();
    let edges = units::all_edges(m);
    for (source, target) in edges.iter() {
        adjacency.entry(source).or_default().push(target);
    }
    let mut state: BTreeMap<&String, u8> = BTreeMap::new(); // 0=unseen 1=open 2=done
    for id in universe.iter() {
        state.insert(id, 0);
    }
    let mut stack: Vec<&String> = vec![];
    for id in universe.iter() {
        if state[id] != 0 {
            continue;
        }
        let mut work: Vec<(&String, bool)> = vec![(id, false)];
        while let Some((node, exiting)) = work.pop() {
            if exiting {
                state.insert(node, 2);
                stack.pop();
                continue;
            }
            if state[node] == 2 {
                continue;
            }
            if state[node] == 1 {
                let pos = stack.iter().position(|s| *s == node).unwrap_or(0);
                let mut cycle: Vec<String> = stack[pos..].iter().map(|s| (*s).clone()).collect();
                cycle.push(node.clone());
                errors.push(format!(
                    "graph: dependency cycle detected: {}",
                    cycle.join(" -> ")
                ));
                continue;
            }
            state.insert(node, 1);
            stack.push(node);
            work.push((node, true));
            if let Some(nexts) = adjacency.get(node) {
                let mut ordered: Vec<&String> = nexts.clone();
                ordered.reverse();
                for next in ordered {
                    work.push((next, false));
                }
            }
        }
    }

    for (class, limit) in &m.install.execution.locks {
        if !units::is_valid_lock_name(class) {
            errors.push(format!(
                "install.execution.locks: '{}' is not a valid lock-class name",
                class
            ));
        }
        if class == "brew" && *limit > 1 {
            errors.push(
                "install.execution.locks: 'brew' is capped at 1 (concurrent `brew` invocations are unsupported)"
                    .to_string(),
            );
        }
    }

    for e in &m.install.require {
        if let Some(lock) = e.lock() {
            if !units::is_valid_lock_name(lock) {
                errors.push(format!(
                    "graph: '{}' has an invalid lock name '{}'",
                    e.id(),
                    lock
                ));
            }
        }
        for req in e.requires() {
            if req.trim().is_empty() {
                errors.push(format!("graph: '{}' has an empty requires entry", e.id()));
            }
        }
    }
}
