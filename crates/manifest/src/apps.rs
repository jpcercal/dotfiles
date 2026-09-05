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
    pub config: Config,
}

fn default_schema_version() -> u32 {
    1
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
    pub brew: Brew,
    pub gem: Gem,
    pub npm: Npm,
    pub pip: Pip,
    pub go: GoPackages,
    pub mas: Mas,
    /// Language toolchains to ensure (rustup/node/python).
    pub toolchains: Toolchains,
    /// Typed, idempotent setup steps (replacements for customCommands).
    #[schemars(with = "Vec<String>")]
    pub bootstrap: Vec<String>,
    /// Parallel execution tuning for the install phase (the DAG engine).
    pub execution: Execution,
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

/// A package list entry: either a bare name (`- "git"`) or a detailed form
/// (`- { name: "phpstan", requires: ["brew-formula:php"] }`) that declares
/// dependency-graph edges for the parallel execution engine. The detailed
/// form splits the package out of its backend's batched install into its own
/// schedulable unit.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PkgEntry {
    Simple(String),
    Detailed(PkgDetail),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PkgDetail {
    pub name: String,
    /// Unit IDs that must complete first, e.g. `["brew-formula:php"]`.
    /// See `crate::units` for the canonical `<prefix>:<name>` namespace.
    #[serde(default)]
    pub requires: Vec<String>,
    /// Resource-class override (default = the backend's own lock class).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = "^[a-z][a-z0-9-]*$"))]
    pub lock: Option<String>,
}

impl PkgEntry {
    pub fn name(&self) -> &str {
        match self {
            PkgEntry::Simple(n) => n,
            PkgEntry::Detailed(d) => &d.name,
        }
    }

    pub fn requires(&self) -> &[String] {
        match self {
            PkgEntry::Simple(_) => &[],
            PkgEntry::Detailed(d) => &d.requires,
        }
    }

    pub fn lock(&self) -> Option<&str> {
        match self {
            PkgEntry::Simple(_) => None,
            PkgEntry::Detailed(d) => d.lock.as_deref(),
        }
    }

    pub fn is_detailed(&self) -> bool {
        matches!(self, PkgEntry::Detailed(_))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Pip {
    /// Packages installed into the uv-managed python.
    pub packages: Vec<PkgEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GoPackages {
    /// Module paths with version suffix, e.g. `github.com/oklog/ulid/v2/cmd/ulid@latest`.
    pub packages: Vec<PkgEntry>,
}

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

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Brew {
    /// Third-party taps in `owner/repo` form. Trusted automatically unless `homebrew/*`.
    #[schemars(length(min = 0))]
    pub taps: Vec<String>,
    pub formulas: Vec<PkgEntry>,
    pub casks: Vec<PkgEntry>,
    /// DEPRECATED: transitional escape hatch for shell one-liners. Entries are
    /// migrated to typed backend/toolchain entries over time.
    #[serde(rename = "customCommands")]
    pub custom_commands: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Gem {
    pub rubygems: Vec<PkgEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Npm {
    pub global: NpmGlobal,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct NpmGlobal {
    pub packages: Vec<PkgEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Mas {
    pub apps: Vec<MasApp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MasApp {
    /// Numeric App Store product id (as string, mas takes strings).
    pub id: String,
    pub name: String,
    /// Unit IDs that must complete first, e.g. `["brew-formula:git"]`.
    /// See `crate::units` for the canonical `<prefix>:<name>` namespace.
    #[serde(default)]
    pub requires: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Directories to create (supports `~`/`$HOME`).
    pub mkdir: Vec<String>,
    pub symbolic_links: Vec<SymLink>,
    pub dockutil: Dockutil,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SymLink {
    pub from: LinkFrom,
    pub to: LinkTo,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkFrom {
    /// Path relative to the dotfiles repo root.
    pub relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkTo {
    /// Destination path (supports `~`/`$HOME`).
    pub absolute_path: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Dockutil {
    /// Destructive pre-steps applied before adding entries.
    #[serde(rename = "_before")]
    pub before: DockBefore,
    pub add: Vec<DockEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DockBefore {
    /// `defaults delete com.apple.dock && killall Dock` before rebuilding.
    pub reset: bool,
    /// `dockutil --remove all` before adding.
    #[serde(rename = "removeAll")]
    pub remove_all: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DockEntry {
    /// Absolute path of the .app bundle.
    pub app: String,
    /// Dock item to position this entry after (e.g. "Finder").
    pub after: Option<String>,
}
