use crate::gates::{battery_info, free_disk_gb};
use crate::paths::Paths;
use crate::report::{Environment, Report, StepReport};
use crate::steps::{self, PipelineEvent};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}
fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn run_id() -> String {
    let ts = chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
    format!("{}-{}", ts, std::process::id())
}

fn rtk_version() -> Option<String> {
    let out = Command::new("rtk").args(["--version"]).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    for token in s.split_whitespace() {
        let t = token.trim_matches(|c: char| !c.is_ascii_digit() && c != '.');
        if t.chars().filter(|&c| c=='.').count()==2 && t.chars().all(|c| c.is_ascii_digit() || c=='.') {
            return Some(t.to_string());
        }
    }
    // fallback: regex via manual
    let mut cur = String::new();
    let mut dots = 0;
    for c in s.chars() {
        if c.is_ascii_digit() { cur.push(c); }
        else if c=='.' { cur.push(c); dots+=1; if dots>2 { cur.clear(); dots=0; } }
        else {
            if dots==2 && !cur.is_empty() { return Some(cur); }
            cur.clear(); dots=0;
        }
    }
    if dots==2 && !cur.is_empty() { Some(cur) } else { None }
}

fn emit_step(name: &str, status: &str, duration: i64, updated: Value, failed: Value, note: &str, run_id: &str) -> StepReport {
    StepReport {
        name: name.to_string(),
        status: status.to_string(),
        duration_seconds: duration,
        updated,
        failed,
        note: note.to_string(),
        raw_log: format!("{}.{}.log", run_id, name),
    }
}

pub struct PipelineOptions {
    pub trigger: String,
    pub sudo_askpass: Option<PathBuf>,
    pub event_tx: Option<mpsc::Sender<PipelineEvent>>,
}

pub fn run_pipeline(paths: &Paths, opts: PipelineOptions) -> anyhow::Result<(Report, PathBuf)> {
    let run_id = run_id();
    let started_at = now_iso();
    let started_secs = now_secs();
    let battery = battery_info();
    let free_gb = free_disk_gb();

    let combined_log = paths.log_dir.join(format!("{}.combined.log", run_id));
    let _ = std::fs::write(&combined_log, "");
    let done_path = paths.log_dir.join(format!("{}.done", run_id));
    let _ = std::fs::remove_file(&done_path);

    // caffeinate
    let caff_child = Command::new("caffeinate").args(["-ims", "-w", &std::process::id().to_string()]).spawn().ok();

    let mut steps: Vec<StepReport> = vec![];

    let total_steps = 12;

    // helper to send StepStarted
    let send_started = |name: &str, idx: usize| {
        if let Some(tx) = &opts.event_tx {
            let _ = tx.send(PipelineEvent::StepStarted { name: name.to_string(), index: idx, total: total_steps });
        }
    };
    let send_finished = |report: StepReport| {
        if let Some(tx) = &opts.event_tx {
            let _ = tx.send(PipelineEvent::StepFinished { report: report.clone() });
        }
    };

    // 1 brew
    send_started("brew", 1);
    let rtk_before = rtk_version();
    let brew_outcome = steps::run_bash_step(
        "brew",
        "HOMEBREW_NO_COLOR=1 HOMEBREW_NO_ASK=1 brew update && HOMEBREW_NO_COLOR=1 HOMEBREW_NO_ASK=1 brew upgrade -y && HOMEBREW_NO_COLOR=1 HOMEBREW_NO_ASK=1 brew upgrade --cask --greedy=false -y && brew autoremove && brew cleanup",
        &paths.log_dir, &run_id, &combined_log, opts.event_tx.clone(), opts.sudo_askpass.as_deref()
    );
    let brew_log = paths.log_dir.join(format!("{}.brew.log", run_id));
    let updated = steps::parse_brew_upgraded(&brew_log);
    let brew_report = emit_step("brew", &brew_outcome.report.status, brew_outcome.report.duration_seconds, updated, Value::Array(vec![]), &brew_outcome.report.note, &run_id);
    send_finished(brew_report.clone());
    steps.push(brew_report);

    // rtk-repatch
    send_started("rtk-repatch", 2);
    let rtk_after = rtk_version();
    let rtk_changed = match (&rtk_before, &rtk_after) {
        (Some(b), Some(a)) if b != a => true,
        _ => false,
    };
    let rtk_report = if !has_command("rtk") {
        emit_step("rtk-repatch", "skipped", 0, Value::Array(vec![]), Value::Array(vec![]), "rtk not installed", &run_id)
    } else if !rtk_changed {
        emit_step("rtk-repatch", "skipped", 0, Value::Array(vec![]), Value::Array(vec![]), "rtk version unchanged", &run_id)
    } else {
        let o = steps::run_step("rtk-repatch", "rtk", &["init", "-g", "--opencode", "--auto-patch"], &paths.log_dir, &run_id, &combined_log, opts.event_tx.clone(), opts.sudo_askpass.as_deref());
        let status = if o.exit_code==0 { "success" } else { "failed" };
        let note = if o.exit_code==0 { "opencode re-patched".to_string() } else { format!("rtk init exited {}", o.exit_code) };
        emit_step("rtk-repatch", status, o.report.duration_seconds, Value::Array(vec![]), Value::Array(vec![]), &note, &run_id)
    };
    send_finished(rtk_report.clone());
    steps.push(rtk_report);

    // 3 mas
    send_started("mas", 3);
    let mas_report = if !has_command("mas") {
        emit_step("mas", "skipped", 0, Value::Array(vec![]), Value::Array(vec![]), "mas not installed", &run_id)
    } else {
        // Check mas outdated before
        let before_out = Command::new("mas").arg("outdated").output();
        let before = before_out.map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        // Heuristic: if mas outdated fails, assume session unavailable (bash checks exit code)
        let before_success = Command::new("mas").arg("outdated").output().map(|o| o.status.success()).unwrap_or(false);
        if !before_success {
            emit_step("mas", "skipped", 0, Value::Array(vec![]), Value::Array(vec![]), "App Store session unavailable", &run_id)
        } else {
            let o = steps::run_step("mas", "mas", &["upgrade"], &paths.log_dir, &run_id, &combined_log, opts.event_tx.clone(), opts.sudo_askpass.as_deref());
            let after = Command::new("mas").arg("outdated").output().map(|p| String::from_utf8_lossy(&p.stdout).to_string()).unwrap_or_default();
            // diff logic: updated = before - after by name
            let before_names: std::collections::HashSet<String> = before.lines().map(|l| {
                let without_id = l.splitn(2, char::is_whitespace).nth(1).unwrap_or(l).trim();
                without_id.split(" (").next().unwrap_or(without_id).trim().to_string()
            }).collect();
            let after_names: std::collections::HashSet<String> = after.lines().map(|l| {
                let without_id = l.splitn(2, char::is_whitespace).nth(1).unwrap_or(l).trim();
                without_id.split(" (").next().unwrap_or(without_id).trim().to_string()
            }).collect();
            let updated_names: Vec<Value> = before_names.difference(&after_names).map(|n| serde_json::json!({"name": n})).collect();
            let status = if o.exit_code==0 { "success" } else { "failed" };
            let note = if o.exit_code==0 { String::new() } else { format!("mas exited {}", o.exit_code) };
            emit_step("mas", status, o.report.duration_seconds, Value::Array(updated_names), Value::Array(vec![]), &note, &run_id)
        }
    };
    send_finished(mas_report.clone());
    steps.push(mas_report);

    // 4 rust
    send_started("rust", 4);
    let rust_report = if !has_command("rustup") {
        emit_step("rust", "skipped", 0, Value::Array(vec![]), Value::Array(vec![]), "rustup not installed", &run_id)
    } else {
        let o = steps::run_step("rust", "rustup", &["update"], &paths.log_dir, &run_id, &combined_log, opts.event_tx.clone(), opts.sudo_askpass.as_deref());
        if o.exit_code != 0 {
            emit_step("rust", "failed", o.report.duration_seconds, Value::Array(vec![]), Value::Array(vec![]), &format!("rustup exited {}", o.exit_code), &run_id)
        } else {
            // check cargo install-update
            let has_cargo_update = Command::new("cargo").args(["install-update", "--list"]).output().map(|out| out.status.success()).unwrap_or(false);
            if has_cargo_update {
                let cargo_log = paths.log_dir.join(format!("{}.rust-cargo.log", run_id));
                let start = std::time::Instant::now();
                let cargo_out = Command::new("cargo").args(["install-update", "-a"]).output();
                let dur = start.elapsed().as_secs() as i64;
                match cargo_out {
                    Ok(out) if out.status.success() => {
                        let s = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
                        let _ = std::fs::write(&cargo_log, &s);
                        let updated: Vec<Value> = s.lines().filter(|l| l.contains("Updating ")).map(|l| {
                            let name = l.split_whitespace().nth(1).unwrap_or("").to_string();
                            serde_json::json!({"name": name})
                        }).collect();
                        emit_step("rust", "success", dur, Value::Array(updated), Value::Array(vec![]), "", &run_id)
                    }
                    Ok(out) => {
                        let s = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
                        let _ = std::fs::write(&cargo_log, s);
                        emit_step("rust", "failed", dur, Value::Array(vec![]), Value::Array(vec![]), &format!("cargo install-update exited {}", out.status.code().unwrap_or(1)), &run_id)
                    }
                    Err(e) => emit_step("rust", "failed", 0, Value::Array(vec![]), Value::Array(vec![]), &format!("cargo install-update failed: {}", e), &run_id),
                }
            } else {
                emit_step("rust", "success", o.report.duration_seconds, Value::Array(vec![]), Value::Array(vec![]), "cargo-update not installed, cargo globals skipped", &run_id)
            }
        }
    };
    send_finished(rust_report.clone());
    steps.push(rust_report);

    // 5 php / composer
    send_started("php", 5);
    let php_report = if !has_command("composer") {
        emit_step("php", "skipped", 0, Value::Array(vec![]), Value::Array(vec![]), "composer not installed", &run_id)
    } else {
        let o = steps::run_step("php", "composer", &["global", "update", "--no-interaction"], &paths.log_dir, &run_id, &combined_log, opts.event_tx.clone(), opts.sudo_askpass.as_deref());
        let status = if o.exit_code==0 { "success" } else { "failed" };
        let note = if o.exit_code==0 { String::new() } else { format!("composer exited {}", o.exit_code) };
        emit_step("php", status, o.report.duration_seconds, Value::Array(vec![]), Value::Array(vec![]), &note, &run_id)
    };
    send_finished(php_report.clone());
    steps.push(php_report);

    // 6 node-fn
    send_started("node-fn", 6);
    let node_report = if !has_command("fnm") {
        emit_step("node-fn", "skipped", 0, Value::Array(vec![]), Value::Array(vec![]), "fnm not installed", &run_id)
    } else {
        let old_default = Command::new("fnm").arg("current").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        let npm_globals = Command::new("npm").args(["ls", "-g", "--depth=0", "--json"]).output().map(|o| {
            let s = String::from_utf8_lossy(&o.stdout);
            if let Ok(v) = serde_json::from_str::<Value>(&s) {
                if let Some(deps) = v.get("dependencies").and_then(|d| d.as_object()) {
                    deps.keys().cloned().collect::<Vec<_>>().join(" ")
                } else { "".into() }
            } else { "".into() }
        }).unwrap_or_default();
        let logf = paths.log_dir.join(format!("{}.node-fn.log", run_id));
        let start = std::time::Instant::now();
        // Run via bash to get env handling; replicate bash step_node logic
        let script = format!(
            "eval \"$(fnm env 2>/dev/null)\"; fnm install --lts; fnm default lts-latest; eval \"$(fnm env 2>/dev/null)\"; fnm use lts-latest; if [ -n \"{}\" ]; then npm install -g {}; fi",
            npm_globals, npm_globals
        );
        let out = Command::new("bash").args(["-c", &script]).output();
        let dur = start.elapsed().as_secs() as i64;
        match out {
            Ok(o) => {
                let s = String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr);
                let _ = std::fs::write(&logf, &s);
                if o.status.success() {
                    let new_default = Command::new("fnm").arg("current").output().map(|p| String::from_utf8_lossy(&p.stdout).trim().to_string()).unwrap_or_default();
                    let mut updated = vec![];
                    if !new_default.is_empty() && new_default != old_default && new_default != "default" && new_default != "system" {
                        updated.push(serde_json::json!({"name":"node","from":old_default, "to": new_default}));
                    }
                    // prune old versions
                    if let Ok(list_out) = Command::new("fnm").arg("list").output() {
                        let ls = String::from_utf8_lossy(&list_out.stdout);
                        let mut versions = vec![];
                        for token in ls.split_whitespace() {
                            if token.starts_with('v') && token.chars().filter(|&c| c=='.').count()>=2 {
                                versions.push(token.to_string());
                            }
                        }
                        // dedup sort
                        versions.sort();
                        versions.dedup();
                        for v in versions {
                            if v != new_default && v != old_default {
                                let _ = Command::new("fnm").args(["uninstall", &v]).output();
                                // we don't track success deeply
                            }
                        }
                    }
                    emit_step("node-fn", "success", dur, Value::Array(updated), Value::Array(vec![]), "", &run_id)
                } else {
                    emit_step("node-fn", "failed", dur, Value::Array(vec![]), Value::Array(vec![]), &format!("fnm/npm exited {}", o.status.code().unwrap_or(1)), &run_id)
                }
            }
            Err(e) => emit_step("node-fn", "failed", dur, Value::Array(vec![]), Value::Array(vec![]), &format!("fnm failed: {}", e), &run_id),
        }
    };
    send_finished(node_report.clone());
    steps.push(node_report);

    // 7 python-uv
    send_started("python-uv", 7);
    let py_report = if !has_command("uv") {
        emit_step("python-uv", "skipped", 0, Value::Array(vec![]), Value::Array(vec![]), "uv not installed", &run_id)
    } else {
        let script = "uv self update || true; uv python install; uv pip install --system --break-system-packages -U --python \"$(uv python find)\" pynvim neovim";
        let o = steps::run_bash_step("python-uv", script, &paths.log_dir, &run_id, &combined_log, opts.event_tx.clone(), opts.sudo_askpass.as_deref());
        let status = if o.exit_code==0 { "success" } else { "failed" };
        let note = if o.exit_code==0 { "uv self-update failure ignored (brew-managed)".to_string() } else { format!("uv exited {}", o.exit_code) };
        emit_step("python-uv", status, o.report.duration_seconds, Value::Array(vec![]), Value::Array(vec![]), &note, &run_id)
    };
    send_finished(py_report.clone());
    steps.push(py_report);

    // 8 opencode
    send_started("opencode", 8);
    let opencode_report = if !has_command("opencode") {
        emit_step("opencode", "skipped", 0, Value::Array(vec![]), Value::Array(vec![]), "opencode not installed", &run_id)
    } else {
        let o = steps::run_step("opencode", "opencode", &["upgrade"], &paths.log_dir, &run_id, &combined_log, opts.event_tx.clone(), opts.sudo_askpass.as_deref());
        let log_content = std::fs::read_to_string(paths.log_dir.join(format!("{}.opencode.log", run_id))).unwrap_or_default();
        let updated = if o.exit_code==0 && (log_content.to_lowercase().contains("up to date") || log_content.to_lowercase().contains("already")) {
            Value::Array(vec![])
        } else if o.exit_code==0 {
            serde_json::json!([{"name":"opencode"}])
        } else {
            Value::Array(vec![])
        };
        let status = if o.exit_code==0 { "success" } else { "failed" };
        let note = if o.exit_code==0 { String::new() } else { format!("opencode upgrade exited {}", o.exit_code) };
        emit_step("opencode", status, o.report.duration_seconds, updated, Value::Array(vec![]), &note, &run_id)
    };
    send_finished(opencode_report.clone());
    steps.push(opencode_report);

    // 9 neovim-plugins
    send_started("neovim-plugins", 9);
    let nvim_report = if !has_command("nvim") {
        emit_step("neovim-plugins", "skipped", 0, Value::Array(vec![]), Value::Array(vec![]), "nvim not installed", &run_id)
    } else {
        let o = steps::run_step("neovim-plugins", "nvim", &["--headless", "+PlugUpdate", "+qa"], &paths.log_dir, &run_id, &combined_log, opts.event_tx.clone(), opts.sudo_askpass.as_deref());
        let status = if o.exit_code==0 { "success" } else { "failed" };
        let note = if o.exit_code==0 { String::new() } else { format!("nvim exited {}", o.exit_code) };
        emit_step("neovim-plugins", status, o.report.duration_seconds, Value::Array(vec![]), Value::Array(vec![]), &note, &run_id)
    };
    send_finished(nvim_report.clone());
    steps.push(nvim_report);

    // 10 gem
    send_started("gem", 10);
    let gem_report = if !has_command("gem") {
        emit_step("gem", "skipped", 0, Value::Array(vec![]), Value::Array(vec![]), "gem not installed", &run_id)
    } else {
        let o = steps::run_step("gem", "gem", &["update", "neovim", "--no-document"], &paths.log_dir, &run_id, &combined_log, opts.event_tx.clone(), opts.sudo_askpass.as_deref());
        let status = if o.exit_code==0 { "success" } else { "failed" };
        let note = if o.exit_code==0 { String::new() } else { format!("gem exited {}", o.exit_code) };
        emit_step("gem", status, o.report.duration_seconds, Value::Array(vec![]), Value::Array(vec![]), &note, &run_id)
    };
    send_finished(gem_report.clone());
    steps.push(gem_report);

    // 11 tmux-tpm
    send_started("tmux-tpm", 11);
    let tpm_bin = PathBuf::from("/opt/homebrew/opt/tpm/share/tpm/bin/update_plugins");
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    let tpm_report = if !tpm_bin.exists() || !home.join(".tmux/plugins").is_dir() {
        emit_step("tmux-tpm", "skipped", 0, Value::Array(vec![]), Value::Array(vec![]), "TPM or ~/.tmux/plugins not present", &run_id)
    } else {
        let o = steps::run_step("tmux-tpm", tpm_bin.to_str().unwrap(), &["all"], &paths.log_dir, &run_id, &combined_log, opts.event_tx.clone(), opts.sudo_askpass.as_deref());
        let status = if o.exit_code==0 { "success" } else { "failed" };
        let note = if o.exit_code==0 { String::new() } else { format!("update_plugins exited {}", o.exit_code) };
        emit_step("tmux-tpm", status, o.report.duration_seconds, Value::Array(vec![]), Value::Array(vec![]), &note, &run_id)
    };
    send_finished(tpm_report.clone());
    steps.push(tpm_report);

    // 12 macos
    send_started("macos", 12);
    let sw_out = Command::new("softwareupdate").arg("--list").output().map(|o| String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr)).unwrap_or_default();
    let sw_cnt = sw_out.lines().filter(|l| l.trim().starts_with("* Label:")).count();
    let note = if sw_cnt > 0 { format!("{} macOS updates pending (install manually via System Settings)", sw_cnt) } else { "no macOS updates pending".into() };
    // notify if pending — via notify-rust? pipeline does not notify directly; UI will.
    // But keep behavior: if macos pending, we could emit via PipelineEvent? For now just note.
    let macos_report = emit_step("macos", "success", 0, Value::Array(vec![]), Value::Array(vec![]), &note, &run_id);
    send_finished(macos_report.clone());
    steps.push(macos_report);

    // audit brew deprecated
    let brew_deprecated = steps::brew_deprecated_json();
    let composer_audit = steps::composer_audit_json();

    let _ = std::fs::write(&done_path, "");

    if let Some(mut child) = caff_child {
        let _ = child.kill();
    }

    let finished_at = now_iso();
    let duration = now_secs() - started_secs;
    let status = if steps.iter().any(|s| s.status=="failed") { "partial" } else { "success" };

    let report = Report {
        schema: "dotfiles-updater@1".into(),
        run_id: run_id.clone(),
        trigger: opts.trigger.clone(),
        started_at,
        finished_at,
        duration_seconds: duration,
        status: status.to_string(),
        environment: Environment { on_ac_power: battery.on_ac, battery_pct: battery.battery_pct, free_disk_gb: free_gb },
        steps: steps.clone(),
        audit: serde_json::json!({"brew_deprecated": brew_deprecated, "composer": composer_audit}),
    };

    let report_path = paths.log_dir.join(format!("{}.json", run_id));
    report.write(&report_path)?;

    // update state
    {
        let mut state = crate::state::State::load(&paths.state_file).unwrap_or_default();
        state.last_attempt_at = Some(now_secs());
        if status=="success" {
            state.last_success_at = Some(now_secs());
            state.last_failed_steps = vec![];
            state.last_outcome = Some("success".into());
        } else {
            let failed_names: Vec<String> = steps.iter().filter(|s| s.status=="failed").map(|s| s.name.clone()).collect();
            state.last_failed_steps = failed_names;
            state.last_outcome = Some("partial".into());
        }
        let _ = state.save(&paths.state_file);
    }

    if let Some(tx) = &opts.event_tx {
        let _ = tx.send(PipelineEvent::RunFinished { status: status.to_string(), report_path: report_path.clone() });
    }

    Ok((report, report_path))
}

fn has_command(name: &str) -> bool {
    Command::new("which").arg(name).output().map(|o| o.status.success()).unwrap_or(false)
}
