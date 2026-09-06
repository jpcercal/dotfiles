use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Root of `apps.yaml` — the declarative installation + configuration manifest.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Manifest {
    /// Manifest format version; bump + migrate on breaking changes.
    #[serde(rename = "schema_version", default = "default_schema_version")]
    #[schemars(range(min = 1))]
    pub schema_version: u32,
    pub install: Install,
}

fn default_schema_version() -> u32 {
    2
}

/// Canonical names of typed bootstrap steps (implementations live in
/// `dotfiles-backends::bootstrap`; kept here so manifest validation can reject
/// unknown names at edit time).
pub const KNOWN_BOOTSTRAP_STEPS: &[&str] = &[
    "fzf-keybindings",
    "git-lfs",
    "python-links",
    "nvim-plug",
    "opencode",
    "rtk-patch",
    "claude-mem",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Install {
    /// Language toolchains to ensure (rustup/node/python).
    pub toolchains: Toolchains,
    /// Typed, idempotent setup steps (may carry post-install hooks).
    pub bootstrap: Vec<BootstrapEntry>,
    /// Parallel execution tuning for the install phase (the DAG engine).
    pub execution: Execution,
    /// Unified package list: every installable item as `driver:name` with
    /// optional version, label, requires, lock, and lifecycle hooks.
    #[serde(default)]
    pub require: Vec<RequireEntry>,
}

/// A bootstrap entry: either a bare step name (`"opencode"`) or a detailed
/// form with hooks (`{ id: "opencode", hooks: { post-install: "..." } }`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum BootstrapEntry {
    Simple(String),
    Detailed(BootstrapDetail),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BootstrapDetail {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hooks: Option<Hooks>,
}

impl BootstrapEntry {
    pub fn id(&self) -> &str {
        match self {
            BootstrapEntry::Simple(s) => s.as_str(),
            BootstrapEntry::Detailed(d) => d.id.as_str(),
        }
    }
    pub fn hooks(&self) -> Option<&Hooks> {
        match self {
            BootstrapEntry::Simple(_) => None,
            BootstrapEntry::Detailed(d) => d.hooks.as_ref(),
        }
    }
    pub fn has_hooks(&self) -> bool {
        self.hooks().is_some_and(|h| {
            h.pre_install.is_some()
                || h.post_install.is_some()
                || h.pre_update.is_some()
                || h.post_update.is_some()
                || h.pre_uninstall.is_some()
                || h.post_uninstall.is_some()
        })
    }
}

/// Parallel execution tuning for the install phase. `apps.yaml` is the source
/// of truth for the dependency graph; this section tunes the engine that
/// executes it (worker count + per lock-class concurrency).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Execution {
    /// Max parallel install units; 0 = number of available CPUs. Default 0.
    #[serde(default)]
    pub max_jobs: usize,
    /// Per lock-class concurrency overrides, e.g. `{ mas: 4, go: 4 }`.
    /// Keys are lock classes (`brew`, `mas`, `gem`, `npm`, `pip`, `cargo`,
    /// `go`, `composer`, `toolchain`, `bootstrap`); `brew` is capped at 1
    /// (concurrent `brew` invocations are unsupported by Homebrew).
    #[serde(default)]
    pub locks: std::collections::BTreeMap<String, usize>,
}

/// A single entry in `install.require`: either a bare unit ID string
/// (`"brew-formula:git"`) or a detailed map with version / hooks / edges.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum RequireEntry {
    Simple(String),
    Detailed(RequireDetail),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequireDetail {
    /// Canonical unit ID, e.g. `brew-formula:git`, `mas:1352778147`,
    /// `npm:prettier@3` (version suffix is sugar for the `version` field).
    pub id: String,
    /// Human label, required for `mas:` entries (App Store display name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Unit IDs that must complete first, e.g. `["brew-formula:php"]`.
    /// See `crate::units` for the canonical `<prefix>:<name>` namespace.
    #[serde(default)]
    pub requires: Vec<String>,
    /// Resource-class override (default = the backend's own lock class).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = "^[a-z][a-z0-9-]*$"))]
    pub lock: Option<String>,
    /// Explicit version pin (e.g. `"3"` for npm, `"1.9.0"` for gem).
    /// For `npm:`/`pip:`/`go:` the same pin can be written as `id: "npm:prettier@3"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Lifecycle hook snippets executed via `sh -c` through the exec seam.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hooks: Option<Hooks>,
}

/// Lifecycle hook snippets run via `sh -c` through the exec seam.
/// Each field is a shell snippet (may be multi-line). `post-install` fires
/// whenever the unit ends up present (newly installed or already installed,
/// no failures) so config converges; pre hooks fire ahead of their action.
/// Snippets must be idempotency-preserving.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Hooks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "pre-install", alias = "pre_install")]
    pub pre_install: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "post-install", alias = "post_install")]
    pub post_install: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "pre-update", alias = "pre_update")]
    pub pre_update: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "post-update", alias = "post_update")]
    pub post_update: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "pre-uninstall", alias = "pre_uninstall")]
    pub pre_uninstall: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "post-uninstall", alias = "post_uninstall")]
    pub post_uninstall: Option<String>,
}

impl RequireEntry {
    pub fn id(&self) -> &str {
        match self {
            RequireEntry::Simple(s) => s.as_str(),
            RequireEntry::Detailed(d) => d.id.as_str(),
        }
    }

    pub fn label(&self) -> Option<&str> {
        match self {
            RequireEntry::Simple(_) => None,
            RequireEntry::Detailed(d) => d.label.as_deref(),
        }
    }

    pub fn requires(&self) -> &[String] {
        match self {
            RequireEntry::Simple(_) => &[],
            RequireEntry::Detailed(d) => &d.requires,
        }
    }

    pub fn lock(&self) -> Option<&str> {
        match self {
            RequireEntry::Simple(_) => None,
            RequireEntry::Detailed(d) => d.lock.as_deref(),
        }
    }

    pub fn version(&self) -> Option<&str> {
        match self {
            RequireEntry::Simple(_) => None,
            RequireEntry::Detailed(d) => d.version.as_deref(),
        }
    }

    pub fn hooks(&self) -> Option<&Hooks> {
        match self {
            RequireEntry::Simple(_) => None,
            RequireEntry::Detailed(d) => d.hooks.as_ref(),
        }
    }

    pub fn is_detailed(&self) -> bool {
        matches!(self, RequireEntry::Detailed(_))
    }

    /// Effective version considering both `version` field and `@version` suffix
    /// in the id (for pin-capable drivers). Returns `None` if no pin.
    pub fn effective_version(&self) -> Option<String> {
        if let Some(v) = self.version() {
            if !v.trim().is_empty() {
                return Some(v.to_string());
            }
        }
        // Sugar: `id: "npm:prettier@3"` or `"go:module@latest"`
        let id = self.id();
        crate::units::extract_version_from_id(id)
    }

    /// Whether this entry carries any lifecycle hooks.
    pub fn has_hooks(&self) -> bool {
        self.hooks().is_some_and(|h| {
            h.pre_install.is_some()
                || h.post_install.is_some()
                || h.pre_update.is_some()
                || h.post_update.is_some()
                || h.pre_uninstall.is_some()
                || h.post_uninstall.is_some()
        })
    }

    /// Whether this entry is version-pinned (explicit field or @ suffix).
    pub fn is_pinned(&self) -> bool {
        self.effective_version().is_some()
    }
}

// ---------------------------------------------------------------------------
// Backwards-compat aliases: old per-backend module structs are no longer used
// in `Install`, but we keep these type aliases so external `use` statements
// that imported `PkgEntry`/`PkgDetail` don't break during the migration. They
// map to the unified types.
// ---------------------------------------------------------------------------

/// Deprecated alias — use `RequireEntry`.
pub type PkgEntry = RequireEntry;
/// Deprecated alias — use `RequireDetail`.
pub type PkgDetail = RequireDetail;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Toolchains {
    pub rustup: Option<RustupToolchain>,
    pub node: Option<NodeToolchain>,
    pub python: Option<PythonToolchain>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RustupToolchain {
    #[serde(default = "default_rust_channel")]
    pub channel: String,
}

fn default_rust_channel() -> String {
    "stable".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeToolchain {
    /// Currently only `lts` (via fnm).
    #[serde(default = "default_node_ensure")]
    pub ensure: String,
}

fn default_node_ensure() -> String {
    "lts".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PythonToolchain {
    /// Currently only `uv`.
    #[serde(default = "default_python_provider")]
    pub provider: String,
}

fn default_python_provider() -> String {
    "uv".to_string()
}
