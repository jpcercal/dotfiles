use crate::apps::Manifest;
use crate::error::ManifestError;
use std::collections::BTreeSet;

/// Semantic validation beyond YAML shape. Errors (not warnings): anything that
/// would make an install run fail ambiguously or silently do the wrong thing.
pub fn validate(m: &Manifest) -> Result<(), ManifestError> {
    let mut errors: Vec<String> = vec![];

    dup_check(
        "install.brew.formulas",
        &m.install.brew.formulas,
        &mut errors,
    );
    dup_check("install.brew.casks", &m.install.brew.casks, &mut errors);
    dup_check("install.brew.taps", &m.install.brew.taps, &mut errors);
    dup_check("install.gem.rubygems", &m.install.gem.rubygems, &mut errors);
    dup_check(
        "install.npm.global.packages",
        &m.install.npm.global.packages,
        &mut errors,
    );
    dup_check("install.pip.packages", &m.install.pip.packages, &mut errors);
    dup_check("install.go.packages", &m.install.go.packages, &mut errors);

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

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ManifestError::Validation(errors))
    }
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
