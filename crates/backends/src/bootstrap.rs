use crate::outcome::BackendOutcome;
use anyhow::Result;
use dotfiles_exec::ExecEnv;

/// Bootstrap steps are pure hook carriers: they have no built-in install
/// logic (everything lives in post-install hooks in `apps.yaml`), so every
/// step trivially converges and its hooks fire on every run. Any step id is
/// accepted; validation requires each entry to carry hooks.
pub fn run(name: &str, env: &ExecEnv) -> Result<BackendOutcome> {
    let _ = env;
    let mut out = BackendOutcome {
        backend: "bootstrap",
        ..Default::default()
    };
    out.unchanged.push(name.into());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotfiles_testkit::TestEnv;

    #[test]
    fn any_step_is_a_converged_hook_carrier() {
        // No registry: every step id is accepted, trivially present, so its
        // post-install hook (where the real work lives) fires every run.
        let t = TestEnv::new();
        let out = run("opencode", t.exec()).unwrap();
        assert_eq!(out.unchanged, vec!["opencode"]);
        assert!(out.ok());
        assert!(t.calls().is_empty());
        let out = run("anything-else", t.exec()).unwrap();
        assert_eq!(out.unchanged, vec!["anything-else"]);
    }
}
