use crate::model::{DefaultsValue, PrefEntry, PrefsFile, Typ};
use anyhow::Result;
use dotfiles_exec::ExecEnv;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefStatus {
    Applied,
    Unchanged,
    Failed(String),
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiffEntry {
    pub id: String,
    pub desired: String,
    pub current: Option<String>,
    pub status: DiffStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffStatus {
    InSync,
    Drifted,
    Unreadable,
}

#[derive(Debug, Default)]
pub struct ApplyReport {
    pub results: Vec<(String, PrefStatus)>,
}

impl ApplyReport {
    pub fn failures(&self) -> Vec<&(String, PrefStatus)> {
        self.results
            .iter()
            .filter(|(_, s)| matches!(s, PrefStatus::Failed(_)))
            .collect()
    }
}

/// The kill list from apply-preferences.sh, verbatim — apps that must be
/// restarted for `defaults` changes to take effect.
pub const RESTART_APPS: &[&str] = &[
    "Activity Monitor",
    "Address Book",
    "Calendar",
    "cfprefsd",
    "Contacts",
    "Dock",
    "Finder",
    "Google Chrome Canary",
    "Google Chrome",
    "Mail",
    "Messages",
    "Opera",
    "Photos",
    "Safari",
    "SizeUp",
    "Spectacle",
    "SystemUIServer",
    "Terminal",
    "Transmission",
    "Tweetbot",
    "Twitter",
    "iCal",
];

/// Apply every entry. Individual failures are recorded, not fatal — a
/// machine mid-bootstrap shouldn't abort the remaining prefs.
pub fn apply(env: &ExecEnv, file: &PrefsFile) -> Result<ApplyReport> {
    let mut report = ApplyReport::default();
    for entry in &file.prefs {
        let status = apply_one(env, entry).unwrap_or_else(|e| PrefStatus::Failed(e.to_string()));
        report.results.push((entry.id().to_string(), status));
    }
    Ok(report)
}

fn apply_one(env: &ExecEnv, entry: &PrefEntry) -> Result<PrefStatus> {
    match entry {
        PrefEntry::Defaults {
            domain,
            key,
            typ,
            value,
            add,
            current_host,
            sudo,
            ..
        } => {
            // add-mode (dict/merge) entries always write — the merge is idempotent
            // by construction, and diff-reads of a merged dict are ill-defined.
            if !*add {
                let cur = read_default(env, domain, key, *current_host)?;
                let desired = canonical(env, *typ, value)?;
                if cur.as_deref() == Some(desired.as_str()) {
                    return Ok(PrefStatus::Unchanged);
                }
            }
            write_default(
                env,
                domain,
                key,
                *typ,
                value,
                WriteOpts {
                    add: *add,
                    current_host: *current_host,
                    sudo: *sudo,
                },
            )?;
            Ok(PrefStatus::Applied)
        }
        PrefEntry::Exec {
            program,
            args,
            sudo,
            ignore_error,
            ..
        } => {
            let expanded: Vec<String> = args.iter().map(|a| expand_home(env, a)).collect();
            let argv: Vec<&str> = expanded.iter().map(String::as_str).collect();
            let res = run_maybe_sudo_output(env, program, &argv, *sudo)?;
            if !res.ok() && !*ignore_error {
                return Ok(PrefStatus::Failed(res.stderr.trim().to_string()));
            }
            Ok(PrefStatus::Applied)
        }
        PrefEntry::Builtin {
            name, app, hidden, ..
        } => {
            match name.as_str() {
                "restart-apps" => {
                    for app_name in RESTART_APPS {
                        // sudo killall … &> /dev/null (script parity): exits 1 for
                        // apps that aren't running — expected, ignored.
                        let _ = env.output("sudo", &["killall", app_name]);
                    }
                    Ok(PrefStatus::Applied)
                }
                "login-item" => {
                    let app = app.clone().unwrap_or_default();
                    let base = app
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or(&app)
                        .trim_end_matches(".app")
                        .to_string();
                    let list = env.output(
                    "osascript",
                    &["-e", "tell application \"System Events\" to get the name of every login item"],
                )?;
                    if list.ok() && list.stdout.split(", ").any(|n| n.trim() == base) {
                        return Ok(PrefStatus::Unchanged);
                    }
                    let script = format!(
                    "tell application \"System Events\" to make login item at end with properties {{path:\"{}\", hidden:{}}}",
                    app, hidden
                );
                    let out = env.output("osascript", &["-e", &script])?;
                    if out.ok() {
                        Ok(PrefStatus::Applied)
                    } else {
                        Ok(PrefStatus::Failed(out.stderr.trim().to_string()))
                    }
                }
                other => Ok(PrefStatus::Failed(format!("unknown builtin '{}'", other))),
            }
        }
    }
}

fn expand_home(env: &ExecEnv, s: &str) -> String {
    if !s.contains("~/") {
        return s.to_string();
    }
    s.replace("~/", &format!("{}/", env.home.display()))
}

/// Read-side diff (`prefs diff` / `--check`). Only `defaults` entries are
/// diffable; exec/builtin entries are reported as Unreadable.
pub fn diff(env: &ExecEnv, file: &PrefsFile) -> Result<Vec<DiffEntry>> {
    let mut out = vec![];
    for entry in &file.prefs {
        if let PrefEntry::Defaults {
            id,
            domain,
            key,
            typ,
            value,
            current_host,
            ..
        } = entry
        {
            let desired = canonical(env, *typ, value)?;
            match read_default(env, domain, key, *current_host)? {
                Some(cur) if cur == desired => out.push(DiffEntry {
                    id: id.clone(),
                    desired,
                    current: Some(cur),
                    status: DiffStatus::InSync,
                }),
                other => out.push(DiffEntry {
                    id: id.clone(),
                    desired,
                    current: other,
                    status: DiffStatus::Drifted,
                }),
            }
        }
    }
    Ok(out)
}

/// Canonical text a `defaults read` would print for this value.
pub fn canonical(env: &ExecEnv, typ: Typ, value: &DefaultsValue) -> Result<String> {
    let s = match (typ, value) {
        (Typ::Bool, DefaultsValue::Bool(b)) => if *b { "1" } else { "0" }.to_string(),
        (Typ::Int, DefaultsValue::Int(i)) => i.to_string(),
        (Typ::Float, DefaultsValue::Float(f)) => f.to_string(),
        (Typ::Float, DefaultsValue::Int(i)) => (*i as f64).to_string(), // YAML `0` parses as int; coerce
        (Typ::String, DefaultsValue::Str(s)) => expand_home(env, s),
        (Typ::Array, DefaultsValue::List(items)) => {
            let mut rendered = vec![];
            for i in items {
                rendered.push(format!("    {}", scalar_of(env, i)?));
            }
            format!("(\n{}\n)", rendered.join(",\n"))
        }
        (Typ::Dict, DefaultsValue::Map(m)) => {
            let mut rendered = vec![];
            for (k, v) in m {
                rendered.push(format!("    {} = {};", k, scalar_of(env, v)?));
            }
            format!("{{\n{}\n}}", rendered.join("\n"))
        }
        (typ, value) => anyhow::bail!("type/value mismatch: declared {:?}, got {:?}", typ, value),
    };
    Ok(s)
}

/// Render a scalar (non-container) value for dict/array position.
fn scalar_of(env: &ExecEnv, v: &DefaultsValue) -> Result<String> {
    Ok(match v {
        DefaultsValue::Bool(b) => b.to_string(),
        DefaultsValue::Int(i) => i.to_string(),
        DefaultsValue::Float(f) => f.to_string(),
        DefaultsValue::Str(s) => expand_home(env, s),
        other => anyhow::bail!(
            "nested containers not supported as dict/array elements: {:?}",
            other
        ),
    })
}

/// Append `-type value` args for a (possibly nested) value.
fn push_typed_value(
    args: &mut Vec<String>,
    flag: &str,
    env: &ExecEnv,
    value: &DefaultsValue,
) -> Result<()> {
    match value {
        DefaultsValue::Bool(b) => args.extend(["-bool".into(), b.to_string()]),
        DefaultsValue::Int(i) if flag == "float" => {
            args.extend(["-float".into(), (*i as f64).to_string()])
        } // YAML `0` parses as int; coerce
        DefaultsValue::Int(i) => args.extend(["-int".into(), i.to_string()]),
        DefaultsValue::Float(f) => args.extend(["-float".into(), f.to_string()]),
        DefaultsValue::Str(s) => args.extend(["-string".into(), expand_home(env, s)]),
        DefaultsValue::List(items) => {
            let mut rendered = vec![];
            for i in items {
                rendered.push(scalar_of(env, i)?);
            }
            args.push(format!("-{}", flag));
            args.extend(rendered);
        }
        DefaultsValue::Map(m) => {
            args.push(format!("-{}", flag));
            for (k, v) in m {
                args.push(k.clone());
                push_scalar_typed(args, env, v)?;
            }
        }
    }
    Ok(())
}

fn push_scalar_typed(args: &mut Vec<String>, env: &ExecEnv, v: &DefaultsValue) -> Result<()> {
    match v {
        DefaultsValue::Bool(b) => args.extend(["-bool".into(), b.to_string()]),
        DefaultsValue::Int(i) => args.extend(["-int".into(), i.to_string()]),
        DefaultsValue::Float(f) => args.extend(["-float".into(), f.to_string()]),
        DefaultsValue::Str(s) => args.extend(["-string".into(), expand_home(env, s)]),
        other => anyhow::bail!("dict values must be scalars: {:?}", other),
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct WriteOpts {
    add: bool,
    current_host: bool,
    sudo: bool,
}

fn write_default(
    env: &ExecEnv,
    domain: &str,
    key: &str,
    typ: Typ,
    value: &DefaultsValue,
    opts: WriteOpts,
) -> Result<()> {
    let mut args: Vec<String> = vec![];
    if opts.current_host {
        args.push("-currentHost".into());
    }
    args.push("write".into());
    args.push(domain.into());
    args.push(key.into());
    let flag = match typ {
        Typ::Bool => "bool",
        Typ::Int => "int",
        Typ::Float => "float",
        Typ::String => "string",
        Typ::Array => "array",
        Typ::Dict => "dict",
    };
    let shape_ok = matches!(
        (typ, value),
        (Typ::Array, DefaultsValue::List(_)) | (Typ::Dict, DefaultsValue::Map(_))
    ) || matches!(typ, Typ::Bool | Typ::Int | Typ::Float | Typ::String);
    if !shape_ok {
        anyhow::bail!("type/value mismatch: declared {:?}, got {:?}", typ, value);
    }
    if opts.add && (typ == Typ::Dict || typ == Typ::Array) {
        match value {
            DefaultsValue::Map(m) => {
                for (k, v) in m {
                    args.push("-dict-add".into());
                    args.push(k.clone());
                    push_scalar_typed(&mut args, env, v)?;
                }
            }
            DefaultsValue::List(items) => {
                for i in items {
                    args.push("-array-add".into());
                    args.push(scalar_of(env, i)?);
                }
            }
            _ => unreachable!(),
        }
    } else {
        push_typed_value(&mut args, flag, env, value)?;
    }
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    run_maybe_sudo(env, "defaults", &argv, opts.sudo)
}

fn read_default(
    env: &ExecEnv,
    domain: &str,
    key: &str,
    current_host: bool,
) -> Result<Option<String>> {
    let mut args: Vec<&str> = vec![];
    if current_host {
        args.push("-currentHost");
    }
    args.extend(["read", domain, key]);
    let out = env.output("defaults", &args)?;
    if !out.ok() {
        return Ok(None);
    }
    Ok(Some(normalize_read(&out.stdout)))
}

/// `defaults read` prints values with minor formatting differences from our
/// canonical form; normalize the common scalar cases.
fn normalize_read(raw: &str) -> String {
    let t = raw.trim();
    match t {
        "TRUE" | "YES" => "1".to_string(),
        "FALSE" | "NO" => "0".to_string(),
        _ => t.trim_matches('"').to_string(),
    }
}

fn run_maybe_sudo_output(
    env: &ExecEnv,
    program: &str,
    args: &[&str],
    sudo: bool,
) -> Result<dotfiles_exec::ExecOutput> {
    if sudo {
        let mut full: Vec<&str> = vec![program];
        full.extend_from_slice(args);
        Ok(env.output("sudo", &full)?)
    } else {
        env.output(program, args)
    }
}

fn run_maybe_sudo(env: &ExecEnv, program: &str, args: &[&str], sudo: bool) -> Result<()> {
    let out = run_maybe_sudo_output(env, program, args, sudo)?;
    if !out.ok() {
        anyhow::bail!("{} failed: {}", program, out.stderr.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_prefs;
    use dotfiles_testkit::TestEnv;

    #[test]
    fn canonical_renders_scalar_types() {
        let env = ExecEnv::real();
        assert_eq!(
            canonical(&env, Typ::Bool, &DefaultsValue::Bool(true)).unwrap(),
            "1"
        );
        assert_eq!(
            canonical(&env, Typ::Int, &DefaultsValue::Int(-1)).unwrap(),
            "-1"
        );
        assert_eq!(
            canonical(&env, Typ::String, &DefaultsValue::Str("Always".into())).unwrap(),
            "Always"
        );
    }

    #[test]
    fn apply_writes_only_drifted_defaults() {
        let t = TestEnv::new();
        // AppleShowScrollBars already "Always"; AppleMetricUnits is 0 (drifted)
        t.stub(
            "defaults",
            "if [ \"$1\" = read ]; then \
               case \"$3\" in \
                 AppleShowScrollBars) echo Always; exit 0 ;; \
                 AppleMetricUnits) echo 0; exit 0 ;; \
               esac; exit 1; fi; exit 0",
        );
        let file = parse_prefs(
            r#"
prefs:
  - { id: ui.scrollbars, kind: defaults, domain: NSGlobalDomain, key: AppleShowScrollBars, type: string, value: Always }
  - { id: ui.metric, kind: defaults, domain: NSGlobalDomain, key: AppleMetricUnits, type: bool, value: true }
"#,
        )
        .unwrap();
        let report = apply(t.exec(), &file).unwrap();
        assert_eq!(report.results[0].1, PrefStatus::Unchanged);
        assert_eq!(report.results[1].1, PrefStatus::Applied);
        let writes: Vec<String> = t
            .calls_of("defaults")
            .into_iter()
            .filter(|c| c.contains("write"))
            .collect();
        assert_eq!(
            writes,
            vec!["write NSGlobalDomain AppleMetricUnits -bool true"]
        );
    }

    #[test]
    fn diff_reports_in_sync_and_drifted() {
        let t = TestEnv::new();
        t.stub("defaults", "echo 1; exit 0");
        let file = parse_prefs(
            "prefs:\n  - { id: a, kind: defaults, domain: D, key: K1, type: bool, value: true }\n  - { id: b, kind: defaults, domain: D, key: K2, type: bool, value: false }\n",
        )
        .unwrap();
        let entries = diff(t.exec(), &file).unwrap();
        assert_eq!(entries[0].status, DiffStatus::InSync);
        assert_eq!(entries[1].status, DiffStatus::Drifted);
    }

    #[test]
    fn sudo_defaults_prefixed_with_sudo() {
        let t = TestEnv::new();
        t.stub("defaults", "if [ \"$1\" = read ]; then exit 1; fi; exit 0");
        t.stub_ok("sudo", "");
        let file = parse_prefs(
            "prefs:\n  - { id: x, kind: defaults, domain: /Library/Preferences/com.apple.loginwindow, key: DSBindTimeout, type: int, value: 5, sudo: true }\n",
        )
        .unwrap();
        apply(t.exec(), &file).unwrap();
        assert!(
            t.calls_of("sudo")
                .iter()
                .any(|c| c.starts_with("defaults write /Library/Preferences")),
            "{:?}",
            t.calls_of("sudo")
        );
    }

    #[test]
    fn exec_kind_invokes_whitelisted_program() {
        let t = TestEnv::new();
        t.stub_ok("pmset", "");
        t.stub_ok("sudo", "");
        let file = parse_prefs(
            "prefs:\n  - { id: power.sleep, kind: exec, program: pmset, args: [-a, sleep, \"0\"], sudo: true }\n",
        )
        .unwrap();
        apply(t.exec(), &file).unwrap();
        assert_eq!(t.calls_of("sudo"), vec!["pmset -a sleep 0"]);
    }

    #[test]
    fn login_item_added_once() {
        let t = TestEnv::new();
        // first call = list (name absent) → add; second apply: name present → skip
        t.stub(
            "osascript",
            "case \"$2\" in *'every login item'*) \
               if [ -f \"$HOME/litems\" ]; then cat \"$HOME/litems\"; fi; exit 0 ;; \
             *) echo 'Alfred' > \"$HOME/litems\"; exit 0 ;; esac",
        );
        let file = parse_prefs(
            "prefs:\n  - { id: login.alfred, kind: builtin, name: login-item, app: /Applications/Alfred.app }\n",
        )
        .unwrap();
        let report = apply(t.exec(), &file).unwrap();
        assert_eq!(report.results[0].1, PrefStatus::Applied);
        let report2 = apply(t.exec(), &file).unwrap();
        assert_eq!(report2.results[0].1, PrefStatus::Unchanged);
    }

    #[test]
    fn restart_apps_kills_all_listed() {
        let t = TestEnv::new();
        t.stub_ok("sudo", "");
        let file =
            parse_prefs("prefs:\n  - { id: dock.restart, kind: builtin, name: restart-apps }\n")
                .unwrap();
        apply(t.exec(), &file).unwrap();
        let calls = t.calls_of("sudo");
        assert!(calls.contains(&"killall Dock".to_string()));
        assert!(calls.contains(&"killall Finder".to_string()));
        assert!(calls.contains(&"killall cfprefsd".to_string()));
        assert_eq!(calls.len(), RESTART_APPS.len());
    }

    #[test]
    fn value_type_mismatch_is_a_failure_not_a_panic() {
        let t = TestEnv::new();
        t.stub_ok("defaults", "");
        let file = parse_prefs(
            "prefs:\n  - { id: bad, kind: defaults, domain: D, key: K, type: bool, value: \"notabool\" }\n",
        )
        .unwrap();
        let report = apply(t.exec(), &file).unwrap();
        assert!(matches!(report.results[0].1, PrefStatus::Failed(_)));
    }
}
