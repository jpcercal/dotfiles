use clap::{Parser, Subcommand};

mod askpass;
mod headless;
mod notify;
mod ui_egui;
mod ui_theme;
mod upgrade;

#[derive(Parser)]
#[command(name = "dotfiles", version, about = "dotfiles CLI — single binary for all dotfiles operations")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run system upgrades (brew, mas, rust, node, python, etc.)
    Upgrade(upgrade::UpgradeArgs),
    /// Hidden SUDO_ASKPASS helper
    #[command(hide = true)]
    __Askpass(askpass::AskpassArgs),
}

fn main() -> anyhow::Result<()> {
    ensure_path();
    let cli = Cli::parse();
    match cli.command {
        Commands::Upgrade(args) => upgrade::run(args),
        Commands::__Askpass(args) => askpass::run(args),
    }
}

fn ensure_path() {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    let extra = vec![
        home.join(".local/bin"),
        std::path::PathBuf::from("/opt/homebrew/bin"),
        std::path::PathBuf::from("/opt/homebrew/sbin"),
        home.join("dotfiles/bin"),
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
