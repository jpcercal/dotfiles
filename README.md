# dotfiles

A universal **macOS** package & configuration manager — one Rust binary that
plays the role of `apt`, `brew`, `mas`, `composer`, `cargo`, `npm`, `pip`,
`go install`, Ansible-style configuration, and macOS preference management in
a single, idempotent, testable tool. No shell scripts anywhere: everything is
Rust, driven by two declarative manifests (`apps.yaml`, `prefs.yaml`) validated
against generated JSON Schemas (`schema/`).

## Install the binary

```bash
cargo build --release
mkdir -p ~/.local/bin && cp target/release/dotfiles ~/.local/bin/
```

## Everyday commands (apt-like)

```bash
dotfiles search <query>            # search across all backends
dotfiles info brew:ripgrep         # package info (brew:/cask:/mas:/gem:/npm:/pip:/cargo:/go:/composer:)
dotfiles list --installed          # per backend; or --outdated
dotfiles install                   # everything declared in apps.yaml
dotfiles install cask:iterm2 mas:1352778147
dotfiles remove brew:git
dotfiles update                    # refresh indexes (brew update, …)
dotfiles upgrade                   # upgrade all backends (has --gate/--headless/--dry-run/GUI)
```

## The full pipeline (used to be `make`)

```bash
dotfiles sync                      # bootstrap → install → apply → prefs → history
dotfiles sync --skip prefs,history
dotfiles sync --only apply
dotfiles sync --sandbox            # full E2E + stub tools + temp HOME (zero real effects)
```

Individual jobs are also commands:

```bash
dotfiles bootstrap                 # install Homebrew + taps
dotfiles install                   # apps from apps.yaml (idempotent)
dotfiles apply                     # dirs, symlinks (.bkp backups), dock, shell, nvim plugins
dotfiles prefs apply|diff|validate # ~190 declarative macOS preferences (defaults/pmset/dock/login items)
dotfiles history seed              # seed atuin history from commands.yaml
dotfiles software-update           # macOS updates (manual only, reboots!)
dotfiles doctor                    # environment diagnosis
```

## Scheduled upgrades (LaunchAgent)

```bash
dotfiles agent install             # run `dotfiles upgrade --gate` every 6h + at login
dotfiles agent status
dotfiles agent uninstall
dotfiles agent tick                # one gated tick (what the agent itself runs)
```

The upgrade flow has pre-flight gates (power, network, disk, package-manager
locks, 24h cadence, 24h dialog cooldown), an egui consent/progress window
(terminal fallback), sudo via GUI askpass, JSON reports in
`~/dotfiles/logs/dotfiles-updater` (90-day retention) and state in
`~/.local/state/dotfiles-updater/state.json`. macOS system updates are only
listed, never auto-installed.

## Manifests

- **`apps.yaml`** — packages (brew taps/formulas/casks, gem, npm, pip/uv, go,
  mas), toolchains (rustup/node/python), typed bootstrap steps, plus config
  (`mkdir`, `symbolic_links`, `dockutil`). Validated with
  [# yaml-language-server](schema/apps.schema.json).
- **`prefs.yaml`** — ~190 declarative macOS preferences: typed `defaults`
  entries (bool/int/float/string/array/dict, `current_host`, `sudo`,
  `-dict-add` merge mode), whitelisted `exec` steps (pmset/nvram/PlistBuddy/…),
  and builtins (`login-item`, `restart-apps`). Guardrails: `prefs validate` in
  CI, `prefs apply` is idempotent and non-fatal (parity with the old script),
  `prefs diff` is the drift gate.

## Development

```bash
cargo test --workspace                 # all tests (≈90)
cargo nextest run --workspace --no-default-features   # CI mode: parallel, no GUI build
cargo clippy --workspace --all-targets -- -D warnings
cargo llvm-cov nextest -p dotfiles-exec -p dotfiles-manifest -p dotfiles-backends -p dotfiles-prefs --fail-under-lines 80
```

Repo layout:

```
crates/
  exec/       execution seam (real vs sandbox env, stubs, dry-run)
  manifest/   apps.yaml + commands.yaml types, validation, JSON Schema
  backends/   PackageBackend trait + brew/cask/mas/gem/npm/pip/cargo/go/composer + toolchains + bootstrap
  prefs/      declarative preferences engine (defaults/exec/builtins, apply/diff)
  core/       upgrade pipeline state machine (gates, probes, steps, reports)
  dotfiles/   the CLI binary (+ egui GUI behind the default `gui` feature)
  testkit/    test fixtures (stub binaries with argv recording)
schema/       generated JSON Schemas (committed, CI-enforced freshness)
```
