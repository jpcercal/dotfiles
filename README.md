# dotfiles

A universal **macOS** package & configuration manager — one Rust binary that
 plays the role of `apt`, `brew`, `mas`, `composer`, `cargo`, `npm`, `pip`,
 `go install`, Ansible-style configuration, and macOS preference management in
 a single, idempotent, testable tool. No shell scripts as files: everything is
 Rust, driven by two declarative manifests (`apps.yaml`, `prefs.yaml`) validated
 against generated JSON Schemas (`schema/`). Lifecycle hook snippets are `sh -c`
 through the exec seam.

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
dotfiles install                   # everything declared in apps.yaml (parallel DAG engine; --jobs N/--sequential)
dotfiles install cask:iterm2 mas:1352778147
dotfiles uninstall brew:git
dotfiles update                    # refresh indexes (brew update, …)
dotfiles upgrade                   # upgrade all backends (has --gate/--headless/--dry-run/GUI)
```

## The full pipeline (used to be `make`)

```bash
dotfiles sync                      # bootstrap → install → prefs → history
dotfiles sync --skip prefs,history
dotfiles sync --only install
dotfiles sync --sandbox            # full E2E + stub tools + temp HOME (zero real effects)
```

Individual jobs are also commands:

```bash
dotfiles bootstrap                 # install Homebrew + taps
dotfiles install                   # apps from apps.yaml (idempotent; runs config hooks)
dotfiles prefs apply|diff|validate # ~190 declarative macOS preferences (defaults/pmset/dock/login items)
dotfiles history seed              # seed atuin history from commands.yaml
dotfiles software-update           # macOS updates (manual only, reboots!)
dotfiles doctor                    # environment diagnosis
dotfiles verify                    # every apps.yaml reference exists upstream and is wired correctly
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

- **`apps.yaml`** — packages via `require` (`brew-formula:`, `brew-cask:`,
  `brew-tap:`, `mas:`, `gem:`, `npm:`, `pip:`, `cargo:`, `go:`,
  `custom:`), language toolchains (rustup via `custom:rustup` hooks, node via
  `fnm` post-install hook, python via `uv` post-install hook). Each entry may carry
  `requires:` edges, `version:` pins (npm/pip/gem/cargo/go), and `hooks:`
  lifecycle snippets (`pre-install`, `post-install`, `pre-update`, `pre-uninstall`, etc.) executed via `sh -c`.
  Filesystem config (dirs, symlinks, dock) is declarative `post-install` hooks
  on the owning packages (`zsh`, `git`, `nvim`, `dockutil`, casks, MAS apps).
  Validated with [# yaml-language-server](schema/apps.schema.json).
- **`prefs.yaml`** — ~190 declarative macOS preferences: typed `defaults`
  entries (bool/int/float/string/array/dict, `current_host`, `sudo`,
  `-dict-add` merge mode), whitelisted `exec` steps (pmset/nvram/PlistBuddy/…),
  and builtins (`login-item`, `restart-apps`). Guardrails: `prefs validate` in
  CI, `prefs apply` is idempotent and non-fatal (parity with the old script),
  `prefs diff` is the drift gate.

## Development

Build, test, and contribution conventions live in [AGENTS.md](AGENTS.md) —
including the exact CI verification commands, the execution-seam and
sandbox rules, and the workspace layout.
