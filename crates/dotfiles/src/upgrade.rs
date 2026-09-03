use clap::Parser;
use dotfiles_core::gates;
use dotfiles_core::paths::Paths;
use dotfiles_core::pipeline::{run_pipeline, PipelineOptions};
use dotfiles_core::probes;
use dotfiles_core::state::State;
use dotfiles_core::steps::PipelineEvent;
use std::path::PathBuf;
use std::sync::mpsc;

#[derive(Parser, Debug, Clone)]
pub struct UpgradeArgs {
    /// LaunchAgent tick: silent pre-flight checks; shows dialog at most once per 24h
    #[arg(long)]
    pub gate: bool,

    /// Manual run in a terminal (default): gates print reasons and ask before proceeding
    #[arg(long)]
    pub foreground: bool,

    /// Show gate results, dialog content and report skeleton only
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Run pipeline without GUI, emitting JSON-lines on stdout
    #[arg(long)]
    pub headless: bool,
}

pub fn askpass_socket_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".local/state/dotfiles-updater/askpass.sock")
}

pub fn askpass_wrapper_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let home = dirs::home_dir()?;
    let dir = home.join(".local/state/dotfiles-updater");
    let _ = std::fs::create_dir_all(&dir);
    let wrapper = dir.join("askpass-wrapper.sh");
    let content = format!("#!/bin/sh\nexec \"{}\" __askpass \"$@\"\n", exe.display());
    if std::fs::write(&wrapper, content).is_ok() {
        let _ = std::process::Command::new("chmod")
            .args(["+x", wrapper.to_str()?])
            .output();
        Some(wrapper)
    } else {
        None
    }
}

fn retention_cleanup(log_dir: &std::path::Path) {
    if let Ok(entries) = std::fs::read_dir(log_dir) {
        let cutoff = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(90 * 86400))
            .unwrap_or(std::time::UNIX_EPOCH);
        for e in entries.flatten() {
            if let Ok(meta) = e.metadata() {
                if let Ok(mtime) = meta.modified() {
                    if mtime < cutoff {
                        if let Some(name) = e.file_name().to_str() {
                            if name.ends_with(".json") {
                                let _ = std::fs::remove_file(e.path());
                            }
                        }
                    }
                }
            }
        }
    }
}

fn print_gate(g: &gates::GateResult) {
    println!("  {:12} {}", format!("{}:", g.name), g.reason);
}

pub fn run(args: UpgradeArgs) -> anyhow::Result<()> {
    let paths = Paths::detect();
    paths.ensure_dirs()?;

    if !args.dry_run {
        retention_cleanup(&paths.log_dir);
        // init state if missing
        let _ = State::init_if_missing(&paths.state_file);
    }

    if args.dry_run {
        return dry_run(&paths);
    }

    if args.headless {
        let trigger = if args.gate { "gate" } else { "headless" };
        return headless_run(&paths, trigger);
    }

    if args.gate {
        return gate_run(&paths);
    }

    // default: foreground GUI
    foreground_run(&paths)
}

fn dry_run(paths: &Paths) -> anyhow::Result<()> {
    let battery = gates::battery_info();
    let free_gb = gates::free_disk_gb();
    let state = State::load(&paths.state_file).unwrap_or_default();

    println!("\ndry-run: no state, lock or filesystem mutation\n");
    println!("Gates:");
    print_gate(&gates::gate_power(&battery));
    print_gate(&gates::gate_network());
    print_gate(&gates::gate_disk(free_gb));
    print_gate(&gates::gate_pkgmgr());
    print_gate(&gates::gate_schedule(&state));
    print_gate(&gates::gate_dialog_cooldown(&state));
    println!();

    println!("Dialog preview:");
    println!("--------------------------------------------------------------------------------");
    let sections = probes::probe_all();
    let summary = probes::summary_text(&sections);
    println!("{}", summary);
    println!("--------------------------------------------------------------------------------");
    println!();

    let started_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let run_id = format!(
        "{}-{}",
        chrono::Local::now().format("%Y%m%d%H%M%S"),
        std::process::id()
    );
    let skeleton = serde_json::json!({
        "schema": "dotfiles-updater@1",
        "run_id": run_id,
        "trigger": "dry_run",
        "started_at": started_at,
        "status": "pending",
        "environment": {
            "on_ac_power": battery.on_ac,
            "battery_pct": battery.battery_pct,
            "free_disk_gb": free_gb,
        },
        "steps": [],
        "audit": {"brew_deprecated": [], "composer": null}
    });
    println!("Report skeleton:");
    println!("{}", serde_json::to_string_pretty(&skeleton)?);
    Ok(())
}

fn headless_run(paths: &Paths, trigger: &str) -> anyhow::Result<()> {
    let lock = dotfiles_core::lock::FileLock::acquire(&paths.lock_dir)?;
    if lock.is_none() {
        anyhow::bail!(
            "another run in progress (lock: {})",
            paths.lock_dir.display()
        );
    }
    let _lock = lock.unwrap();

    // For headless, we bypass consent and run pipeline directly
    let askpass = askpass_wrapper_path();
    let (tx, rx) = mpsc::channel::<PipelineEvent>();
    let print_handle = std::thread::spawn(move || {
        for ev in rx {
            match ev {
                PipelineEvent::StepStarted { name, index, total } => {
                    eprintln!("▶ step {}/{}: {}", index, total, name);
                }
                PipelineEvent::LogLine { line, .. } => {
                    println!("{}", line);
                }
                PipelineEvent::StepFinished { report } => {
                    eprintln!("  {} -> {}", report.name, report.status);
                }
                PipelineEvent::RunFinished {
                    status,
                    report_path,
                } => {
                    eprintln!("finished: {} report: {}", status, report_path.display());
                }
                PipelineEvent::SudoPrompt {
                    command,
                    reason,
                    respond,
                } => {
                    eprint!("Sudo required for {} ({}): ", command, reason);
                    let _ = std::io::Write::flush(&mut std::io::stderr());
                    let mut pw = String::new();
                    let _ = std::io::stdin().read_line(&mut pw);
                    let _ = respond.send(pw.trim().to_string());
                }
            }
        }
    });

    let opts = PipelineOptions {
        trigger: trigger.to_string(),
        sudo_askpass: askpass,
        event_tx: Some(tx),
    };
    let (report, path) = run_pipeline(paths, opts)?;
    // give printer time
    std::thread::sleep(std::time::Duration::from_millis(200));
    drop(print_handle);
    println!("{}", serde_json::to_string_pretty(&report)?);
    eprintln!("report: {}", path.display());
    Ok(())
}

/// Open the unified consent→progress window; without the `gui` feature, fall
/// back to the terminal JSON-lines headless run.
fn open_upgrade_window(paths: &Paths, mode: &str) -> anyhow::Result<()> {
    #[cfg(feature = "gui")]
    {
        crate::ui_egui::run_upgrade_window(paths, mode)
    }
    #[cfg(not(feature = "gui"))]
    {
        eprintln!("(no GUI support in this build — running headless)");
        headless_run(paths, mode)
    }
}

fn gate_run(paths: &Paths) -> anyhow::Result<()> {
    let state = State::load(&paths.state_file).unwrap_or_default();

    // schedule gate
    let sched = gates::gate_schedule(&state);
    if !sched.ok {
        eprintln!("{}", sched.reason);
        return Ok(());
    }

    let lock = dotfiles_core::lock::FileLock::acquire(&paths.lock_dir)?;
    if lock.is_none() {
        eprintln!("another run in progress, exiting");
        return Ok(());
    }
    let _lock = lock.unwrap();

    // env gates
    let battery = gates::battery_info();
    let free_gb = gates::free_disk_gb();
    for g in [
        gates::gate_power(&battery),
        gates::gate_network(),
        gates::gate_disk(free_gb),
        gates::gate_pkgmgr(),
    ] {
        eprintln!("gate {}: {}", g.name, g.reason);
        if !g.ok {
            return Ok(());
        }
    }

    let cooldown = gates::gate_dialog_cooldown(&state);
    if !cooldown.ok {
        eprintln!("{}", cooldown.reason);
        return Ok(());
    }

    // Single-window consent → progress (fixes macOS “Choose Application” + hang from two run_natives)
    open_upgrade_window(paths, "gate")
}

fn foreground_run(paths: &Paths) -> anyhow::Result<()> {
    let lock = dotfiles_core::lock::FileLock::acquire(&paths.lock_dir)?;
    if lock.is_none() {
        anyhow::bail!(
            "another run is in progress (lock: {})",
            paths.lock_dir.display()
        );
    }
    let _lock = lock.unwrap();

    let battery = gates::battery_info();
    let free_gb = gates::free_disk_gb();
    let mut failed_gates = vec![];
    for g in [
        gates::gate_power(&battery),
        gates::gate_network(),
        gates::gate_disk(free_gb),
        gates::gate_pkgmgr(),
    ] {
        if !g.ok {
            failed_gates.push(g);
        }
    }

    if !failed_gates.is_empty() {
        for g in &failed_gates {
            eprintln!("gate {}: {}", g.name, g.reason);
        }
        // In GUI mode we show these in the header; we still solicit consent.
        // In terminal, ask Y/n? But since we are now GUI-first, we just proceed to GUI
        // and let the GUI header show gate warnings. If stdin is a tty and user wants
        // to abort, they can postpone.
        // For backwards compat when run in a pure terminal without display, fallback:
        if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
            // Check if we have a GUI session (macOS always has one if launched from terminal)
            // On macOS, we can still show GUI even without DISPLAY.
            // So only prompt if we truly are headless.
            if !has_gui() {
                eprint!("Some pre-flight gates failed. Proceed anyway? [y/N] ");
                use std::io::{self, Write};
                io::stdout().flush()?;
                let mut line = String::new();
                io::stdin().read_line(&mut line)?;
                if line.trim().to_lowercase() != "y" {
                    eprintln!("aborted");
                    return Ok(());
                }
            }
        }
    }

    // Single-window consent → progress (same fix)
    open_upgrade_window(paths, "foreground")
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Consent {
    Proceed,
    Postpone,
}

fn has_gui() -> bool {
    // On macOS, GUI is available if we can connect to WindowServer.
    // eframe will succeed if so. For a quick check, see if we're in a gui login session.
    // Simplest: try to see if launchctl gui domain exists or just return true on macOS.
    #[cfg(target_os = "macos")]
    return true;
    #[cfg(not(target_os = "macos"))]
    return std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok();
}

#[cfg(feature = "gui")]
#[allow(dead_code)]
fn solicit_consent_gui(
    paths: &Paths,
    summary: &str,
    sections: &[probes::Section],
) -> anyhow::Result<Consent> {
    // Stamp last_dialog_at BEFORE showing (covers dismiss/locked-screen)
    {
        let mut s = State::load(&paths.state_file).unwrap_or_default();
        s.last_dialog_at = Some(chrono::Utc::now().timestamp());
        s.save(&paths.state_file)?;
    }

    if !has_gui() {
        // Fallback to terminal prompt
        eprint!("Proceed? [Y/n] ");
        use std::io::{self, Write};
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        if line.trim().to_lowercase() == "n" {
            return Ok(Consent::Postpone);
        } else {
            return Ok(Consent::Proceed);
        }
    }

    // Launch egui consent window — it blocks until user clicks
    let consent = crate::ui_egui::show_consent(summary, sections, paths)?;
    Ok(consent)
}

#[cfg(feature = "gui")]
#[allow(dead_code)]
fn run_pipeline_with_gui(paths: &Paths, trigger: &str) -> anyhow::Result<()> {
    // This will open the egui progress window and run the pipeline in a background thread,
    // streaming LogLine events to the UI.
    crate::ui_egui::run_progress(paths, trigger)
}
