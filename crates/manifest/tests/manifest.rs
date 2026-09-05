use dotfiles_manifest::*;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn parses_real_apps_yaml() {
    let m = load_manifest(&repo_root().join("apps.yaml"))
        .expect("real apps.yaml must parse and validate");
    assert_eq!(m.schema_version, 1);
    assert!(!m.install.brew.formulas.is_empty());
    assert!(!m.install.brew.casks.is_empty());
    assert!(!m.install.mas.apps.is_empty());
    assert!(!m.config.symbolic_links.is_empty());
    assert!(m.config.dockutil.before.reset);
    assert!(m.config.dockutil.before.remove_all);
    assert!(!m.config.dockutil.add.is_empty());
    for entry in &m.config.dockutil.add {
        assert_eq!(entry.after.as_deref(), Some("Finder"));
    }
}

#[test]
fn parses_real_commands_yaml() {
    let c =
        load_commands(&repo_root().join("commands.yaml")).expect("real commands.yaml must parse");
    assert!(c.sections().count() > 30, "expected many sections");
    assert!(c.command_count() > 100, "expected many commands");
    let git = c.0.get("git").expect("git section exists");
    assert!(!git.commands.is_empty());
    for (_, section) in c.sections() {
        assert!(!section.description.trim().is_empty());
        for entry in &section.commands {
            assert!(!entry.command.trim().is_empty());
            assert!(!entry.description.trim().is_empty());
        }
    }
}

#[test]
fn rejects_duplicate_formulas() {
    let yaml = r#"
install:
  brew:
    formulas: ["git", "git"]
"#;
    let err = parse_manifest(yaml).unwrap_err();
    assert!(err.to_string().contains("duplicate entry 'git'"), "{}", err);
}

#[test]
fn rejects_malformed_tap() {
    let yaml = r#"
install:
  brew:
    taps: ["NotATap"]
"#;
    let err = parse_manifest(yaml).unwrap_err();
    assert!(err.to_string().contains("owner/repo"), "{}", err);
}

#[test]
fn rejects_non_numeric_mas_id() {
    let yaml = r#"
install:
  mas:
    apps:
      - { id: "abc", name: "Foo" }
"#;
    let err = parse_manifest(yaml).unwrap_err();
    assert!(err.to_string().contains("numeric App Store id"), "{}", err);
}

#[test]
fn rejects_absolute_link_source() {
    let yaml = r#"
config:
  symbolic_links:
    - from: { relative_path: "/etc/passwd" }
      to: { absolute_path: "~/.x" }
"#;
    let err = parse_manifest(yaml).unwrap_err();
    assert!(err.to_string().contains("must be relative"), "{}", err);
}

#[test]
fn rejects_unknown_fields() {
    let yaml = "install:\n  brew:\n    formuls: [git]\n";
    let err = parse_manifest(yaml).unwrap_err();
    assert!(matches!(err, ManifestError::Yaml { .. }), "{}", err);
}

#[test]
fn empty_document_is_valid_manifest() {
    let m = parse_manifest("---\n").expect("empty doc parses");
    assert_eq!(m.schema_version, 1);
    assert!(m.install.brew.formulas.is_empty());
}

#[test]
fn missing_files_report_io_errors() {
    let err = load_manifest(std::path::Path::new("/nonexistent/apps.yaml")).unwrap_err();
    assert!(matches!(err, ManifestError::Io { .. }), "{}", err);
    let err = load_commands(std::path::Path::new("/nonexistent/commands.yaml")).unwrap_err();
    assert!(matches!(err, ManifestError::Io { .. }), "{}", err);
}

#[test]
fn invalid_yaml_rejected() {
    let err = parse_manifest("install:\n  brew:\n   - [\n").unwrap_err();
    assert!(matches!(err, ManifestError::Yaml { .. }), "{}", err);
}

#[test]
fn rejects_empty_custom_command() {
    let err = parse_manifest("install:\n  brew:\n    customCommands: [\"\"]\n").unwrap_err();
    assert!(err.to_string().contains("empty command"), "{}", err);
}

#[test]
fn rejects_duplicate_mas_id_and_empty_name() {
    let err = parse_manifest(
        "install:\n  mas:\n    apps:\n      - { id: \"1\", name: \"A\" }\n      - { id: \"1\", name: \"B\" }\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("duplicate id 1"), "{}", err);
    let err = parse_manifest("install:\n  mas:\n    apps:\n      - { id: \"2\", name: \"\" }\n")
        .unwrap_err();
    assert!(err.to_string().contains("empty name"), "{}", err);
}

#[test]
fn rejects_unknown_bootstrap_step_and_bad_toolchains() {
    let err = parse_manifest("install:\n  bootstrap: [nope]\n").unwrap_err();
    assert!(err.to_string().contains("unknown step"), "{}", err);
    let err =
        parse_manifest("install:\n  toolchains:\n    node: { ensure: \"20\" }\n").unwrap_err();
    assert!(
        err.to_string().contains("install.toolchains.node.ensure"),
        "{}",
        err
    );
    let err = parse_manifest("install:\n  toolchains:\n    python: { provider: \"system\" }\n")
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("install.toolchains.python.provider"),
        "{}",
        err
    );
}

#[test]
fn rejects_invalid_dock_and_link_entries() {
    let err =
        parse_manifest("config:\n  dockutil:\n    add:\n      - { app: \"relative/Foo.app\" }\n")
            .unwrap_err();
    assert!(err.to_string().contains("absolute .app path"), "{}", err);
    let err = parse_manifest(
        "config:\n  dockutil:\n    add:\n      - { app: \"/Applications/Foo.app\", after: \"\" }\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("empty 'after'"), "{}", err);
    let err = parse_manifest(
        "config:\n  symbolic_links:\n    - from: { relative_path: \".x\" }\n      to: { absolute_path: \"\" }\n",
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("empty to.absolute_path"),
        "{}",
        err
    );
    let err = parse_manifest("install:\n  brew:\n    formulas: [\"\"]\n").unwrap_err();
    assert!(err.to_string().contains("empty entry"), "{}", err);
}

#[test]
fn schema_mentions_all_top_level_sections() {
    let schema = schema_json().expect("schema export");
    for needle in [
        "\"install\"",
        "\"config\"",
        "\"symbolic_links\"",
        "\"dockutil\"",
        "\"mas\"",
        "customCommands",
    ] {
        assert!(schema.contains(needle), "schema missing {}", needle);
    }
}

#[test]
fn parses_mixed_simple_and_detailed_entries() {
    let m = parse_manifest(
        r#"
install:
  execution:
    max_jobs: 8
    locks: { mas: 4 }
  brew:
    formulas:
      - "git"
      - name: "phpstan"
        requires: ["brew-formula:php"]
      - "php"
  mas:
    apps:
      - { id: "1", name: "A", requires: ["brew-formula:git"] }
"#,
    )
    .expect("mixed entries parse");
    assert_eq!(m.install.execution.max_jobs, 8);
    assert_eq!(m.install.execution.locks.get("mas"), Some(&4));
    assert_eq!(m.install.brew.formulas.len(), 3);
    assert!(!m.install.brew.formulas[0].is_detailed());
    assert_eq!(m.install.brew.formulas[0].name(), "git");
    assert!(m.install.brew.formulas[0].requires().is_empty());
    assert!(m.install.brew.formulas[1].is_detailed());
    assert_eq!(
        m.install.brew.formulas[1].requires(),
        &["brew-formula:php".to_string()]
    );
    assert_eq!(m.install.mas.apps[0].requires, vec!["brew-formula:git"]);
}

#[test]
fn rejects_unknown_requires_target() {
    let err = parse_manifest(
        "install:\n  brew:\n    formulas:\n      - \"git\"\n      - { name: \"phpstan\", requires: [\"brew-formula:php\"] }\n",
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("no such package declared"),
        "{}",
        err
    );
    let err = parse_manifest(
        "install:\n  brew:\n    formulas:\n      - { name: \"a\", requires: [\"apt:vim\"] }\n      - \"vim\"\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("unknown unit prefix"), "{}", err);
}

#[test]
fn rejects_dependency_cycles() {
    let err = parse_manifest(
        "install:\n  brew:\n    formulas:\n      - { name: \"a\", requires: [\"brew-formula:b\"] }\n      - { name: \"b\", requires: [\"brew-formula:a\"] }\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("dependency cycle"), "{}", err);
    // self-loop
    let err = parse_manifest(
        "install:\n  brew:\n    formulas:\n      - { name: \"a\", requires: [\"brew-formula:a\"] }\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("dependency cycle"), "{}", err);
}

#[test]
fn rejects_bad_lock_config() {
    let err = parse_manifest("install:\n  execution:\n    locks: { brew: 4 }\n").unwrap_err();
    assert!(err.to_string().contains("'brew' is capped at 1"), "{}", err);
    let err = parse_manifest("install:\n  execution:\n    locks: { 'BAD NAME': 2 }\n").unwrap_err();
    assert!(
        err.to_string().contains("not a valid lock-class name"),
        "{}",
        err
    );
    let err = parse_manifest(
        "install:\n  brew:\n    formulas:\n      - { name: \"a\", lock: \"BAD\" }\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("invalid lock name"), "{}", err);
    let err = parse_manifest(
        "install:\n  brew:\n    formulas:\n      - { name: \"a\", requires: [\"\"] }\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("empty requires entry"), "{}", err);
}

#[test]
fn unit_namespace_helpers() {
    assert_eq!(
        split_unit_id("brew-formula:php"),
        Some(("brew-formula", "php"))
    );
    assert_eq!(
        split_unit_id("go:github.com/x/y@v1.0"),
        Some(("go", "github.com/x/y@v1.0"))
    );
    assert_eq!(split_unit_id("apt:vim"), None);
    assert_eq!(split_unit_id("brew-formula:"), None);
    assert_eq!(split_unit_id("no-colon"), None);
    assert_eq!(lock_class_for("brew-formula"), "brew");
    assert_eq!(lock_class_for("brew-cask"), "brew");
    assert_eq!(lock_class_for("brew-tap"), "brew");
    assert_eq!(lock_class_for("mas"), "mas");
    assert!(is_valid_lock_name("my-lock2"));
    assert!(!is_valid_lock_name("BAD"));
    assert!(!is_valid_lock_name(""));
}

#[test]
fn implicit_edges_follow_declared_tools() {
    let m = parse_manifest(
        "install:\n  brew:\n    formulas: [fnm, uv, fzf, git, rtk]\n  toolchains:\n    node: {}\n    python: {}\n  bootstrap: [fzf-keybindings, git-lfs, python-links, claude-mem, rtk-patch, opencode, nvim-plug]\n",
    )
    .unwrap();
    let ids = unit_ids(&m);
    assert!(!ids.contains("brew-tap:hashicorp/tap"));
    assert!(ids.contains("toolchain:node"));
    assert!(ids.contains("bootstrap:claude-mem"));
    assert_eq!(
        implicit_requires("toolchain:node", &m),
        vec!["brew-formula:fnm"]
    );
    assert_eq!(
        implicit_requires("toolchain:python", &m),
        vec!["brew-formula:uv"]
    );
    assert_eq!(
        implicit_requires("bootstrap:claude-mem", &m),
        vec!["toolchain:node"]
    );
    assert_eq!(
        implicit_requires("bootstrap:python-links", &m),
        vec!["toolchain:python"]
    );
    assert_eq!(
        implicit_requires("bootstrap:fzf-keybindings", &m),
        vec!["brew-formula:fzf"]
    );
    assert!(implicit_requires("bootstrap:opencode", &m).is_empty());
    assert!(implicit_requires("bootstrap:nvim-plug", &m).is_empty());
    // undeclared tools produce no edges (runtime bail/skip preserved)
    let bare = parse_manifest("install:\n  bootstrap: [claude-mem, rtk-patch]\n").unwrap();
    assert!(implicit_requires("bootstrap:claude-mem", &bare).is_empty());
    assert!(implicit_requires("bootstrap:rtk-patch", &bare).is_empty());
}
