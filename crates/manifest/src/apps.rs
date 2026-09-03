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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Pip {
    /// Packages installed into the uv-managed python.
    pub packages: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GoPackages {
    /// Module paths with version suffix, e.g. `github.com/oklog/ulid/v2/cmd/ulid@latest`.
    pub packages: Vec<String>,
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
    pub formulas: Vec<String>,
    pub casks: Vec<String>,
    /// DEPRECATED: transitional escape hatch for shell one-liners. Entries are
    /// migrated to typed backend/toolchain entries over time.
    #[serde(rename = "customCommands")]
    pub custom_commands: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Gem {
    pub rubygems: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Npm {
    pub global: NpmGlobal,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct NpmGlobal {
    pub packages: Vec<String>,
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
