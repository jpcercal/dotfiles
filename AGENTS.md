# AGENTS.md

Machine-readable briefing for AI coding agents working in this repo.
Humans: see [README.md](README.md) for what this project is and how to use it.

## Project

Single Rust binary (`dotfiles`) that manages a macOS machine: packages
(Homebrew formulae/casks, MAS, gem, npm, pip/uv, cargo, go, composer),
language toolchains (rustup/node/python — converged via post-install hooks
on carrier formulae and `custom:` entries), filesystem config (dirs,
symlinks, dock, shell, nvim via post-install hooks), declarative macOS
preferences (`prefs.yaml`), atuin history seeding, and a gated
scheduled-upgrade pipeline. Driven by two declarative manifests
(`apps.yaml`, `prefs.yaml`) validated against generated JSON Schemas
(`schema/`).

## Hard rules

- **No shell scripts as files.** All logic is Rust. Never add `.sh` files,
  inline shell-outs in build scripts, or shell one-liners as a substitute
  for real implementation. The sole exception is `require` lifecycle
  hook snippets: YAML-carried `sh -c` snippets (pre/post-install/update/uninstall)
  executed through the `dotfiles-exec` seam — reviewable, sandboxed, and stub-able
  (the `sh` stub records argv in tests/`sync --sandbox`), not committed as files.
- **Everything must be idempotent.** Install/prefs/sync are safe to
  re-run; re-running must converge, not duplicate or error.
- **Never invoke real system tools directly.** All process execution goes
  through the `dotfiles-exec` seam (`Exec` trait: real vs sandbox env,
  dry-run, stubs). No `std::process::Command` outside that seam.
- **Tests must have zero real effects.** Use the sandbox env plus the
  `testkit` stub binaries (record argv, assert on invocations). Never touch
  the real `$HOME`, real package managers, or real macOS settings in tests.
- **Keep `apps.yaml` / `prefs.yaml` valid.** Both are schema-validated;
  `prefs validate` runs in CI and `prefs diff` is the drift gate.

## Build

```bash
cargo build --release
mkdir -p ~/.local/bin && cp target/release/dotfiles ~/.local/bin/
```

- Default features include the `gui` feature (egui consent/progress window).
- Headless/CI builds use `--no-default-features` (skips eframe/egui, ~halves
  compile time).

## Verify (CI parity)

CI (`.github/workflows/macos.yml`) runs these; match them exactly:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --no-default-features -- -D warnings
cargo nextest run --workspace --no-default-features --profile ci
cargo llvm-cov nextest --profile ci \
  -p dotfiles-exec -p dotfiles-manifest -p dotfiles-backends -p dotfiles-prefs \
  --fail-under-lines 80 --summary-only
```

Notes:

- `cargo-nextest` and `cargo-llvm-cov` are installed via `taiki-e/install-action`
  in CI; locally use `cargo install cargo-nextest cargo-llvm-cov` if missing.
- `dotfiles-core` is intentionally excluded from line coverage: its
  pipeline/probes execute real system tools and are covered by the
  `sync --sandbox` E2E test instead.

## End-to-end check

```bash
dotfiles sync --sandbox   # full pipeline + stub tools + temp HOME, zero real effects
```

Use this (not a real `sync`) to verify pipeline-level changes.

## Schemas

`schema/apps.schema.json` and `schema/prefs.schema.json` are generated and
**committed** (CI enforces freshness). After changing manifest or prefs
types, regenerate:

```bash
dotfiles schema --kind apps --write
dotfiles schema --kind prefs --write
```

(`apps.yaml` carries a `# yaml-language-server` directive pointing at
`schema/apps.schema.json`; keep it in sync.)

## Workspace layout

```
crates/
  exec/       execution seam (real vs sandbox env, stubs, dry-run)
              + report (Reporter/Event: the only user-feedback channel, sudo sniffing, streamed runs)
  manifest/   apps.yaml + commands.yaml types, validation, JSON Schema (+ units: unit-ID namespace)
  backends/   PackageBackend trait + brew/cask/mas/gem/npm/pip/cargo/go/composer + custom (hook carriers)
              + graph (manifest → DAG) + schedule (parallel ready-queue executor) + orchestrate (engine wiring)
  prefs/      declarative preferences engine (defaults/exec/builtins, apply/diff)
  core/       upgrade pipeline state machine (gates, probes, steps, reports)
  dotfiles/   the CLI binary (+ egui GUI behind the default `gui` feature)
              + term_report (TermReporter: sections, per-unit blocks, elevation notices)
  testkit/    test fixtures (stub binaries with argv recording)
schema/       generated JSON Schemas (committed, CI-enforced freshness)
e2e/          reduced fixture manifests for the real-machine CI E2E job
```

## User feedback (reporting seam + sudo consciousness)

- **Library crates never print.** All user-visible output from `exec` /
  `backends` / `prefs` flows as `report::Event`s through the `Reporter`
  carried by `ExecEnv` (`Arc`, survives clones and scheduler threads;
  default `NoopReporter`). The CLI installs `TermReporter`; tests use
  `RecordingReporter` or nothing.
- **Layout contract (buffered blocks, live status):** `Section` = job
  (`▶ install`), `Subsection` = backend group; units announce `→ id` live
  while commands/output accumulate per unit and flush as one grouped,
  indented block on `UnitFinished` (`✓`/`✗`). Everything is shown —
  successful blocks included, never gated behind verbosity flags. Colors via
  `owo-colors` (`supports-colors`), plain when piped or `NO_COLOR`.
- **Sudo is announced, every time, with reason.** `ExecEnv` sniffs `sudo`
  spawns (past sudo's own flags to the inner command) and emits
  `Event::Elevate { command, reason }`; callers that know *why* use
  `env.elevate(program, args, reason)` (reasoned announcement replaces the
  sniff — exactly once). Warmups are necessity-gated: `prefs apply` diffs
  first and only pre-caches when an elevated entry is out of sync;
  `install_all` skips the cask warmup when every cask is already installed.
  `software-update` / `cache clean` show the exact `sudo …` command in the
  confirmation prompt.

## Install engine (dependency graph + parallel scheduler)

`apps.yaml` is the source of truth for install dependencies. `require` is a
flat list of `driver:name` entries at the root level (no `install:` wrapper);
each entry is either a bare string (`- "brew-formula:git"`) or a detailed map
(`- { id: "brew-formula:phpstan", requires: ["brew-formula:php"], version: "1.0", hooks: { post-install: "echo hi" } }`)
declaring dependency-graph edges, version pins (npm/pip/gem/cargo/go only;
`@version` sugar in the id e.g. `npm:prettier@3` is normalized to `version`;
brew/mas pins are hard validation errors), and lifecycle hooks. Detailed,
version-pinned, hook-carrying, or referenced packages split out of their
backend's batched install into schedulable single units. Aliases
`tap:`/`formula:`/`cask:` normalize to `brew-tap:`/`brew-formula:`/`brew-cask:`.
Canonical unit IDs (`crates/manifest/src/units.rs`): `brew-formula:x`,
`brew-cask:x`, `brew-tap:o/r`, `mas:<id>`, `gem:`,
`npm:`, `pip:`, `cargo:`, `go:`, `composer:`, `custom:<step>`. Implicit edges
(taps → brew units, npm → `brew-formula:fnm`, pip → `brew-formula:uv`) live in
`units::implicit_requires`; validation rejects unknown targets and cycles.
Hooks (`pre-install`, `post-install`, `pre-update`, `post-update`,
`pre-uninstall`, `post-uninstall`) are `sh -c` snippets executed through the
exec seam. `post-install` hooks fire whenever the unit ends up present (newly
installed **or** already installed, with no failures) so filesystem/dock config
converges on every run; pre hooks fire only ahead of the associated action.
`pre-update`/`post-update` fire during `dotfiles update`/`upgrade`;
`pre-uninstall`/`post-uninstall` fire during `dotfiles uninstall` (for
`custom:` specs, the hooks ARE the action). Snippets must be
idempotency-preserving (e.g. `ln -sfn`, guarded `if` skips); never probe tool
availability (`command -v`) or swallow faults (`|| true`) — a failing hook
fails its unit with the hook's stderr, which fails install/sync. Multi-line
snippets start with `set -e` so the first fault aborts the snippet.
`execution` tunes the engine (`max_jobs`, per lock-class `locks`; `brew`
capped at 1). Execution: `graph::build` → `schedule::run`
(`std::thread::scope` ready-queue; failures block dependents as `skipped
(blocked by …)`, never abort). CLI: `install`/`sync` accept `--jobs <N>` /
`--sequential` (legacy path: `install_all_sequential`). `dotfiles install`
specs also accept the `brew-formula:`/`brew-cask:` aliases.

Language toolchains converge via hooks rather than a typed `toolchain:`
section: node LTS via the `brew-formula:fnm` post-install hook (`fnm install
--lts` + `fnm default lts-latest`), python via the `brew-formula:uv`
post-install hook (`uv python install`), and rustup via `custom:rustup` (curl
`sh.rustup.rs` installer in `post-install`, `rustup update` in `pre-update`,
`rustup self uninstall -y` in `pre-uninstall`). Implicit edges point `npm:*`
→ `brew-formula:fnm` and `pip:*` → `brew-formula:uv` directly.

The gated upgrade pipeline (`crates/core`) runs hardcoded ecosystem upgrades
(brew, mas, cargo, fnm, uv, etc.), then the CLI wrapper fires
`update_all_with_opts` — manifest-declared update hooks across the graph
(pre-update → action → post-update). This gives `custom:rustup`'s
`rustup update` a firing point in the agent-tick flow without any core↔manifest
coupling.

## CLI surface (orientation)

- `dotfiles sync [--only <job>] [--skip <jobs>] [--sandbox] [--jobs <N>] [--sequential]` — full pipeline:
  bootstrap → install → prefs → history
- `dotfiles install [pkg...] [--jobs <N>] [--sequential]`, `uninstall`, `search`, `info`, `list`, `update`,
  `upgrade` (apt-like package ops; `--gate/--headless/--dry-run` on upgrade)
- `dotfiles bootstrap|history|software-update|doctor`
- `dotfiles verify` — parallel read-only reference check: every
  apps.yaml formula/cask/tap/MAS id/gem/npm/pip/go module exists upstream
  (exit non-zero on any miss; filesystem/dock config now lives in hooks)
- `dotfiles prefs apply|diff|validate`
- `dotfiles agent install|status|uninstall|tick` (LaunchAgent, gated upgrades)
- `dotfiles schema --kind <apps|prefs> [--write]`
