//! `dotfiles doctor` — environment diagnosis with ok/warn/fail rows.

use crate::ctx::Ctx;
use anyhow::Result;

struct Check {
    name: &'static str,
    critical: bool,
    status: &'static str, // ok | warn | fail
    detail: String,
}

pub fn run(ctx: &Ctx) -> Result<()> {
    let mut checks = vec![];

    let brew = ctx.env.has_command("brew");
    checks.push(Check {
        name: "homebrew",
        critical: true,
        status: if brew { "ok" } else { "fail" },
        detail: if brew {
            "installed".into()
        } else {
            "run `dotfiles bootstrap`".into()
        },
    });

    let clt = ctx
        .env
        .output("xcode-select", &["-p"])
        .map(|o| o.ok())
        .unwrap_or(false);
    checks.push(Check {
        name: "xcode-clt",
        critical: true,
        status: if clt { "ok" } else { "fail" },
        detail: if clt {
            "installed".into()
        } else {
            "run `xcode-select --install`".into()
        },
    });

    let manifest = ctx.manifest();
    checks.push(Check {
        name: "manifest",
        critical: true,
        status: if manifest.is_ok() { "ok" } else { "fail" },
        detail: match &manifest {
            Ok(m) => format!(
                "{} formulas, {} casks, {} mas apps, {} links",
                m.install.brew.formulas.len(),
                m.install.brew.casks.len(),
                m.install.mas.apps.len(),
                m.config.symbolic_links.len()
            ),
            Err(e) => e.to_string(),
        },
    });

    let local_bin = ctx.env.home.join(".local/bin");
    checks.push(Check {
        name: "~/.local/bin",
        critical: false,
        status: if local_bin.is_dir() { "ok" } else { "warn" },
        detail: if local_bin.is_dir() {
            "exists".into()
        } else {
            "missing — will be created by `dotfiles apply`".into()
        },
    });

    let user = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
    let zsh = ctx
        .env
        .output(
            "dscl",
            &[".", "-read", &format!("/Users/{}", user), "UserShell"],
        )
        .map(|o| o.stdout)
        .unwrap_or_default();
    let shell_ok = zsh.contains("/bin/zsh");
    checks.push(Check {
        name: "login shell",
        critical: false,
        status: if shell_ok { "ok" } else { "warn" },
        detail: zsh
            .split_whitespace()
            .last()
            .unwrap_or("unknown")
            .to_string(),
    });

    let mut critical_failures = 0;
    for c in &checks {
        let mark = match c.status {
            "ok" => "✓",
            "warn" => "!",
            _ => "✗",
        };
        println!("{} {:14} {}", mark, c.name, c.detail);
        if c.critical && c.status == "fail" {
            critical_failures += 1;
        }
    }
    if critical_failures > 0 {
        anyhow::bail!("{} critical check(s) failed", critical_failures);
    }
    Ok(())
}
