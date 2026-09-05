use crate::apps::Manifest;
use crate::error::ManifestError;
use crate::units;
use std::collections::{BTreeMap, BTreeSet};

/// Semantic validation beyond YAML shape. Errors (not warnings): anything that
/// would make an install run fail ambiguously or silently do the wrong thing.
pub fn validate(m: &Manifest) -> Result<(), ManifestError> {
    let mut errors: Vec<String> = vec![];

    dup_check_entries(
        "install.brew.formulas",
        &m.install.brew.formulas,
        &mut errors,
    );
    dup_check_entries("install.brew.casks", &m.install.brew.casks, &mut errors);
    dup_check("install.brew.taps", &m.install.brew.taps, &mut errors);
    dup_check_entries("install.gem.rubygems", &m.install.gem.rubygems, &mut errors);
    dup_check_entries(
        "install.npm.global.packages",
        &m.install.npm.global.packages,
        &mut errors,
    );
    dup_check_entries("install.pip.packages", &m.install.pip.packages, &mut errors);
    dup_check_entries("install.go.packages", &m.install.go.packages, &mut errors);

    for step in &m.install.bootstrap {
        if !crate::apps::KNOWN_BOOTSTRAP_STEPS.contains(&step.as_str()) {
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

    for tap in &m.install.brew.taps {
        let parts: Vec<&str> = tap.split('/').collect();
        let well_formed = parts.len() == 2
            && parts.iter().all(|p| {
                !p.is_empty()
                    && p.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            });
        if !well_formed {
            errors.push(format!(
                "install.brew.taps: '{}' is not in owner/repo form",
                tap
            ));
        }
    }

    let mut mas_ids = BTreeSet::new();
    for app in &m.install.mas.apps {
        if app.id.is_empty() || !app.id.chars().all(|c| c.is_ascii_digit()) {
            errors.push(format!(
                "install.mas.apps: id '{}' is not a numeric App Store id",
                app.id
            ));
        }
        if !mas_ids.insert(app.id.clone()) {
            errors.push(format!("install.mas.apps: duplicate id {}", app.id));
        }
        if app.name.trim().is_empty() {
            errors.push(format!("install.mas.apps: id {} has an empty name", app.id));
        }
    }

    for cmd in &m.install.brew.custom_commands {
        if cmd.trim().is_empty() {
            errors.push("install.brew.customCommands: empty command".to_string());
        }
    }

    for link in &m.config.symbolic_links {
        if link.from.relative_path.trim().is_empty() {
            errors.push("config.symbolic_links: empty from.relative_path".to_string());
        }
        if link.from.relative_path.starts_with('/') {
            errors.push(format!(
                "config.symbolic_links: from.relative_path '{}' must be relative",
                link.from.relative_path
            ));
        }
        if link.to.absolute_path.trim().is_empty() {
            errors.push("config.symbolic_links: empty to.absolute_path".to_string());
        }
    }

    for entry in &m.config.dockutil.add {
        if !entry.app.starts_with('/') || !entry.app.ends_with(".app") {
            errors.push(format!(
                "config.dockutil.add: app '{}' must be an absolute .app path",
                entry.app
            ));
        }
        if let Some(after) = &entry.after {
            if after.trim().is_empty() {
                errors.push(format!(
                    "config.dockutil.add: entry '{}' has an empty 'after'",
                    entry.app
                ));
            }
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

    for entries in [
        &m.install.brew.formulas,
        &m.install.brew.casks,
        &m.install.gem.rubygems,
        &m.install.npm.global.packages,
        &m.install.pip.packages,
        &m.install.go.packages,
    ] {
        for e in entries {
            if let Some(lock) = e.lock() {
                if !units::is_valid_lock_name(lock) {
                    errors.push(format!(
                        "graph: '{}' has an invalid lock name '{}'",
                        e.name(),
                        lock
                    ));
                }
            }
            for req in e.requires() {
                if req.trim().is_empty() {
                    errors.push(format!("graph: '{}' has an empty requires entry", e.name()));
                }
            }
        }
    }
    for app in &m.install.mas.apps {
        for req in &app.requires {
            if req.trim().is_empty() {
                errors.push(format!("graph: mas:{} has an empty requires entry", app.id));
            }
        }
    }
}

fn dup_check_entries(field: &str, items: &[crate::apps::PkgEntry], errors: &mut Vec<String>) {
    let names: Vec<String> = items.iter().map(|e| e.name().to_string()).collect();
    dup_check(field, &names, errors);
}

fn dup_check(field: &str, items: &[String], errors: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    for item in items {
        if item.trim().is_empty() {
            errors.push(format!("{}: empty entry", field));
        } else if !seen.insert(item.clone()) {
            errors.push(format!("{}: duplicate entry '{}'", field, item));
        }
    }
}
