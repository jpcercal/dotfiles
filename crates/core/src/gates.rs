use crate::state::State;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub const MIN_BATTERY: i32 = 50;
pub const MIN_DISK_GB: i64 = 10;
pub const CADENCE_SECS: i64 = 86400;
pub const DIALOG_COOLDOWN_SECS: i64 = 86400;

#[derive(Debug, Clone)]
pub struct GateResult {
    pub name: &'static str,
    pub ok: bool,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct PowerInfo {
    pub on_ac: bool,
    pub battery_pct: i32,
}

pub fn battery_info() -> PowerInfo {
    let mut on_ac = false;
    let mut battery_pct: i32 = 100;

    if let Ok(out) = Command::new("pmset").args(["-g", "batt"]).output() {
        let s = String::from_utf8_lossy(&out.stdout);
        if s.contains("AC Power") {
            on_ac = true;
        }
        // find NN%
        for token in s.split_whitespace() {
            if token.ends_with('%') {
                if let Ok(v) = token.trim_end_matches('%').parse::<i32>() {
                    battery_pct = v;
                    break;
                }
            }
        }
    }

    if let Ok(v) = std::env::var("DFU_ON_AC") {
        on_ac = v == "1";
    }
    if let Ok(v) = std::env::var("DFU_BATTERY_PCT") {
        if let Ok(p) = v.parse::<i32>() {
            battery_pct = p;
        }
    }

    PowerInfo { on_ac, battery_pct }
}

pub fn free_disk_gb() -> i64 {
    // df -g /
    if let Ok(out) = Command::new("df").args(["-g", "/"]).output() {
        let s = String::from_utf8_lossy(&out.stdout);
        for line in s.lines().skip(1) {
            let cols: Vec<_> = line.split_whitespace().collect();
            if cols.len() >= 4 {
                if let Ok(v) = cols[3].parse::<i64>() {
                    return v;
                }
            }
        }
    }
    0
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn gate_power(info: &PowerInfo) -> GateResult {
    if info.on_ac {
        GateResult {
            name: "power",
            ok: true,
            reason: format!("ok: on AC power (battery {}%)", info.battery_pct),
        }
    } else if info.battery_pct >= MIN_BATTERY {
        GateResult {
            name: "power",
            ok: true,
            reason: format!("ok: battery {}% (>= {}%)", info.battery_pct, MIN_BATTERY),
        }
    } else {
        GateResult {
            name: "power",
            ok: false,
            reason: format!(
                "skip: battery {}% below {}% and not on AC",
                info.battery_pct, MIN_BATTERY
            ),
        }
    }
}

pub fn gate_disk(free_gb: i64) -> GateResult {
    if free_gb >= MIN_DISK_GB {
        GateResult {
            name: "disk",
            ok: true,
            reason: format!("ok: {}GB free (>= {}GB)", free_gb, MIN_DISK_GB),
        }
    } else {
        GateResult {
            name: "disk",
            ok: false,
            reason: format!("skip: only {}GB free (need {}GB)", free_gb, MIN_DISK_GB),
        }
    }
}

pub fn gate_network() -> GateResult {
    let check = |url: &str| {
        Command::new("curl")
            .args(["-fsSI", "--max-time", "5", url])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    if check("https://formulae.brew.sh") && check("https://www.apple.com/library/test/success.html")
    {
        GateResult {
            name: "network",
            ok: true,
            reason: "ok: brew CDN and Apple CDN reachable".into(),
        }
    } else {
        GateResult {
            name: "network",
            ok: false,
            reason: "skip: network unreachable".into(),
        }
    }
}

pub fn gate_pkgmgr() -> GateResult {
    let running = Command::new("pgrep")
        .args(["-f", "/(brew|mas)( |$)"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if running {
        GateResult {
            name: "pkgmgr",
            ok: false,
            reason: "skip: another brew/mas process is running".into(),
        }
    } else {
        GateResult {
            name: "pkgmgr",
            ok: true,
            reason: "ok: no brew/mas process running".into(),
        }
    }
}

pub fn gate_schedule(state: &State) -> GateResult {
    let now = now_secs();
    match state.last_success_at {
        None => GateResult {
            name: "schedule",
            ok: true,
            reason: "ok: last success never (due)".into(),
        },
        Some(last) if now - last >= CADENCE_SECS => GateResult {
            name: "schedule",
            ok: true,
            reason: format!("ok: last success {} (due)", last),
        },
        Some(last) => GateResult {
            name: "schedule",
            ok: false,
            reason: format!("skip: ran successfully {} (< {}s ago)", last, CADENCE_SECS),
        },
    }
}

pub fn gate_dialog_cooldown(state: &State) -> GateResult {
    let now = now_secs();
    match state.last_dialog_at {
        None => GateResult {
            name: "dialog_cooldown",
            ok: true,
            reason: "ok: no dialog in the last 24h".into(),
        },
        Some(last) if now - last >= DIALOG_COOLDOWN_SECS => GateResult {
            name: "dialog_cooldown",
            ok: true,
            reason: "ok: no dialog in the last 24h".into(),
        },
        Some(_) => GateResult {
            name: "dialog_cooldown",
            ok: false,
            reason: "skip: dialog already shown within 24h (once-per-day cap)".into(),
        },
    }
}

pub fn gate_env(
    power: &GateResult,
    network: &GateResult,
    disk: &GateResult,
    pkgmgr: &GateResult,
) -> Option<GateResult> {
    for g in [power, network, disk, pkgmgr] {
        if !g.ok {
            return Some(g.clone());
        }
    }
    None
}
