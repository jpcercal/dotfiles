use crate::outcome::BackendOutcome;
use anyhow::Result;
use dotfiles_exec::ExecEnv;

/// Language toolchains (rustup / fnm-node / uv-python). Not a PackageBackend —
/// toolchains are ensured and upgraded, not "package installed".
pub struct Toolchain;

pub const RUSTUP_INIT_URL_BASE: &str = "https://static.rust-lang.org/rustup/dist";

impl Toolchain {
    /// Ensure rustup is installed; downloads the rustup-init *binary* (not the
    /// shell bootstrap) and executes it.
    pub fn ensure_rustup(env: &ExecEnv, channel: &str) -> Result<BackendOutcome> {
        let mut out = BackendOutcome::empty("brew"); // backend label overridden below
        out.backend = "toolchain:rustup";
        if env.has_command("rustup") {
            out.unchanged.push("rustup".into());
            return Ok(out);
        }
        let arch = std::env::consts::ARCH;
        let url = format!("{}/{}-apple-darwin/rustup-init", RUSTUP_INIT_URL_BASE, arch);
        let tmp = std::env::temp_dir().join("dotfiles-rustup-init");
        let dl = env.output(
            "curl",
            &[
                "-fSL",
                "--proto",
                "=https",
                "--tlsv1.2",
                "-o",
                tmp.to_str().unwrap(),
                &url,
            ],
        )?;
        if !dl.ok() {
            anyhow::bail!("rustup-init download failed: {}", dl.stderr.trim());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        let tool = tmp.to_string_lossy().to_string();
        let run = env.output(
            &tool,
            &["-y", "--default-toolchain", channel, "--profile", "default"],
        )?;
        let _ = std::fs::remove_file(&tmp);
        if !run.ok() {
            anyhow::bail!("rustup-init failed: {}", run.stderr.trim());
        }
        out.changed.push(format!("rustup ({})", channel));
        Ok(out)
    }

    pub fn upgrade_rustup(env: &ExecEnv) -> Result<BackendOutcome> {
        let mut out = BackendOutcome {
            backend: "toolchain:rustup",
            ..Default::default()
        };
        if !env.has_command("rustup") {
            out.unchanged.push("rustup not installed".into());
            return Ok(out);
        }
        let res = env.output("rustup", &["update"])?;
        if res.ok() {
            out.changed.push("rustup update".into());
        } else {
            out.note = res.stderr.trim().to_string();
        }
        Ok(out)
    }

    /// Ensure the latest Node LTS via fnm.
    pub fn ensure_node(env: &ExecEnv) -> Result<BackendOutcome> {
        let mut out = BackendOutcome {
            backend: "toolchain:node",
            ..Default::default()
        };
        if !env.has_command("fnm") {
            anyhow::bail!("fnm not installed — install formula 'fnm' first");
        }
        let install = env.output("fnm", &["install", "--lts"])?;
        if !install.ok() {
            anyhow::bail!("fnm install --lts failed: {}", install.stderr.trim());
        }
        let default = env.output("fnm", &["default", "lts-latest"])?;
        if !default.ok() {
            anyhow::bail!("fnm default lts-latest failed: {}", default.stderr.trim());
        }
        out.changed.push("node lts-latest".into());
        Ok(out)
    }

    /// Ensure uv-managed python exists.
    pub fn ensure_python(env: &ExecEnv) -> Result<BackendOutcome> {
        let mut out = BackendOutcome {
            backend: "toolchain:python",
            ..Default::default()
        };
        if !env.has_command("uv") {
            anyhow::bail!("uv not installed — install formula 'uv' first");
        }
        let res = env.output("uv", &["python", "install"])?;
        if res.ok() {
            out.changed.push("python (uv)".into());
        } else {
            anyhow::bail!("uv python install failed: {}", res.stderr.trim());
        }
        Ok(out)
    }

    pub fn upgrade_python(env: &ExecEnv) -> Result<BackendOutcome> {
        Self::ensure_python(env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotfiles_testkit::TestEnv;

    #[test]
    fn ensure_rustup_is_noop_when_rustup_present() {
        let t = TestEnv::new();
        t.stub_ok("rustup", "rustup 1.27");
        let out = Toolchain::ensure_rustup(t.exec(), "stable").unwrap();
        assert_eq!(out.unchanged, vec!["rustup"]);
        assert!(t.calls().is_empty());
    }

    #[test]
    fn ensure_rustup_downloads_binary_and_runs_it() {
        let t = TestEnv::new();
        // curl "writes" the binary: our fake is a script that greps nothing.
        t.stub(
            "curl",
            "printf '#!/bin/sh\\necho fake-rustup-init\\nexit 0\\n' > \"$6\"; exit 0",
        );
        let out = Toolchain::ensure_rustup(t.exec(), "stable").unwrap();
        assert_eq!(out.changed, vec!["rustup (stable)"]);
        let curl_call = t.calls_of("curl").into_iter().next().expect("curl call");
        assert!(
            curl_call.contains("https://static.rust-lang.org/rustup/dist/"),
            "{}",
            curl_call
        );
        assert!(curl_call.ends_with("rustup-init"), "{}", curl_call);
    }

    #[test]
    fn ensure_node_runs_fnm_lts_flow() {
        let t = TestEnv::new();
        t.stub_ok("fnm", "");
        Toolchain::ensure_node(t.exec()).unwrap();
        assert_eq!(
            t.calls_of("fnm"),
            vec!["install --lts", "default lts-latest"]
        );
    }
}
