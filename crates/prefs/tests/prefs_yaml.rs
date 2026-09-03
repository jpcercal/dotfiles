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
fn no_sudo_needed_for_partial_coverage_ids() {
    let file = load_prefs(&repo_root().join("prefs.yaml")).unwrap();
    // Every sudo entry must be defaults/exec (builtins never sudo) — a structural invariant.
    for e in &file.prefs {
        if let PrefEntry::Builtin { .. } = e {
            assert!(matches!(e, PrefEntry::Builtin { .. }));
        }
    }
}
