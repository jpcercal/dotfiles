use serde_json::Value;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Section {
    pub title: String,
    pub count: usize,
    pub status: String,
    pub items: Vec<String>,
}

impl Section {
    pub fn new(
        title: impl Into<String>,
        count: usize,
        status: impl Into<String>,
        items: Vec<String>,
    ) -> Self {
        Self {
            title: title.into(),
            count,
            status: status.into(),
            items,
        }
    }

    pub fn formatted(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("{} ({})\n", self.title, self.count));
        s.push_str(&format!("=> {}\n", self.status));
        for it in &self.items {
            s.push_str(&format!("* {}\n", it));
        }
        s.push('\n');
        s
    }
}

fn has_command(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run_with_timeout(secs: u64, program: &str, args: &[&str]) -> Option<String> {
    // try gtimeout if present
    let timeout_bin = if Command::new("which")
        .arg("gtimeout")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        Some("gtimeout")
    } else {
        None
    };

    let mut cmd = if let Some(bin) = timeout_bin {
        let mut c = Command::new(bin);
        c.arg(secs.to_string());
        c.arg(program);
        c.args(args);
        c
    } else {
        let mut c = Command::new(program);
        c.args(args);
        c
    };
    let out = cmd.output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

#[allow(dead_code)]
fn run_with_timeout_stderr(
    secs: u64,
    program: &str,
    args: &[&str],
) -> Option<(String, String, bool)> {
    let timeout_bin = if Command::new("which")
        .arg("gtimeout")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        Some("gtimeout")
    } else {
        None
    };
    let mut cmd = if let Some(bin) = timeout_bin {
        let mut c = Command::new(bin);
        c.arg(secs.to_string());
        c.arg(program);
        c.args(args);
        c
    } else {
        let mut c = Command::new(program);
        c.args(args);
        c
    };
    let out = cmd.output().ok()?;
    Some((
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    ))
}

pub fn probe_brew() -> Section {
    if !has_command("brew") {
        return Section::new("Brew", 0, "Unavailable (not installed)", vec![]);
    }
    let json_str = Command::new("brew")
        .args(["outdated", "--json=v2"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_else(|_| r#"{"formulae":[],"casks":[]}"#.into());
    let v: Value =
        serde_json::from_str(&json_str).unwrap_or(serde_json::json!({"formulae":[],"casks":[]}));
    let mut f_items = vec![];
    let mut c_items = vec![];
    if let Some(arr) = v.get("formulae").and_then(|x| x.as_array()) {
        for f in arr {
            let name = f.get("name").and_then(|x| x.as_str()).unwrap_or("");
            let installed = f
                .get("installed_versions")
                .and_then(|x| x.as_array())
                .and_then(|a| a.first())
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let current = f
                .get("current_version")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if !name.is_empty() {
                f_items.push(format!("{} ({} -> {})", name, installed, current));
            }
        }
    }
    if let Some(arr) = v.get("casks").and_then(|x| x.as_array()) {
        for c in arr {
            let name = c.get("name").and_then(|x| x.as_str()).unwrap_or("");
            let installed = c
                .get("installed_versions")
                .and_then(|x| x.as_array())
                .and_then(|a| a.first())
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let current = c
                .get("current_version")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if !name.is_empty() {
                c_items.push(format!("{} ({} -> {})", name, installed, current));
            }
        }
    }
    // We mimic bash: two sections but here we return combined? Bash does two separate section_add.
    // To keep parity, we'll return a single aggregated section for "Brew" if needed elsewhere,
    // but probe_summary below creates two sections explicitly.
    // This function is not used directly for summary; probe_summary handles both.
    // Return formulae as primary.
    Section::new(
        "Brew Formulae",
        f_items.len(),
        if f_items.is_empty() {
            "No updates available"
        } else {
            "Updates available"
        },
        f_items,
    )
}

pub fn probe_brew_sections() -> Vec<Section> {
    if !has_command("brew") {
        return vec![
            Section::new("Brew Formulae", 0, "Unavailable (not installed)", vec![]),
            Section::new("Brew Casks", 0, "Unavailable (not installed)", vec![]),
        ];
    }
    let json_str = Command::new("brew")
        .args(["outdated", "--json=v2"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_else(|_| r#"{"formulae":[],"casks":[]}"#.into());
    let v: Value =
        serde_json::from_str(&json_str).unwrap_or(serde_json::json!({"formulae":[],"casks":[]}));
    let mut f_items = vec![];
    let mut c_items = vec![];
    if let Some(arr) = v.get("formulae").and_then(|x| x.as_array()) {
        for f in arr {
            let name = f.get("name").and_then(|x| x.as_str()).unwrap_or("");
            let installed = f
                .get("installed_versions")
                .and_then(|x| x.as_array())
                .and_then(|a| a.first())
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let current = f
                .get("current_version")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if !name.is_empty() {
                f_items.push(format!("{} ({} -> {})", name, installed, current));
            }
        }
    }
    if let Some(arr) = v.get("casks").and_then(|x| x.as_array()) {
        for c in arr {
            let name = c.get("name").and_then(|x| x.as_str()).unwrap_or("");
            let installed = c
                .get("installed_versions")
                .and_then(|x| x.as_array())
                .and_then(|a| a.first())
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let current = c
                .get("current_version")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if !name.is_empty() {
                c_items.push(format!("{} ({} -> {})", name, installed, current));
            }
        }
    }
    vec![
        Section::new(
            "Brew Formulae",
            f_items.len(),
            if f_items.is_empty() {
                "No updates available"
            } else {
                "Updates available"
            },
            f_items,
        ),
        Section::new(
            "Brew Casks",
            c_items.len(),
            if c_items.is_empty() {
                "No updates available"
            } else {
                "Updates available"
            },
            c_items,
        ),
    ]
}

pub fn probe_mas() -> Section {
    if !has_command("mas") {
        return Section::new("MAS", 0, "Unavailable (not installed)", vec![]);
    }
    let out = Command::new("mas").arg("outdated").output();
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout);
            let mut items = vec![];
            for line in s.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                // bash: sed -E 's/^[0-9]+[[:space:]]+//; s/[[:space:]]+\(([^ ]+) -> ([^)]+)\)$/ (\1 -> \2)/'
                // Simplified: take line, try to parse.
                // mas outdated format: "123 AppName (1.0 -> 2.0)"
                let without_id = trimmed
                    .split_once(char::is_whitespace)
                    .map(|x| x.1)
                    .unwrap_or(trimmed)
                    .trim();
                let item = without_id.to_string();
                items.push(item);
            }
            // Filter to those that look like outdated (bash used grep -E '^[0-9]+')
            // We already iterated all lines; keep only non-empty.
            Section::new(
                "MAS",
                items.len(),
                if items.is_empty() {
                    "No updates available"
                } else {
                    "Updates available"
                },
                items,
            )
        }
        _ => Section::new("MAS", 0, "Unavailable (App Store session)", vec![]),
    }
}

pub fn probe_rust() -> Section {
    if !has_command("rustup") {
        return Section::new("Rust", 0, "Unavailable (not installed)", vec![]);
    }
    let out = run_with_timeout(30, "rustup", &["check"]);
    let s = out.unwrap_or_default();
    let mut items = vec![];
    for line in s.lines() {
        if line.contains("Update available") {
            // bash sed: s/^[[:space:]]*//; s/ - Update available : ([^ ]+)([[:space:]]+\(.*\))? -> ([^ ]+)([[:space:]]+\(.*\))?/ (\1 -> \3)/
            let trimmed = line.trim();
            // Extract: "<name> - Update available : <from> ... -> <to> ..."
            if let Some(idx) = trimmed.find(" - Update available : ") {
                let name = trimmed[..idx].trim();
                let rest = &trimmed[idx + " - Update available : ".len()..];
                // rest: "1.78.0 (abc) -> 1.79.0 (def)"  take first token and third token
                let parts: Vec<&str> = rest.split("->").collect();
                if parts.len() >= 2 {
                    let from = parts[0].split_whitespace().next().unwrap_or("").trim();
                    let to = parts[1].split_whitespace().next().unwrap_or("").trim();
                    items.push(format!("{} ({} -> {})", name, from, to));
                } else {
                    items.push(trimmed.to_string());
                }
            } else {
                items.push(trimmed.to_string());
            }
        }
    }
    Section::new(
        "Rust",
        items.len(),
        if items.is_empty() {
            "No updates available"
        } else {
            "Updates available"
        },
        items,
    )
}

pub fn probe_node() -> Section {
    if !has_command("fnm") {
        return Section::new("Node (fnm)", 0, "Unavailable (not installed)", vec![]);
    }
    // eval "$(fnm env)" not needed for ls-remote? we try direct.
    let cur = Command::new("fnm")
        .arg("current")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let target = run_with_timeout(30, "fnm", &["ls-remote", "--lts"])
        .and_then(|s| {
            s.lines()
                .last()
                .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
        })
        .unwrap_or_default();
    let mut items = vec![];
    if !target.is_empty() && !cur.is_empty() && cur != target && cur != "system" && cur != "default"
    {
        items.push(format!("node ({} -> {})", cur, target));
    }
    // npm outdated -g --json
    if let Ok(out) = Command::new("npm")
        .args(["outdated", "-g", "--json"])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout);
        // npm outdated exits 1 when outdated; we still parse. Also need to handle stray docs: jq -s '.[0] // {}'
        // So try parse as json, if array take first.
        let v: Value = serde_json::from_str(&s)
            .or_else(|_| {
                // try splitting? If there are multiple json objects, take first?
                // simplest: find first '{' to last '}'
                let start = s.find('{').unwrap_or(0);
                let end = s.rfind('}').map(|i| i + 1).unwrap_or(s.len());
                serde_json::from_str(&s[start..end])
            })
            .unwrap_or(Value::Object(Default::default()));
        // If v is array, take first
        let obj = if v.is_array() {
            v.as_array()
                .and_then(|a| a.first())
                .cloned()
                .unwrap_or(Value::Object(Default::default()))
        } else {
            v
        };
        if let Some(map) = obj.as_object() {
            for (p, info) in map {
                let cur_v = info.get("current").and_then(|x| x.as_str()).unwrap_or("");
                let latest = info.get("latest").and_then(|x| x.as_str()).unwrap_or("");
                if !cur_v.is_empty() && !latest.is_empty() {
                    items.push(format!("{} ({} -> {})", p, cur_v, latest));
                }
            }
        }
    }
    Section::new(
        "Node (fnm)",
        items.len(),
        if items.is_empty() {
            "No updates available"
        } else {
            "Updates available"
        },
        items,
    )
}

pub fn probe_python() -> Section {
    if !has_command("uv") {
        return Section::new("Python (uv)", 0, "Unavailable (not installed)", vec![]);
    }
    let mut items = vec![];
    let uv_ver = Command::new("uv")
        .args(["--version"])
        .output()
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stdout);
            s.split_whitespace().nth(1).unwrap_or("").to_string()
        })
        .unwrap_or_default();
    let uv_latest = run_with_timeout(
        8,
        "curl",
        &["-fsSL", "--max-time", "8", "https://pypi.org/pypi/uv/json"],
    )
    .and_then(|s| {
        let v: Value = serde_json::from_str(&s).ok()?;
        v.get("info")
            .and_then(|i| i.get("version"))
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
    })
    .unwrap_or_default();
    if !uv_ver.is_empty() && !uv_latest.is_empty() && uv_ver != uv_latest {
        items.push(format!("uv ({} -> {})", uv_ver, uv_latest));
    }
    // Check pynvim/neovim
    let py = Command::new("uv")
        .args(["python", "find"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if !py.is_empty() {
        let pip_check = Command::new("uv")
            .args([
                "pip",
                "install",
                "--dry-run",
                "-U",
                "--break-system-packages",
                "--python",
                &py,
                "pynvim",
                "neovim",
            ])
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr)
            })
            .unwrap_or_default();
        // get current versions
        let list_out = Command::new("uv")
            .args(["pip", "list", "--python", &py])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        let mut pnv = String::new();
        let mut nv = String::new();
        for line in list_out.lines() {
            if line.starts_with("pynvim ") {
                pnv = line.split_whitespace().nth(1).unwrap_or("").to_string();
            }
            if line.starts_with("neovim ") {
                nv = line.split_whitespace().nth(1).unwrap_or("").to_string();
            }
        }
        let mut pnv_new = String::new();
        let mut nv_new = String::new();
        for line in pip_check.lines() {
            let t = line.trim();
            if t.starts_with("+ pynvim==") {
                pnv_new = t
                    .trim_start_matches("+ pynvim==")
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string();
            }
            if t.starts_with("+ neovim==") {
                nv_new = t
                    .trim_start_matches("+ neovim==")
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string();
            }
            // also handle "+ pynvim==x.y.z"
            if t.contains("pynvim==") && pnv_new.is_empty() {
                if let Some(idx) = t.find("pynvim==") {
                    let rest = &t[idx + "pynvim==".len()..];
                    pnv_new = rest
                        .split(|c: char| c.is_whitespace() || c == ')')
                        .next()
                        .unwrap_or("")
                        .to_string();
                }
            }
            if t.contains("neovim==") && nv_new.is_empty() {
                if let Some(idx) = t.find("neovim==") {
                    let rest = &t[idx + "neovim==".len()..];
                    nv_new = rest
                        .split(|c: char| c.is_whitespace() || c == ')')
                        .next()
                        .unwrap_or("")
                        .to_string();
                }
            }
        }
        if !pnv.is_empty() && !pnv_new.is_empty() && pnv != pnv_new {
            items.push(format!("pynvim ({} -> {})", pnv, pnv_new));
        }
        if !nv.is_empty() && !nv_new.is_empty() && nv != nv_new {
            items.push(format!("neovim ({} -> {})", nv, nv_new));
        }
    }
    Section::new(
        "Python (uv)",
        items.len(),
        if items.is_empty() {
            "No updates available"
        } else {
            "Updates available"
        },
        items,
    )
}

pub fn probe_opencode() -> Section {
    if !has_command("opencode") {
        return Section::new("opencode", 0, "Unavailable (not installed)", vec![]);
    }
    let cur = Command::new("opencode")
        .args(["--version"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .unwrap_or_default();
    let latest = run_with_timeout(
        8,
        "curl",
        &[
            "-fsSL",
            "--max-time",
            "8",
            "https://registry.npmjs.org/opencode-ai/latest",
        ],
    )
    .and_then(|s| {
        let v: Value = serde_json::from_str(&s).ok()?;
        v.get("version")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
    })
    .unwrap_or_default();
    let mut items = vec![];
    if !cur.is_empty() && !latest.is_empty() && cur != latest {
        items.push(format!("opencode ({} -> {})", cur, latest));
    }
    Section::new(
        "opencode",
        items.len(),
        if items.is_empty() {
            "No updates available"
        } else {
            "Updates available"
        },
        items,
    )
}

pub fn probe_nvim() -> Section {
    if !has_command("nvim") {
        return Section::new("Neovim Plugins", 0, "Unavailable (not installed)", vec![]);
    }
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    let vimrc = home.join(".vimrc");
    let content = std::fs::read_to_string(&vimrc).unwrap_or_default();
    let mut items = vec![];
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with("Plug ") {
            // Plug 'xxx' or Plug "xxx"
            let rest = t.trim_start_matches("Plug").trim();
            let first_char = rest.chars().next().unwrap_or(' ');
            if first_char == '\'' || first_char == '"' {
                let quote = first_char;
                if let Some(end) = rest[1..].find(quote) {
                    let name = &rest[1..1 + end];
                    items.push(name.to_string());
                }
            }
        }
    }
    Section::new(
        "Neovim Plugins",
        items.len(),
        if items.is_empty() {
            "No updates available"
        } else {
            "Checked at run time (results in the JSON report)"
        },
        items,
    )
}

pub fn probe_gem() -> Section {
    if !has_command("gem") {
        return Section::new("Gem", 0, "Unavailable (not installed)", vec![]);
    }
    let out = run_with_timeout(30, "gem", &["outdated"]);
    let line = out
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("neovim "))
        .map(|s| s.to_string())
        .unwrap_or_default();
    let mut items = vec![];
    if !line.is_empty() {
        // line: "neovim (0.9.0 < 0.10.0)"
        // bash: s/^([^ ]+) \(([^ ]+) < ([^)]+)\).*/\1 (\2 -> \3)/
        if let Some(start) = line.find('(') {
            let name = line[..start].trim();
            let inside = line[start + 1..].trim_end_matches(')');
            let parts: Vec<&str> = inside.split('<').collect();
            if parts.len() == 2 {
                let from = parts[0].trim();
                let to = parts[1].trim();
                items.push(format!("{} ({} -> {})", name, from, to));
            } else {
                items.push(line);
            }
        } else {
            items.push(line);
        }
    }
    Section::new(
        "Gem",
        items.len(),
        if items.is_empty() {
            "No updates available"
        } else {
            "Updates available"
        },
        items,
    )
}

pub fn probe_macos() -> Section {
    let out = run_with_timeout(60, "softwareupdate", &["--list"]);
    let s = out.unwrap_or_default();
    let mut items = vec![];
    for line in s.lines() {
        let t = line.trim();
        if t.starts_with("* Label: ") {
            items.push(t.trim_start_matches("* Label: ").to_string());
        }
    }
    Section::new(
        "macOS",
        items.len(),
        if items.is_empty() {
            "No updates available"
        } else {
            "Updates available (install manually via System Settings)"
        },
        items,
    )
}

pub fn probe_tpm() -> Option<Section> {
    let home = dirs::home_dir()?;
    let plugins = home.join(".tmux/plugins");
    if !plugins.is_dir() {
        return None;
    }
    let mut items = vec![];
    if let Ok(entries) = std::fs::read_dir(&plugins) {
        for e in entries.flatten() {
            let p = e.path().join(".git");
            if p.exists() {
                if let Some(name) = e.file_name().to_str() {
                    items.push(name.to_string());
                }
            }
        }
    }
    if items.is_empty() {
        return None;
    }
    Some(Section::new(
        "Tmux TPM",
        items.len(),
        format!("Checked at run time — {}", items.join(", ")),
        items.clone(),
    ))
}

pub fn probe_all() -> Vec<Section> {
    // Parallel startup: every independent version check runs concurrently
    // via std::thread::scope, so wall time ≈ max(slowest probe) instead of sum.
    std::thread::scope(|s| {
        let brew_h = s.spawn(probe_brew_sections);
        let mas_h = s.spawn(probe_mas);
        let rust_h = s.spawn(probe_rust);
        let node_h = s.spawn(probe_node);
        let python_h = s.spawn(probe_python);
        let opencode_h = s.spawn(probe_opencode);
        let nvim_h = s.spawn(probe_nvim);
        let gem_h = s.spawn(probe_gem);
        let macos_h = s.spawn(probe_macos);
        let tpm_h = s.spawn(probe_tpm);

        let mut sections = Vec::with_capacity(11);
        // Preserve original ordering for stable UI.
        sections.extend(brew_h.join().unwrap_or_default());
        sections.push(
            mas_h
                .join()
                .unwrap_or_else(|_| Section::new("MAS", 0, "Unavailable (probe failed)", vec![])),
        );
        sections.push(
            rust_h
                .join()
                .unwrap_or_else(|_| Section::new("Rust", 0, "Unavailable (probe failed)", vec![])),
        );
        sections.push(node_h.join().unwrap_or_else(|_| {
            Section::new("Node (fnm)", 0, "Unavailable (probe failed)", vec![])
        }));
        sections.push(python_h.join().unwrap_or_else(|_| {
            Section::new("Python (uv)", 0, "Unavailable (probe failed)", vec![])
        }));
        sections.push(
            opencode_h.join().unwrap_or_else(|_| {
                Section::new("opencode", 0, "Unavailable (probe failed)", vec![])
            }),
        );
        sections.push(nvim_h.join().unwrap_or_else(|_| {
            Section::new("Neovim Plugins", 0, "Unavailable (probe failed)", vec![])
        }));
        sections.push(
            gem_h
                .join()
                .unwrap_or_else(|_| Section::new("Gem", 0, "Unavailable (probe failed)", vec![])),
        );
        sections.push(
            macos_h
                .join()
                .unwrap_or_else(|_| Section::new("macOS", 0, "Unavailable (probe failed)", vec![])),
        );
        if let Some(sec) = tpm_h.join().unwrap_or(None) {
            sections.push(sec);
        }
        sections
    })
}

pub fn summary_text(sections: &[Section]) -> String {
    let mut s = String::new();
    for sec in sections {
        s.push_str(&sec.formatted());
    }
    s.trim_end().to_string()
}
