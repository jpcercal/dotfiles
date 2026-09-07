use dotfiles_manifest::*;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn parses_real_apps_yaml() {
    let m = load_manifest(&repo_root().join("apps.yaml"))
        .expect("real apps.yaml must parse and validate");
    assert_eq!(m.schema_version, 2);
    assert!(!m.install.require.is_empty());
    // config has been moved to post-install hooks; real file has no config section
    // some entries carry hooks (zsh, neovim, etc.)
    assert!(m.install.require.iter().any(|e| e.has_hooks()));
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
  require:
    - "brew-formula:git"
    - "brew-formula:git"
"#;
    let err = parse_manifest(yaml).unwrap_err();
    assert!(err.to_string().contains("duplicate entry"), "{}", err);
}

#[test]
fn rejects_malformed_tap() {
    let yaml = r#"
install:
  require:
    - "brew-tap:NotATap"
"#;
    let err = parse_manifest(yaml).unwrap_err();
    assert!(err.to_string().contains("owner/repo"), "{}", err);
}

#[test]
fn rejects_non_numeric_mas_id() {
    let yaml = r#"
install:
  require:
    - id: "mas:abc"
      label: "Foo"
"#;
    let err = parse_manifest(yaml).unwrap_err();
    assert!(err.to_string().contains("numeric App Store id"), "{}", err);
}

#[test]
fn rejects_absolute_link_source() {
    // config section has been removed — unknown fields are rejected
    let yaml = r#"
config:
  symbolic_links:
    - from: { relative_path: "/etc/passwd" }
      to: { absolute_path: "~/.x" }
"#;
    let err = parse_manifest(yaml).unwrap_err();
    assert!(matches!(err, ManifestError::Yaml { .. }), "{}", err);
}

#[test]
fn rejects_unknown_fields() {
    // Actually top-level unknown field: manifest has deny_unknown_fields, so this fails_yaml
    let yaml2 = "install:\n  require:\n    - \"brew-formula:git\"\n  brew:\n    formulas: [git]\n";
    let err = parse_manifest(yaml2).unwrap_err();
    assert!(matches!(err, ManifestError::Yaml { .. }), "{}", err);
}

#[test]
fn empty_document_is_valid_manifest() {
    let m = parse_manifest("---\n").expect("empty doc parses");
    assert_eq!(m.schema_version, 2);
    assert!(m.install.require.is_empty());
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
    let err = parse_manifest("install:\n  require:\n   - [\n").unwrap_err();
    assert!(matches!(err, ManifestError::Yaml { .. }), "{}", err);
}

#[test]
fn rejects_empty_require_id() {
    let err = parse_manifest("install:\n  require:\n    - \"\"\n").unwrap_err();
    assert!(err.to_string().contains("empty"), "{}", err);
}

#[test]
fn rejects_duplicate_mas_id_and_empty_name() {
    let err = parse_manifest(
        "install:\n  require:\n    - id: \"mas:1\"\n      label: \"A\"\n    - id: \"mas:1\"\n      label: \"B\"\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("duplicate"), "{}", err);
    let err = parse_manifest("install:\n  require:\n    - id: \"mas:2\"\n      label: \"\"\n")
        .unwrap_err();
    assert!(err.to_string().contains("label"), "{}", err);
}

#[test]
fn rejects_hookless_bootstrap_step_and_bad_toolchains() {
    // Bootstrap steps carry no built-in logic — bare entries are dead.
    let err = parse_manifest("install:\n  bootstrap: [nope]\n").unwrap_err();
    assert!(err.to_string().contains("carries no hooks"), "{}", err);
    // Removed typed steps are only valid with hooks attached.
    for step in [
        "fzf-keybindings",
        "git-lfs",
        "python-links",
        "nvim-plug",
        "rtk-patch",
        "claude-mem",
        "opencode",
    ] {
        let err = parse_manifest(&format!("install:\n  bootstrap: [{step}]\n")).unwrap_err();
        assert!(
            err.to_string().contains("carries no hooks"),
            "{step}: {err}"
        );
    }
    // Any step id with hooks is accepted (pure hook carrier).
    let m = parse_manifest(
        "install:\n  bootstrap:\n    - id: \"custom-step\"\n      hooks:\n        post-install: \"echo hi\"\n",
    )
    .unwrap();
    assert_eq!(m.install.bootstrap[0].id(), "custom-step");
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
    // config section removed — these configs are now rejected as unknown fields
    let err =
        parse_manifest("config:\n  dockutil:\n    add:\n      - { app: \"relative/Foo.app\" }\n")
            .unwrap_err();
    assert!(matches!(err, ManifestError::Yaml { .. }), "{}", err);
    let err = parse_manifest("install:\n  require:\n    - \"\"\n").unwrap_err();
    assert!(err.to_string().contains("empty"), "{}", err);
}

#[test]
fn schema_mentions_all_top_level_sections() {
    let schema = schema_json().expect("schema export");
    for needle in ["\"install\"", "\"require\""] {
        assert!(schema.contains(needle), "schema missing {}", needle);
    }
    // config has been removed — ensure it is not present
    assert!(
        !schema.contains("\"symbolic_links\""),
        "schema should not contain config"
    );
}

#[test]
fn parses_mixed_simple_and_detailed_entries() {
    let m = parse_manifest(
        r#"
install:
  execution:
    max_jobs: 8
    locks: { mas: 4 }
  require:
    - "brew-formula:git"
    - id: "brew-formula:phpstan"
      requires: ["brew-formula:php"]
    - "brew-formula:php"
    - id: "mas:1"
      label: "A"
      requires: ["brew-formula:git"]
"#,
    )
    .expect("mixed entries parse");
    assert_eq!(m.install.execution.max_jobs, 8);
    assert_eq!(m.install.execution.locks.get("mas"), Some(&4));
    assert_eq!(m.install.require.len(), 4);
    assert!(!m.install.require[0].is_detailed());
    assert_eq!(m.install.require[0].id(), "brew-formula:git");
    assert!(m.install.require[0].requires().is_empty());
    assert!(m.install.require[1].is_detailed());
    assert_eq!(
        m.install.require[1].requires(),
        &["brew-formula:php".to_string()]
    );
    assert_eq!(m.install.require[3].label(), Some("A"));
}

#[test]
fn rejects_unknown_requires_target() {
    let err = parse_manifest(
        "install:\n  require:\n    - \"brew-formula:git\"\n    - id: \"brew-formula:phpstan\"\n      requires: [\"brew-formula:php\"]\n",
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("no such package declared"),
        "{}",
        err
    );
    let err = parse_manifest(
        "install:\n  require:\n    - id: \"brew-formula:a\"\n      requires: [\"apt:vim\"]\n    - \"brew-formula:vim\"\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("unknown unit prefix"), "{}", err);
}

#[test]
fn rejects_dependency_cycles() {
    let err = parse_manifest(
        "install:\n  require:\n    - id: \"brew-formula:a\"\n      requires: [\"brew-formula:b\"]\n    - id: \"brew-formula:b\"\n      requires: [\"brew-formula:a\"]\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("dependency cycle"), "{}", err);
    // self-loop
    let err = parse_manifest(
        "install:\n  require:\n    - id: \"brew-formula:a\"\n      requires: [\"brew-formula:a\"]\n",
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
    let err =
        parse_manifest("install:\n  require:\n    - id: \"brew-formula:a\"\n      lock: \"BAD\"\n")
            .unwrap_err();
    assert!(err.to_string().contains("invalid lock name"), "{}", err);
    let err = parse_manifest(
        "install:\n  require:\n    - id: \"brew-formula:a\"\n      requires: [\"\"]\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("empty requires entry"), "{}", err);
}

#[test]
fn rejects_version_on_unsupported_drivers() {
    let err = parse_manifest(
        "install:\n  require:\n    - id: \"brew-formula:git\"\n      version: \"1.0\"\n",
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("does not support version"),
        "{}",
        err
    );
    let err = parse_manifest(
        "install:\n  require:\n    - id: \"mas:123\"\n      label: \"Foo\"\n      version: \"1.0\"\n",
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("does not support version"),
        "{}",
        err
    );
}

#[test]
fn unit_namespace_helpers() {
    assert_eq!(
        split_unit_id("brew-formula:php"),
        Some(("brew-formula".to_string(), "php".to_string()))
    );
    // go with version sugar stripped: bare name without @v1.0
    assert_eq!(
        split_unit_id("go:github.com/x/y@v1.0"),
        Some(("go".to_string(), "github.com/x/y".to_string()))
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
        "install:\n  require:\n    - \"brew-formula:fnm\"\n    - \"brew-formula:uv\"\n    - \"brew-formula:git\"\n  toolchains:\n    node: {}\n    python: {}\n  bootstrap:\n    - id: \"opencode\"\n      hooks:\n        post-install: \"echo hi\"\n",
    )
    .unwrap();
    let ids = unit_ids(&m);
    assert!(!ids.contains("brew-tap:hashicorp/tap"));
    assert!(ids.contains("toolchain:node"));
    assert!(ids.contains("bootstrap:opencode"));
    assert_eq!(
        implicit_requires("toolchain:node", &m),
        vec!["brew-formula:fnm"]
    );
    assert_eq!(
        implicit_requires("toolchain:python", &m),
        vec!["brew-formula:uv"]
    );
    // opencode is the only typed step left (remote installer, no tool edges);
    // all other setup moved to post-install hooks on owning packages.
    assert!(implicit_requires("bootstrap:opencode", &m).is_empty());
}

#[test]
fn aliases_normalize() {
    let m = parse_manifest(
        "install:\n  require:\n    - \"tap:owner/repo\"\n    - \"formula:git\"\n    - \"cask:iterm2\"\n",
    )
    .unwrap();
    let ids = unit_ids(&m);
    assert!(ids.contains("brew-tap:owner/repo"));
    assert!(ids.contains("brew-formula:git"));
    assert!(ids.contains("brew-cask:iterm2"));
}

#[test]
fn version_sugar_and_hooks() {
    let m = parse_manifest(
        r#"
install:
  require:
    - id: "npm:prettier@3"
      hooks:
        post-install: "echo hi"
    - id: "gem:neovim"
      version: "0.9.0"
"#,
    )
    .unwrap();
    assert_eq!(
        m.install.require[0].effective_version(),
        Some("3".to_string())
    );
    assert!(m.install.require[0].has_hooks());
    assert_eq!(
        m.install.require[1].effective_version(),
        Some("0.9.0".to_string())
    );
}
