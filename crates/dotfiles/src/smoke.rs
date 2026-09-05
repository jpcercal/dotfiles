//! Post-install smoke test (`dotfiles doctor --smoke`): invoke every manifest
//! package to prove it actually runs. Check IDs reuse the canonical
//! dependency-graph namespace (`brew-formula:jq`, `npm:prettier`, …) so the
//! report reads identically to `verify`, `install` and the graph.
//!
//! Invocation per type (missing invoking tool → SKIP, like `verify`; a
//! present-but-broken package → FAIL, which fails the command):
//! - brew formula → `<brew --prefix f>/bin/<f> --help` (first executable in
//!   that bin dir as fallback; formulae without binaries SKIP). `go` uses
//!   `version` (`go --help` exits 2).
//! - brew cask → `brew list --cask <c>` plus an existence check of a listed
//!   artifact (GUI bundles are never launched).
//! - MAS app → `mas info <id>` (read-only).
//! - gem → `ruby -e "gem '<g>'"` (activates the installed gem spec).
//! - npm → `<npm prefix -g>/bin/<p> --help`.
//! - pip → `<uv python> -c "importlib.metadata.version('<p>')"` (validates the
//!   distribution regardless of its import name).
//! - go → `<GOBIN|GOPATH/bin|~/go/bin>/<binary> --help`.
//! - toolchains → `rustc --help`, `fnm exec --using=lts-latest node --help`,
//!   `<uv python> --help`.

use crate::ctx::Ctx;
use anyhow::Result;

#[derive(Debug)]
pub enum SmokeStatus {
    Ok,
    Failed(String),
    Skipped(String),
}

pub struct SmokeCheck {
    pub id: String,
    pub status: SmokeStatus,
}

impl SmokeCheck {
    fn ok(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: SmokeStatus::Ok,
        }
    }

    fn failed(id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: SmokeStatus::Failed(reason.into()),
        }
    }

    fn skipped(id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: SmokeStatus::Skipped(reason.into()),
        }
    }

    pub fn is_failed(&self) -> bool {
        matches!(self.status, SmokeStatus::Failed(_))
    }
}

pub fn run(ctx: &Ctx) -> Result<()> {
    let mut checks = collect(ctx);
    checks.sort_by(|a, b| a.id.cmp(&b.id));
    print_report(&checks)
}

fn collect(ctx: &Ctx) -> Vec<SmokeCheck> {
    let m = match ctx.manifest() {
        Ok(m) => m,
        Err(e) => {
            return vec![SmokeCheck::failed(
                "manifest",
                format!("cannot load manifest: {e:#}"),
            )];
        }
    };
    let env = &ctx.env;
    let mut checks = vec![];
    for f in &m.install.brew.formulas {
        checks.push(smoke_formula(env, f.name()));
    }
    for c in &m.install.brew.casks {
        checks.push(smoke_cask(env, c.name()));
    }
    for a in &m.install.mas.apps {
        checks.push(smoke_mas(env, &a.id));
    }
    for g in &m.install.gem.rubygems {
        checks.push(smoke_gem(env, g.name()));
    }
    for p in &m.install.npm.global.packages {
        checks.push(smoke_npm(env, p.name()));
    }
    for p in &m.install.pip.packages {
        checks.push(smoke_pip(env, p.name()));
    }
    for p in &m.install.go.packages {
        checks.push(smoke_go(env, p.name()));
    }
    if m.install.toolchains.rustup.is_some() {
        checks.push(smoke_tool(env, "toolchain:rustup", "rustc", &["--help"]));
    }
    if m.install.toolchains.node.is_some() {
        checks.push(smoke_node(env));
    }
    if m.install.toolchains.python.is_some() {
        checks.push(smoke_python(env));
    }
    checks
}

fn print_report(checks: &[SmokeCheck]) -> Result<()> {
    let mut ok = 0;
    let mut skipped = 0;
    for c in checks {
        match &c.status {
            SmokeStatus::Ok => {
                println!("ok       {}", c.id);
                ok += 1;
            }
            SmokeStatus::Failed(reason) => {
                println!("FAIL     {} — {}", c.id, reason);
            }
            SmokeStatus::Skipped(reason) => {
                println!("SKIP     {} — {}", c.id, reason);
                skipped += 1;
            }
        }
    }
    let failed = checks.iter().filter(|c| c.is_failed()).count();
    println!("smoke: {ok} ok, {failed} failed, {skipped} skipped");
    if failed > 0 {
        anyhow::bail!("smoke: {failed} check(s) failed");
    }
    Ok(())
}

/// Run `tool args…`: missing tool → SKIP, non-zero exit → Failed, zero → Ok.
fn smoke_tool(env: &dotfiles_exec::ExecEnv, id: &str, tool: &str, args: &[&str]) -> SmokeCheck {
    if !env.has_command(tool) {
        return SmokeCheck::skipped(id, format!("{tool} not installed"));
    }
    match env.output(tool, args) {
        Err(e) => SmokeCheck::skipped(id, format!("{tool} failed to run: {e}")),
        Ok(res) if res.ok() => SmokeCheck::ok(id),
        Ok(res) => {
            let detail = res.stderr.trim();
            let detail = detail.lines().last().unwrap_or("exited non-zero").trim();
            SmokeCheck::failed(id, format!("{tool}: {detail}"))
        }
    }
}

/// Probe flags per formula binary. Everything answers `--help` except `go`
/// (`go --help` exits 2; `go version` is the stable invocation).
fn formula_probe_args(name: &str) -> &[&str] {
    match name {
        "go" => &["version"],
        _ => &["--help"],
    }
}

fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        path.is_file()
            && path
                .metadata()
                .map(|m| m.mode() & 0o111 != 0)
                .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// First executable directly under `dir` (sorted, deterministic).
fn first_executable(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_executable(p))
        .collect();
    names.sort();
    names.into_iter().next()
}

fn smoke_formula(env: &dotfiles_exec::ExecEnv, name: &str) -> SmokeCheck {
    let id = format!("brew-formula:{name}");
    if !env.has_command("brew") {
        return SmokeCheck::skipped(&id, "brew not installed");
    }
    let prefix = match env.output("brew", &["--prefix", name]) {
        Ok(res) if res.ok() => res.stdout.trim().to_string(),
        _ => return SmokeCheck::failed(&id, "brew: formula not installed"),
    };
    if prefix.is_empty() {
        return SmokeCheck::failed(&id, "brew: formula not installed");
    }
    let bin_dir = std::path::PathBuf::from(&prefix).join("bin");
    let bin = bin_dir.join(name);
    let bin = if is_executable(&bin) {
        bin
    } else {
        match first_executable(&bin_dir) {
            Some(b) => b,
            None => return SmokeCheck::skipped(&id, "formula has no binaries"),
        }
    };
    let bin_str = bin.to_string_lossy().to_string();
    match env.output(&bin_str, formula_probe_args(name)) {
        Err(e) => SmokeCheck::skipped(&id, format!("failed to run: {e}")),
        Ok(res) if res.ok() => SmokeCheck::ok(&id),
        Ok(res) => {
            let detail = res.stderr.trim();
            let detail = detail.lines().last().unwrap_or("exited non-zero").trim();
            SmokeCheck::failed(&id, format!("{}: {detail}", bin.display()))
        }
    }
}

fn smoke_cask(env: &dotfiles_exec::ExecEnv, name: &str) -> SmokeCheck {
    let id = format!("brew-cask:{name}");
    if !env.has_command("brew") {
        return SmokeCheck::skipped(&id, "brew not installed");
    }
    // Never launch GUI bundles: validate the installed artifacts exist.
    match env.output("brew", &["list", "--cask", name]) {
        Err(e) => SmokeCheck::skipped(&id, format!("brew failed to run: {e}")),
        Ok(res) if !res.ok() => SmokeCheck::failed(&id, "brew: cask not installed"),
        Ok(res) => {
            let landed = res
                .stdout
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && std::path::Path::new(l).exists())
                .count();
            if landed > 0 {
                SmokeCheck::ok(&id)
            } else {
                SmokeCheck::failed(&id, "brew: no installed artifacts on disk")
            }
        }
    }
}

fn smoke_mas(env: &dotfiles_exec::ExecEnv, app_id: &str) -> SmokeCheck {
    smoke_tool(env, &format!("mas:{app_id}"), "mas", &["info", app_id])
}

fn smoke_gem(env: &dotfiles_exec::ExecEnv, name: &str) -> SmokeCheck {
    let id = format!("gem:{name}");
    if !env.has_command("gem") || !env.has_command("ruby") {
        return SmokeCheck::skipped(&id, "gem/ruby not installed");
    }
    // Activating the gem spec validates the installation without knowing the
    // library's require path (raises Gem::LoadError when absent).
    let probe = format!("gem '{name}'");
    match env.output("ruby", &["-e", &probe]) {
        Err(e) => SmokeCheck::skipped(&id, format!("ruby failed to run: {e}")),
        Ok(res) if res.ok() => SmokeCheck::ok(&id),
        Ok(res) => {
            // Ruby prints multi-line backtraces: the error is the first line.
            let detail = res.stderr.trim();
            let detail = detail.lines().next().unwrap_or("exited non-zero").trim();
            SmokeCheck::failed(&id, format!("ruby: {detail}"))
        }
    }
}

fn smoke_npm(env: &dotfiles_exec::ExecEnv, name: &str) -> SmokeCheck {
    let id = format!("npm:{name}");
    if !env.has_command("npm") {
        return SmokeCheck::skipped(&id, "npm not installed");
    }
    let prefix = match env.output("npm", &["prefix", "-g"]) {
        Ok(res) if res.ok() => res.stdout.trim().to_string(),
        _ => return SmokeCheck::failed(&id, "npm: cannot resolve global prefix"),
    };
    let bin = std::path::PathBuf::from(&prefix).join("bin").join(name);
    if !is_executable(&bin) {
        return SmokeCheck::failed(&id, "npm: package binary not installed");
    }
    smoke_tool(env, &id, &bin.to_string_lossy(), &["--help"])
}

fn uv_python(env: &dotfiles_exec::ExecEnv) -> Option<String> {
    let out = env.output("uv", &["python", "find"]).ok()?;
    if !out.ok() {
        return None;
    }
    let p = out.stdout.trim().to_string();
    (!p.is_empty()).then_some(p)
}

fn smoke_pip(env: &dotfiles_exec::ExecEnv, name: &str) -> SmokeCheck {
    let id = format!("pip:{name}");
    if !env.has_command("uv") {
        return SmokeCheck::skipped(&id, "uv not installed");
    }
    let Some(python) = uv_python(env) else {
        return SmokeCheck::failed(&id, "uv: no managed python");
    };
    // importlib.metadata validates the *distribution* regardless of the
    // module's import name.
    let probe = format!("from importlib.metadata import version; print(version('{name}'))");
    match env.output(&python, &["-c", &probe]) {
        Err(e) => SmokeCheck::skipped(&id, format!("python failed to run: {e}")),
        Ok(res) if res.ok() => SmokeCheck::ok(&id),
        Ok(res) => {
            let detail = res.stderr.trim();
            let detail = detail.lines().last().unwrap_or("exited non-zero").trim();
            SmokeCheck::failed(&id, format!("python: {detail}"))
        }
    }
}

fn go_binary_name(spec: &str) -> String {
    let path = spec.split('@').next().unwrap_or(spec);
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn go_bin_dir(env: &dotfiles_exec::ExecEnv) -> std::path::PathBuf {
    let out = env.output("go", &["env", "GOBIN"]).ok();
    let gobin = out.map(|o| o.stdout.trim().to_string()).unwrap_or_default();
    if !gobin.is_empty() {
        return std::path::PathBuf::from(gobin);
    }
    let out = env.output("go", &["env", "GOPATH"]).ok();
    let gopath = out.map(|o| o.stdout.trim().to_string()).unwrap_or_default();
    if !gopath.is_empty() {
        return std::path::PathBuf::from(gopath).join("bin");
    }
    env.home.join("go/bin")
}

fn smoke_go(env: &dotfiles_exec::ExecEnv, spec: &str) -> SmokeCheck {
    let id = format!("go:{spec}");
    if !env.has_command("go") {
        return SmokeCheck::skipped(&id, "go not installed");
    }
    let bin = go_bin_dir(env).join(go_binary_name(spec));
    if !is_executable(&bin) {
        return SmokeCheck::failed(&id, "go: binary not installed");
    }
    smoke_tool(env, &id, &bin.to_string_lossy(), &["--help"])
}

fn smoke_node(env: &dotfiles_exec::ExecEnv) -> SmokeCheck {
    const ID: &str = "toolchain:node";
    if !env.has_command("fnm") {
        return SmokeCheck::skipped(ID, "fnm not installed");
    }
    // Validates the fnm-managed LTS end to end (works without `fnm env`).
    smoke_tool(
        env,
        ID,
        "fnm",
        &["exec", "--using=lts-latest", "node", "--help"],
    )
}

fn smoke_python(env: &dotfiles_exec::ExecEnv) -> SmokeCheck {
    const ID: &str = "toolchain:python";
    if !env.has_command("uv") {
        return SmokeCheck::skipped(ID, "uv not installed");
    }
    let Some(python) = uv_python(env) else {
        return SmokeCheck::failed(ID, "uv: no managed python");
    };
    smoke_tool(env, ID, &python, &["--help"])
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
        ctx.env = ctx.env.clone().with_isolated_base_paths(&[]);
        ctx
    }

    /// Real executable file under the sandbox that records its argv to `log`.
    fn fake_bin(t: &TestEnv, rel: &str, log: &str, exit: i32) -> std::path::PathBuf {
        let path = t.root().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!("#!/bin/sh\necho \"$@\" >> '{log}'\nexit {exit}\n"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn status_of(checks: &[SmokeCheck], id: &str) -> SmokeStatus {
        let c = checks
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("{id}"));
        match &c.status {
            SmokeStatus::Ok => SmokeStatus::Ok,
            SmokeStatus::Failed(r) => SmokeStatus::Failed(r.clone()),
            SmokeStatus::Skipped(r) => SmokeStatus::Skipped(r.clone()),
        }
    }

    fn is_ok(checks: &[SmokeCheck], id: &str) -> bool {
        matches!(status_of(checks, id), SmokeStatus::Ok)
    }

    #[test]
    fn everything_skips_without_tools() {
        let t = TestEnv::new();
        let ctx = ctx_with_manifest(
            &t,
            "install:\n  brew:\n    formulas: [jq]\n    casks: [caffeine]\n  gem:\n    rubygems: [neovim]\n  npm:\n    global:\n      packages: [prettier]\n  pip:\n    packages: [pynvim]\n  go:\n    packages: [\"example.com/x/tool@latest\"]\n  mas:\n    apps:\n      - { id: \"1\", name: A }\n  toolchains:\n    rustup: {}\n    node: {}\n    python: {}\n",
        );
        let checks = collect(&ctx);
        assert!(!checks.is_empty());
        assert!(!checks.iter().any(|c| c.is_failed()));
        assert!(checks.iter().any(|c| c.id == "brew-formula:jq"));
        assert!(checks.iter().any(|c| c.id == "toolchain:node"));
    }

    #[test]
    fn formula_invokes_prefix_bin_with_help() {
        let t = TestEnv::new();
        let log = t.root().join("argv.log");
        let bin = fake_bin(&t, "pfx/bin/jq", &log.display().to_string(), 0);
        assert!(is_executable(&bin));
        t.stub(
            "brew",
            &format!(
                "if [ \"$1\" = --prefix ]; then echo '{}'; exit 0; fi; exit 1",
                t.root().join("pfx").display()
            ),
        );
        let ctx = ctx_with_manifest(&t, "install:\n  brew:\n    formulas: [jq]\n");
        let checks = collect(&ctx);
        assert!(is_ok(&checks, "brew-formula:jq"));
        assert_eq!(
            std::fs::read_to_string(&log).unwrap().trim(),
            "--help",
            "binary invoked with --help"
        );
    }

    #[test]
    fn formula_uses_version_for_go_and_falls_back_to_first_bin() {
        let t = TestEnv::new();
        let log = t.root().join("argv.log");
        // No `bin/go` here — only another executable, which must be picked.
        fake_bin(&t, "pfx/bin/gofmt", &log.display().to_string(), 0);
        t.stub(
            "brew",
            &format!(
                "if [ \"$1\" = --prefix ]; then echo '{}'; exit 0; fi; exit 1",
                t.root().join("pfx").display()
            ),
        );
        let ctx = ctx_with_manifest(&t, "install:\n  brew:\n    formulas: [go]\n");
        let checks = collect(&ctx);
        assert!(is_ok(&checks, "brew-formula:go"));
        // Fallback binary still probed with go's `version` flags.
        assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "version");
        assert_eq!(formula_probe_args("go"), &["version"]);
        assert_eq!(formula_probe_args("jq"), &["--help"]);
    }

    #[test]
    fn formula_missing_or_binaryless() {
        let t = TestEnv::new();
        // --prefix fails → formula not installed → Failed.
        t.stub("brew", "if [ \"$1\" = --prefix ]; then exit 1; fi; exit 1");
        let ctx = ctx_with_manifest(
            &t,
            "install:\n  brew:\n    formulas: [ghost]\n    casks: [ghost-cask]\n",
        );
        let checks = collect(&ctx);
        assert!(matches!(
            status_of(&checks, "brew-formula:ghost"),
            SmokeStatus::Failed(_)
        ));
        // `brew list --cask` fails → Failed (never launches GUI bundles).
        assert!(matches!(
            status_of(&checks, "brew-cask:ghost-cask"),
            SmokeStatus::Failed(_)
        ));
        assert!(t
            .calls_of("brew")
            .iter()
            .all(|c| !c.contains("open") && c != "ghost-cask"));
    }

    #[test]
    fn cask_ok_when_artifact_exists() {
        let t = TestEnv::new();
        let app = t.root().join("Applications/Caffeine.app");
        std::fs::create_dir_all(&app).unwrap();
        t.stub(
            "brew",
            &format!(
                "if [ \"$2\" = --cask ]; then echo '{}'; exit 0; fi; exit 1",
                app.display()
            ),
        );
        let ctx = ctx_with_manifest(&t, "install:\n  brew:\n    casks: [caffeine]\n");
        let checks = collect(&ctx);
        assert!(is_ok(&checks, "brew-cask:caffeine"));
    }

    #[test]
    fn gem_activates_spec_via_ruby() {
        let t = TestEnv::new();
        t.stub("gem", "exit 0");
        t.stub(
            "ruby",
            "case \"$*\" in *neovim*) exit 0 ;; *) echo 'first-line-error' 1>&2; echo 'traceback-tail' 1>&2; exit 1 ;; esac",
        );
        let ctx = ctx_with_manifest(&t, "install:\n  gem:\n    rubygems: [neovim, ghost]\n");
        let checks = collect(&ctx);
        assert!(is_ok(&checks, "gem:neovim"));
        // Ruby backtraces: the failure detail is the first stderr line.
        match status_of(&checks, "gem:ghost") {
            SmokeStatus::Failed(reason) => {
                assert!(reason.contains("first-line-error"), "{reason}");
                assert!(!reason.contains("traceback-tail"), "{reason}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(t.calls_of("ruby").iter().all(|c| c.starts_with("-e ")));
    }

    #[test]
    fn npm_uses_global_prefix_bin() {
        let t = TestEnv::new();
        let log = t.root().join("argv.log");
        fake_bin(&t, "nprefix/bin/prettier", &log.display().to_string(), 0);
        t.stub(
            "npm",
            &format!(
                "if [ \"$1\" = prefix ]; then echo '{}'; exit 0; fi; exit 1",
                t.root().join("nprefix").display()
            ),
        );
        let ctx = ctx_with_manifest(
            &t,
            "install:\n  npm:\n    global:\n      packages: [prettier]\n",
        );
        let checks = collect(&ctx);
        assert!(is_ok(&checks, "npm:prettier"));
        assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "--help");
    }

    #[test]
    fn pip_validates_distribution_via_uv_python() {
        let t = TestEnv::new();
        let py = fake_bin(&t, "upy/bin/python3", "/dev/null", 0);
        t.stub("uv", &format!("echo '{}'; exit 0", py.display()));
        let ctx = ctx_with_manifest(&t, "install:\n  pip:\n    packages: [pynvim]\n");
        let checks = collect(&ctx);
        assert!(is_ok(&checks, "pip:pynvim"));
        let calls = t.calls_of("uv");
        assert_eq!(calls, vec!["python find"]);
    }

    #[test]
    fn go_invokes_gobin_binary() {
        let t = TestEnv::new();
        let log = t.root().join("argv.log");
        fake_bin(&t, "gobin/ulid", &log.display().to_string(), 0);
        t.stub(
            "go",
            &format!(
                "if [ \"$1 $2\" = \"env GOBIN\" ]; then echo '{}'; exit 0; fi; exit 1",
                t.root().join("gobin").display()
            ),
        );
        let ctx = ctx_with_manifest(
            &t,
            "install:\n  go:\n    packages: [\"github.com/oklog/ulid/v2/cmd/ulid@latest\"]\n",
        );
        let checks = collect(&ctx);
        assert!(is_ok(
            &checks,
            "go:github.com/oklog/ulid/v2/cmd/ulid@latest"
        ));
        assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "--help");
        assert_eq!(
            go_binary_name("github.com/oklog/ulid/v2/cmd/ulid@latest"),
            "ulid"
        );
    }

    #[test]
    fn toolchains_smoke_their_binaries() {
        let t = TestEnv::new();
        t.stub_ok("rustc", "rustc 1.0");
        t.stub("fnm", "exit 0");
        let py = fake_bin(&t, "upy/bin/python3", "/dev/null", 0);
        t.stub("uv", &format!("echo '{}'; exit 0", py.display()));
        let ctx = ctx_with_manifest(
            &t,
            "install:\n  toolchains:\n    rustup: {}\n    node: {}\n    python: {}\n",
        );
        let checks = collect(&ctx);
        assert!(is_ok(&checks, "toolchain:rustup"));
        assert!(is_ok(&checks, "toolchain:node"));
        assert!(is_ok(&checks, "toolchain:python"));
        assert!(t.calls_of("rustc").iter().any(|c| c == "--help"));
        assert!(t.calls_of("fnm").iter().any(|c| c.contains("node")));
    }

    #[test]
    fn report_bails_on_failure() {
        let t = TestEnv::new();
        t.stub("brew", "exit 1");
        let ctx = ctx_with_manifest(&t, "install:\n  brew:\n    formulas: [ghost]\n");
        let checks = collect(&ctx);
        assert!(print_report(&checks).is_err());
        let ok_checks = vec![SmokeCheck::ok("brew-formula:jq")];
        assert!(print_report(&ok_checks).is_ok());
    }
}
