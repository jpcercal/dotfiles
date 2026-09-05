//! `dotfiles verify` — read-only reference checking for `apps.yaml`.
//!
//! Two check families, both parallelized (sharded `std::thread::scope`, same
//! philosophy as the upgrade probes):
//! - **local**: symlink sources exist in the repo, symlink target dirs are
//!   covered by `config.mkdir` (or exist), dock entries resolve to a declared
//!   cask / MAS app or an allowlisted system path;
//! - **probes**: every referenced formula / cask / tap / MAS id / gem / npm /
//!   pip / go module exists upstream (`brew info`, `mas info`, `npm view`,
//!   PyPI JSON, `go list -m`, …).
//!
//! Unit IDs use the canonical dependency-graph namespace
//! (`brew-formula:git`, `brew-cask:iterm2`, …) so `requires:` entries,
//! `install` args and this report spell a package identically. Exit status is
//! non-zero when any check is missing; unavailable probe tools report `SKIP`
//! (never a false failure).

use crate::ctx::Ctx;
use anyhow::Result;
use clap::Parser;
use dotfiles_exec::ExecEnv;

#[derive(Parser, Debug)]
pub struct VerifyArgs {
    /// Only run local reference checks (no network/tool probes).
    #[arg(long)]
    pub local_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckStatus {
    Ok,
    Missing(String),
    Skipped(String),
}

#[derive(Debug, Clone)]
pub struct Check {
    pub id: String,
    pub status: CheckStatus,
}

impl Check {
    fn ok(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: CheckStatus::Ok,
        }
    }
    fn missing(id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: CheckStatus::Missing(reason.into()),
        }
    }
    fn skipped(id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: CheckStatus::Skipped(reason.into()),
        }
    }

    pub fn is_missing(&self) -> bool {
        matches!(self.status, CheckStatus::Missing(_))
    }
}

pub fn run(ctx: &Ctx, args: VerifyArgs) -> Result<()> {
    let checks = collect(ctx, args.local_only)?;
    print_report(&checks)
}

/// Run all checks and return them sorted by unit ID (deterministic).
pub fn collect(ctx: &Ctx, local_only: bool) -> Result<Vec<Check>> {
    // Loading the manifest re-runs full validation (shape + graph), so an
    // invalid manifest is a fatal error here, distinct from missing refs.
    let m = ctx.manifest()?;
    let mut checks = local_checks(ctx, &m);
    if !local_only {
        checks.extend(probe_checks(&ctx.env, &m));
    }
    checks.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(checks)
}

pub fn print_report(checks: &[Check]) -> Result<()> {
    let mut ok = 0;
    let mut skipped = 0;
    for c in checks {
        match &c.status {
            CheckStatus::Ok => {
                println!("ok       {}", c.id);
                ok += 1;
            }
            CheckStatus::Missing(reason) => {
                println!("MISSING  {} — {}", c.id, reason);
            }
            CheckStatus::Skipped(reason) => {
                println!("SKIP     {} — {}", c.id, reason);
                skipped += 1;
            }
        }
    }
    let missing = checks.iter().filter(|c| c.is_missing()).count();
    println!("verify: {ok} ok, {missing} missing, {skipped} skipped");
    if missing > 0 {
        anyhow::bail!("verify: {missing} missing reference(s)");
    }
    Ok(())
}

/// Local reference checks: symlinks + dock (no tools, no network).
fn local_checks(ctx: &Ctx, m: &dotfiles_manifest::Manifest) -> Vec<Check> {
    let mut checks = vec![];

    let mkdirs: Vec<std::path::PathBuf> =
        m.config.mkdir.iter().map(|d| ctx.env.expand(d)).collect();
    for link in &m.config.symbolic_links {
        let id = format!("link:{}", link.to.absolute_path);
        let src = ctx.dotfiles_dir.join(&link.from.relative_path);
        if !src.exists() {
            checks.push(Check::missing(
                &id,
                format!("source '{}' not in repo", link.from.relative_path),
            ));
            continue;
        }
        let dst = ctx.env.expand(&link.to.absolute_path);
        let parent = dst.parent().map(|p| p.to_path_buf());
        let covered = match &parent {
            None => true,
            Some(p) => {
                p == &ctx.env.home
                    || p.exists()
                    || mkdirs.iter().any(|d| p == d || p.starts_with(d))
            }
        };
        if covered {
            checks.push(Check::ok(&id));
        } else {
            checks.push(Check::missing(
                &id,
                format!(
                    "target dir '{}' not covered by config.mkdir and does not exist",
                    parent.map(|p| p.display().to_string()).unwrap_or_default()
                ),
            ));
        }
    }

    // Declared install names for dock resolution.
    let casks: Vec<String> = m
        .install
        .brew
        .casks
        .iter()
        .map(|c| c.name().to_string())
        .collect();
    let mas_names: Vec<String> = m.install.mas.apps.iter().map(|a| a.name.clone()).collect();
    for entry in &m.config.dockutil.add {
        let id = format!("dock:{}", entry.app);
        if dock_resolves(&entry.app, &casks, &mas_names) {
            checks.push(Check::ok(&id));
        } else {
            checks.push(Check::missing(
                &id,
                "not a system app and matches no declared cask or MAS app".to_string(),
            ));
        }
    }

    checks
}

/// Does a dock `.app` path resolve to a managed app or the system?
/// Matching is token-insensitive (`Brave Browser.app` ↔ `brave-browser`,
/// `Airmail.app` ↔ MAS `Airmail`); either side being a prefix of the other
/// counts (`iTerm.app` ↔ cask `iterm2`, `Fantastical.app` ↔ `Fantastical 2`).
fn dock_resolves(app: &str, casks: &[String], mas_names: &[String]) -> bool {
    if app.starts_with("/System/") {
        return true;
    }
    let base = app.rsplit('/').next().unwrap_or(app);
    let base = base.strip_suffix(".app").unwrap_or(base);
    let norm = normalize_app_name(base);
    if norm.is_empty() {
        return false;
    }
    casks
        .iter()
        .map(|c| normalize_app_name(c))
        .any(|c| names_match(&norm, &c))
        || mas_names
            .iter()
            .map(|n| normalize_app_name(n))
            .any(|n| names_match(&norm, &n))
}

fn normalize_app_name(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

fn names_match(bundle: &str, declared: &str) -> bool {
    bundle == declared || bundle.starts_with(declared) || declared.starts_with(bundle)
}

/// Existence probes against the real ecosystems, sharded over a bounded
/// thread pool (16 shards). Each probe is read-only.
fn probe_checks(env: &ExecEnv, m: &dotfiles_manifest::Manifest) -> Vec<Check> {
    let mut tasks: Vec<(String, ProbeKind)> = vec![];
    for f in &m.install.brew.formulas {
        tasks.push((
            format!("brew-formula:{}", f.name()),
            ProbeKind::BrewFormula(f.name().to_string()),
        ));
    }
    for c in &m.install.brew.casks {
        tasks.push((
            format!("brew-cask:{}", c.name()),
            ProbeKind::BrewCask(c.name().to_string()),
        ));
    }
    for t in &m.install.brew.taps {
        tasks.push((format!("brew-tap:{t}"), ProbeKind::BrewTap(t.clone())));
    }
    for a in &m.install.mas.apps {
        tasks.push((format!("mas:{}", a.id), ProbeKind::Mas(a.id.clone())));
    }
    for g in &m.install.gem.rubygems {
        tasks.push((
            format!("gem:{}", g.name()),
            ProbeKind::Gem(g.name().to_string()),
        ));
    }
    for p in &m.install.npm.global.packages {
        tasks.push((
            format!("npm:{}", p.name()),
            ProbeKind::Npm(p.name().to_string()),
        ));
    }
    for p in &m.install.pip.packages {
        tasks.push((
            format!("pip:{}", p.name()),
            ProbeKind::Pip(p.name().to_string()),
        ));
    }
    for p in &m.install.go.packages {
        tasks.push((
            format!("go:{}", p.name()),
            ProbeKind::Go(p.name().to_string()),
        ));
    }

    if tasks.is_empty() {
        return vec![];
    }
    const SHARDS: usize = 16;
    let chunk = tasks.len().div_ceil(SHARDS);
    std::thread::scope(|s| {
        let mut handles = vec![];
        for shard in tasks.chunks(chunk) {
            handles.push(s.spawn(move || {
                let mut out = vec![];
                for (id, probe) in shard {
                    out.push(run_probe(env, id, probe));
                }
                out
            }));
        }
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("verify worker panicked"))
            .collect()
    })
}

// The probe enum lives at module level so both builders and runners name it.
enum ProbeKind {
    BrewFormula(String),
    BrewCask(String),
    BrewTap(String),
    Mas(String),
    Gem(String),
    Npm(String),
    Pip(String),
    Go(String),
}

/// Helper: run `tool args…`, mapping a missing tool / spawn error to SKIP,
/// a non-zero exit to MISSING, and a zero exit to OK.
fn tool_probe(env: &ExecEnv, id: &str, tool: &str, args: &[&str]) -> Check {
    if !env.has_command(tool) {
        return Check::skipped(id, format!("{tool} not installed"));
    }
    match env.output(tool, args) {
        Err(e) => Check::skipped(id, format!("{tool} failed to run: {e}")),
        Ok(res) if res.ok() => Check::ok(id),
        Ok(res) => {
            let detail = res.stderr.trim();
            let detail = detail.lines().last().unwrap_or("not found").trim();
            Check::missing(id, format!("{tool}: {detail}"))
        }
    }
}

fn run_probe(env: &ExecEnv, id: &str, probe: &ProbeKind) -> Check {
    match probe {
        ProbeKind::BrewFormula(name) => tool_probe(env, id, "brew", &["info", "--formula", name]),
        ProbeKind::BrewCask(name) => tool_probe(env, id, "brew", &["info", "--cask", name]),
        ProbeKind::BrewTap(tap) => tool_probe(env, id, "brew", &["tap-info", tap]),
        ProbeKind::Mas(app_id) => tool_probe(env, id, "mas", &["info", app_id]),
        ProbeKind::Gem(name) => {
            if !env.has_command("gem") {
                return Check::skipped(id, "gem not installed");
            }
            match env.output("gem", &["list", "--remote", "--exact", name]) {
                Err(e) => Check::skipped(id, format!("gem failed to run: {e}")),
                Ok(res) if res.ok() && !res.stdout.trim().is_empty() => Check::ok(id),
                Ok(_) => Check::missing(id, "gem: no such remote gem"),
            }
        }
        ProbeKind::Npm(name) => tool_probe(env, id, "npm", &["view", name, "version"]),
        ProbeKind::Pip(name) => {
            let url = format!("https://pypi.org/pypi/{name}/json");
            tool_probe(env, id, "curl", &["-fsSL", "--max-time", "20", &url])
        }
        ProbeKind::Go(module) => {
            let at = format!("{module}@latest");
            tool_probe(env, id, "go", &["list", "-m", &at])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotfiles_testkit::TestEnv;

    fn ctx_with_manifest(t: &TestEnv, yaml: &str) -> Ctx {
        let dir = t.dotfiles_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("apps.yaml"), yaml).unwrap();
        let mut ctx = Ctx::sandbox(t.root(), false).unwrap();
        // Ctx::sandbox inherits the process PATH (only TestEnv::exec() is
        // isolated): restrict to the stub dir so unstubbed tools can never
        // leak to the real machine mid-test.
        ctx.env = ctx.env.clone().with_isolated_base_paths(&[]);
        ctx
    }

    #[test]
    fn local_checks_cover_links_and_dock() {
        let t = TestEnv::new();
        t.write("dotfiles/.zshrc", "x\n");
        let ctx = ctx_with_manifest(
            &t,
            r#"
install:
  brew:
    casks: [iterm2]
  mas:
    apps:
      - { id: "1", name: "Trello" }
config:
  mkdir: ["~/.config/nvim/"]
  symbolic_links:
    - from: { relative_path: ".zshrc" }
      to: { absolute_path: "~/.zshrc" }
    - from: { relative_path: ".ghost" }
      to: { absolute_path: "~/.ghost" }
  dockutil:
    _before: { reset: false, removeAll: false }
    add:
      - app: "/Applications/iTerm.app"
      - app: "/System/Applications/Music.app"
      - app: "/Applications/Nope.app"
"#,
        );
        let m = ctx.manifest().unwrap();
        let checks = local_checks(&ctx, &m);
        let status = |id: &str| {
            checks
                .iter()
                .find(|c| c.id == id)
                .unwrap_or_else(|| panic!("{id}"))
                .status
                .clone()
        };
        assert_eq!(status("link:~/.zshrc"), CheckStatus::Ok);
        assert!(matches!(status("link:~/.ghost"), CheckStatus::Missing(_)));
        // iTerm.app ↔ cask iterm2 via prefix matching
        assert_eq!(status("dock:/Applications/iTerm.app"), CheckStatus::Ok);
        assert_eq!(
            status("dock:/System/Applications/Music.app"),
            CheckStatus::Ok
        );
        assert!(matches!(
            status("dock:/Applications/Nope.app"),
            CheckStatus::Missing(_)
        ));
    }

    #[test]
    fn probes_use_canonical_ids_and_skip_missing_tools() {
        let t = TestEnv::new();
        // No tools stubbed: everything skips, nothing is missing.
        let ctx = ctx_with_manifest(
            &t,
            "install:\n  brew:\n    formulas: [git]\n    taps: [a/b]\n  mas:\n    apps:\n      - { id: \"1\", name: A }\n",
        );
        let checks = collect(&ctx, false).unwrap();
        assert!(!checks.is_empty());
        assert!(!checks.iter().any(|c| c.is_missing()));
        assert!(checks
            .iter()
            .any(|c| c.id == "brew-formula:git" && matches!(c.status, CheckStatus::Skipped(_))));
    }

    #[test]
    fn probes_detect_present_and_absent_packages() {
        let t = TestEnv::new();
        t.stub(
            "brew",
            "case \"$*\" in *--formula*) [ \"$3\" = git ] && exit 0; exit 1 ;; *) exit 0 ;; esac",
        );
        t.stub(
            "mas",
            "case \"$2\" in 1) exit 0 ;; *) echo nope 1>&2; exit 1 ;; esac",
        );
        let ctx = ctx_with_manifest(
            &t,
            "install:\n  brew:\n    formulas: [git, ghost-pkg]\n  mas:\n    apps:\n      - { id: \"1\", name: A }\n      - { id: \"2\", name: B }\n",
        );
        let checks = collect(&ctx, false).unwrap();
        let status = |id: &str| {
            checks
                .iter()
                .find(|c| c.id == id)
                .unwrap_or_else(|| panic!("{id}"))
                .status
                .clone()
        };
        assert_eq!(status("brew-formula:git"), CheckStatus::Ok);
        assert!(matches!(
            status("brew-formula:ghost-pkg"),
            CheckStatus::Missing(_)
        ));
        assert_eq!(status("mas:1"), CheckStatus::Ok);
        assert!(matches!(status("mas:2"), CheckStatus::Missing(_)));
    }

    #[test]
    fn invalid_manifest_is_fatal_not_missing() {
        let t = TestEnv::new();
        let ctx = ctx_with_manifest(&t, "install:\n  brew:\n    formulas: [\"\"]\n");
        assert!(collect(&ctx, true).is_err());
    }

    #[test]
    fn dock_name_matching() {
        assert!(dock_resolves("/System/Applications/Music.app", &[], &[]));
        assert!(dock_resolves(
            "/Applications/Brave Browser.app",
            &["brave-browser".into()],
            &[]
        ));
        assert!(dock_resolves(
            "/Applications/iTerm.app",
            &["iterm2".into()],
            &[]
        ));
        assert!(dock_resolves(
            "/Applications/Airmail.app",
            &[],
            &["Airmail".into()]
        ));
        assert!(!dock_resolves(
            "/Applications/Nope.app",
            &["slack".into()],
            &["Trello".into()]
        ));
    }

    #[test]
    fn manifest_without_packages_collects_nothing() {
        let t = TestEnv::new();
        let ctx = ctx_with_manifest(&t, "---\n");
        let checks = collect(&ctx, false).unwrap();
        assert!(checks.is_empty());
        assert!(print_report(&checks).is_ok());
    }
}
