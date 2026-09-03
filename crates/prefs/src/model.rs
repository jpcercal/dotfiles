use dotfiles_manifest::ManifestError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Root of `prefs.yaml` — declarative macOS preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PrefsFile {
    pub prefs: Vec<PrefEntry>,
}

/// One preference. The `kind` tag selects how it's applied:
/// `defaults` | `exec` | `builtin`. (serde cannot combine `flatten` with a
/// tagged enum, so the entry itself is the tagged enum.)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PrefEntry {
    /// `defaults write`
    Defaults {
        /// Unique dotted identifier, e.g. `ui.scrollbars`.
        id: String,
        domain: String,
        key: String,
        #[serde(rename = "type")]
        typ: Typ,
        value: DefaultsValue,
        /// Merge into the existing array/dict instead of replacing it
        /// (`-array-add` / `-dict-add` semantics).
        #[serde(default)]
        add: bool,
        /// `defaults -currentHost write`
        #[serde(default, rename = "current_host")]
        current_host: bool,
        #[serde(default)]
        sudo: bool,
        /// Human note (from the original script's comments).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// Whitelisted system tool invocation (pmset/nvram/killall/...).
    Exec {
        id: String,
        program: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        sudo: bool,
        /// Don't record non-zero exits as failures (e.g. killall for an app
        /// that isn't running — matches the script's `… &> /dev/null`).
        #[serde(default, rename = "ignore_error")]
        ignore_error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// Named engine builtin (e.g. `restart-apps`, `login-item`).
    Builtin {
        id: String,
        name: String,
        /// For `login-item`: absolute .app path.
        #[serde(default)]
        app: Option<String>,
        /// For `login-item`: hide the app on login.
        #[serde(default)]
        hidden: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
}

impl PrefEntry {
    pub fn id(&self) -> &str {
        match self {
            PrefEntry::Defaults { id, .. }
            | PrefEntry::Exec { id, .. }
            | PrefEntry::Builtin { id, .. } => id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Typ {
    Bool,
    Int,
    Float,
    String,
    Array,
    Dict,
}

/// Typed defaults value (`-array` entries and `-dict` values may nest).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum DefaultsValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<DefaultsValue>),
    Map(std::collections::BTreeMap<String, DefaultsValue>),
}

/// Programs the `exec` kind may run. Whitelist = reviewable attack surface.
/// Absolute paths are matched by basename.
pub const ALLOWED_PROGRAMS: &[&str] = &[
    "pmset",
    "nvram",
    "killall",
    "chflags",
    "systemsetup",
    "scutil",
    "mkdir",
    "chmod",
    "touch",
    "osascript",
    "mdutil",
    "networksetup",
    "spctl",
    "xattr",
    "dscacheutil",
    "defaults",
    "open",
    "sqlite3",
    "launchctl",
    "pbcopy",
    "diskutil",
    "PlistBuddy",
    "lsregister",
    "find",
    "rm",
];

/// Is `program` whitelisted? (absolute paths matched by basename)
pub fn program_allowed(program: &str) -> bool {
    let base = program.rsplit('/').next().unwrap_or(program);
    ALLOWED_PROGRAMS.contains(&program) || ALLOWED_PROGRAMS.contains(&base)
}

/// Builtin names the engine implements.
pub const KNOWN_BUILTINS: &[&str] = &["restart-apps", "login-item"];

impl PrefsFile {
    pub fn validate(&self) -> Result<(), ManifestError> {
        let mut errors = vec![];
        let mut ids = BTreeSet::new();
        for e in &self.prefs {
            let id = e.id();
            if id.trim().is_empty() {
                errors.push("pref with empty id".to_string());
            } else if !ids.insert(id.to_string()) {
                errors.push(format!("duplicate pref id '{}'", id));
            }
            match e {
                PrefEntry::Defaults { domain, key, .. } => {
                    if domain.trim().is_empty() || key.trim().is_empty() {
                        errors.push(format!("{}: defaults requires non-empty domain/key", id));
                    }
                }
                PrefEntry::Exec { program, .. } => {
                    if !program_allowed(program) {
                        errors.push(format!(
                            "{}: exec program '{}' not whitelisted (allowed: {})",
                            id,
                            program,
                            ALLOWED_PROGRAMS.join(", ")
                        ));
                    }
                }
                PrefEntry::Builtin { name, app, .. } => {
                    if !KNOWN_BUILTINS.contains(&name.as_str()) {
                        errors.push(format!(
                            "{}: unknown builtin '{}' (known: {})",
                            id,
                            name,
                            KNOWN_BUILTINS.join(", ")
                        ));
                    }
                    if name.as_str() == "login-item" {
                        match app {
                            Some(a)
                                if a.starts_with('/')
                                    && a.trim_end_matches('/').ends_with(".app") => {}
                            _ => errors
                                .push(format!("{}: login-item requires an absolute .app path", id)),
                        }
                    }
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ManifestError::Validation(errors))
        }
    }
}
