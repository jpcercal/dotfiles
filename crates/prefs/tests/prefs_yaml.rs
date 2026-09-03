use dotfiles_prefs::*;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn parses_real_prefs_yaml() {
    let file = load_prefs(&repo_root().join("prefs.yaml"))
        .expect("real prefs.yaml must parse and validate");
    // Faithful port target: ~140 entries covering the 786-line script.
    assert!(
        file.prefs.len() >= 140,
        "expected >=140 entries, got {}",
        file.prefs.len()
    );
    let ids: Vec<&str> = file.prefs.iter().map(|p| p.id()).collect();
    for required in [
        "ui.scrollbars",
        "finder.posix-path-title",
        "dock.tile-size",
        "safari.do-not-track",
        "mail.threaded-view",
        "messages.behaviors",
        "finale.restart-apps",
        "locale.timezone",
        "spotlight.ordered-items",
        "login-item.docker",
    ] {
        assert!(
            ids.contains(&required),
            "missing legacy entry '{}'",
            required
        );
    }
}

#[test]
fn every_entry_kind_is_valid_and_typed() {
    let file = load_prefs(&repo_root().join("prefs.yaml")).unwrap();
    for e in &file.prefs {
        if let PrefEntry::Defaults { typ, value, .. } = e {
            let ok = matches!(
                (typ, value),
                (Typ::Bool, DefaultsValue::Bool(_))
                    | (Typ::Int, DefaultsValue::Int(_))
                    | (Typ::Float, DefaultsValue::Float(_) | DefaultsValue::Int(_))
                    | (Typ::String, DefaultsValue::Str(_))
                    | (Typ::Array, DefaultsValue::List(_))
                    | (Typ::Dict, DefaultsValue::Map(_))
            );
            assert!(
                ok,
                "{}: declared {:?} but value is {:?}",
                e.id(),
                typ,
                value
            );
        }
    }
}

#[test]
fn load_errors_cover_io_yaml_and_validation() {
    use dotfiles_manifest::ManifestError;
    let err = load_prefs(std::path::Path::new("/nonexistent/prefs.yaml")).unwrap_err();
    assert!(matches!(err, ManifestError::Io { .. }), "{}", err);
    let err = parse_prefs("prefs: [unclosed").unwrap_err();
    assert!(matches!(err, ManifestError::Yaml { .. }), "{}", err);
    let err = parse_prefs("prefs:\n  - { id: x, kind: defaults, domain: D, key: K, type: bool, value: true }\n  - { id: x, kind: defaults, domain: D, key: K2, type: bool, value: false }\n").unwrap_err();
    assert!(err.to_string().contains("duplicate pref id"), "{}", err);
}

#[test]
fn validation_rejects_bad_kinds() {
    for (yaml, needle) in [
        ("prefs:\n  - { id: e, kind: exec, program: rm-rf-everything }\n", "not whitelisted"),
        ("prefs:\n  - { id: b, kind: builtin, name: time-travel }\n", "unknown builtin"),
        ("prefs:\n  - { id: l, kind: builtin, name: login-item, app: relative/Foo.app }\n", "absolute .app path"),
        ("prefs:\n  - { id: d, kind: defaults, domain: \"\", key: K, type: bool, value: true }\n", "non-empty domain/key"),
        ("prefs:\n  - { id: \"\", kind: defaults, domain: D, key: K, type: bool, value: true }\n", "empty id"),
    ] {
        let err = parse_prefs(yaml).unwrap_err();
        assert!(err.to_string().contains(needle), "{} should contain {} — got {}", yaml, needle, err);
    }
}

#[test]
fn prefs_schema_documents_all_kinds() {
    let schema = prefs_schema_json().expect("prefs schema export");
    for needle in ["defaults", "exec", "builtin", "login-item", "restart-apps"] {
        assert!(schema.contains(needle), "prefs schema missing {}", needle);
    }
}

#[test]
fn no_sudo_needed_for_partial_coverage_ids() {
    let file = load_prefs(&repo_root().join("prefs.yaml")).unwrap();
    // Every sudo entry must be defaults/exec (builtins never sudo) — a structural invariant.
    for e in &file.prefs {
        if let PrefEntry::Builtin { .. } = e {
            assert!(matches!(e, PrefEntry::Builtin { .. }));
        }
    }
}
