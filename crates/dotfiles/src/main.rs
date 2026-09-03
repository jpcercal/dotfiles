use clap::{Parser, Subcommand};

mod agent;
mod apply;
mod askpass;
mod bootstrap;
mod cache;
mod completion;
mod ctx;
mod doctor;
mod history;
#[cfg(feature = "gui")]
mod notify;
mod pkg;
mod prefs_cmd;
mod schema;
mod software_update;
mod sync;
#[cfg(feature = "gui")]
mod ui_egui;
#[cfg(feature = "gui")]
mod ui_theme;
mod upgrade;

#[derive(Parser)]
#[command(
    name = "dotfiles",
    version,
    about = "dotfiles — universal macOS package & configuration manager"
)]
struct Cli {
    /// Print what would run without changing anything
    #[arg(long, global = true)]
    dry_run: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install packages (no args = everything in the manifest)
    Install(pkg::InstallArgs),
    /// Remove packages
    Remove(pkg::RemoveArgs),
    /// Search packages across all backends
    Search(pkg::SearchArgs),
    /// List installed/outdated packages
    List(pkg::ListArgs),
    /// Show package info
    Info(pkg::InfoArgs),
    /// Refresh package metadata (brew update & friends)
    Update(pkg::UpdateArgs),
    /// Run system upgrades (brew, mas, rust, node, python, etc.)
    Upgrade(upgrade::UpgradeArgs),
    /// Bootstrap the machine itself (Homebrew, taps)
    Bootstrap(bootstrap::BootstrapArgs),
    /// Diagnose the environment (brew, shell, PATH, manifest)
    Doctor,
    /// Apply configuration: dirs, symlinks, dock, shell, nvim plugins
    Apply(apply::ApplyArgs),
    /// Declarative macOS preferences (defaults/pmset/dock/login-items)
    Prefs(prefs_cmd::PrefsArgs),
    /// Seed shell history (atuin) from commands.yaml
    History(history::HistoryArgs),
    /// macOS system update (manual only — reboots the machine!)
    SoftwareUpdate(software_update::SoftwareUpdateArgs),
    /// Run the full pipeline (bootstrap → install → apply → prefs → history)
    Sync(sync::SyncArgs),
    /// Manage the LaunchAgent (scheduled upgrades)
    Agent(agent::AgentArgs),
    /// Cache maintenance (user + system caches)
    Cache(cache::CacheArgs),
    /// Print the apps.yaml JSON Schema
    Schema(schema::SchemaArgs),
    /// Generate shell completions
    Completion(completion::CompletionArgs),
    /// Hidden SUDO_ASKPASS helper
    #[command(hide = true)]
    #[command(name = "__askpass")]
    Askpass(askpass::AskpassArgs),
}

fn main() -> anyhow::Result<()> {
    ensure_path();
    let cli = Cli::parse();
    let ctx = ctx::Ctx::real(cli.dry_run);
    match cli.command {
        Commands::Install(args) => pkg::install(&ctx, args),
        Commands::Remove(args) => pkg::remove(&ctx, args),
        Commands::Search(args) => pkg::search(&ctx, args),
        Commands::List(args) => pkg::list(&ctx, args),
        Commands::Info(args) => pkg::info(&ctx, args),
        Commands::Update(args) => pkg::update(&ctx, args),
        Commands::Upgrade(args) => upgrade::run(args),
        Commands::Bootstrap(args) => bootstrap::run(&ctx, args),
        Commands::Doctor => doctor::run(&ctx),
        Commands::Apply(args) => apply::run(&ctx, args),
        Commands::Prefs(args) => prefs_cmd::run(&ctx, args),
        Commands::History(args) => history::run(&ctx, args),
        Commands::SoftwareUpdate(args) => software_update::run(&ctx, args),
        Commands::Sync(args) => sync::run(&ctx, args),
        Commands::Agent(args) => agent::run(&ctx, args),
        Commands::Cache(args) => cache::run(&ctx, args),
        Commands::Schema(args) => schema::run(args),
        Commands::Completion(args) => completion::run(args),
        Commands::Askpass(args) => askpass::run(args),
    }
}

fn ensure_path() {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    let extra = [
        home.join(".local/bin"),
        std::path::PathBuf::from("/opt/homebrew/bin"),
        std::path::PathBuf::from("/opt/homebrew/sbin"),
        home.join(".cargo/bin"),
        home.join(".opencode/bin"),
        std::path::PathBuf::from("/usr/local/bin"),
        std::path::PathBuf::from("/usr/bin"),
        std::path::PathBuf::from("/bin"),
    ];
    let current = std::env::var("PATH").unwrap_or_default();
    let mut parts: Vec<String> = current.split(':').map(|s| s.to_string()).collect();
    for p in extra.iter().rev() {
        let s = p.to_string_lossy().to_string();
        if !parts.contains(&s) {
            parts.insert(0, s);
        }
    }
    let new_path = parts.join(":");
    unsafe { std::env::set_var("PATH", new_path) };
}

/// Compile-time wiring check for clap_complete.
pub fn cli_for_completion() -> clap::Command {
    use clap::CommandFactory;
    Cli::command()
}
