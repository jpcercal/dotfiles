# AGENTS.md

Machine-readable briefing for AI coding agents working in this repo.
Humans: see [README.md](README.md) for what this project is and how to use it.

## Project

Single Rust binary (`dotfiles`) that manages a macOS machine: packages
(Homebrew formulae/casks, MAS, gem, npm, pip/uv, cargo, go, composer),
toolchains (rustup/node/python), filesystem config (dirs, symlinks, dock,
shell, nvim), declarative macOS preferences (`prefs.yaml`), atuin history
seeding, and a gated scheduled-upgrade pipeline. Driven by two declarative
manifests (`apps.yaml`, `prefs.yaml`) validated against generated JSON
Schemas (`schema/`).

## Hard rules

- **No shell scripts as files.** All logic is Rust. Never add `.sh` files,
  inline shell-outs in build scripts, or shell one-liners as a substitute
  for real implementation. The sole exception is `install.require` lifecycle
  hook snippets: YAML-carried `sh -c` snippets (pre/post-install/update/uninstall)
  executed through the `dotfiles-exec` seam — reviewable, sandboxed, and stub-able
  (the `sh` stub records argv in tests/`sync --sandbox`), not committed as files.
- **Everything must be idempotent.** Install/apply/prefs/sync are safe to
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
  manifest/   apps.yaml + commands.yaml types, validation, JSON Schema (+ units: unit-ID namespace)
  backends/   PackageBackend trait + brew/cask/mas/gem/npm/pip/cargo/go/composer + toolchains + bootstrap
              + graph (manifest → DAG) + schedule (parallel ready-queue executor) + orchestrate (engine wiring)
  prefs/      declarative preferences engine (defaults/exec/builtins, apply/diff)
  core/       upgrade pipeline state machine (gates, probes, steps, reports)
  dotfiles/   the CLI binary (+ egui GUI behind the default `gui` feature)
  testkit/    test fixtures (stub binaries with argv recording)
schema/       generated JSON Schemas (committed, CI-enforced freshness)
e2e/          reduced fixture manifests for the real-machine CI E2E job
```

## Install engine (dependency graph + parallel scheduler)

`apps.yaml` is the source of truth for install dependencies. `install.require`
is a flat list of `driver:name` entries; each entry is either a bare string
(`- "brew-formula:git"`) or a detailed map
(`- { id: "brew-formula:phpstan", requires: ["brew-formula:php"], version: "1.0", hooks: { post-install: "echo hi" } }`)
declaring dependency-graph edges, version pins (npm/pip/gem/cargo/go only;
`@version` sugar in the id e.g. `npm:prettier@3` is normalized to `version`;
brew/mas pins are hard validation errors), and lifecycle hooks. Detailed,
version-pinned, hook-carrying, or referenced packages split out of their
backend's batched install into schedulable single units. Aliases
`tap:`/`formula:`/`cask:` normalize to `brew-tap:`/`brew-formula:`/`brew-cask:`.
Canonical unit IDs (`crates/manifest/src/units.rs`): `brew-formula:x`,
`brew-cask:x`, `brew-tap:o/r`, `mas:<id>` (with required `label:`), `gem:`,
`npm:`, `pip:`, `cargo:`, `go:`, `composer:`, `toolchain:rustup|node|python`,
`bootstrap:<step>`. Implicit edges (taps → brew units, toolchains → npm/pip,
tool binaries → bootstrap steps) live in `units::implicit_requires`;
validation rejects unknown targets and cycles. Hooks (`pre-install`,
`post-install`, `pre-update`, `post-update`, `pre-uninstall`,
`post-uninstall`) are `sh -c` snippets executed through the exec seam; they
fire only when the associated action actually changes state and are
idempotency-preserving (re-running without changes does not re-fire). `install.execution`
tunes the engine (`max_jobs`, per lock-class `locks`; `brew` capped at 1).
Execution: `graph::build` → `schedule::run` (`std::thread::scope` ready-queue;
failures block dependents as `skipped (blocked by …)`, never abort). CLI:
`install`/`sync` accept `--jobs <N>` / `--sequential` (legacy path:
`install_all_sequential`). `dotfiles install` specs also accept the
`brew-formula:`/`brew-cask:` aliases.

## CLI surface (orientation)

- `dotfiles sync [--only <job>] [--skip <jobs>] [--sandbox] [--jobs <N>] [--sequential]` — full pipeline:
  bootstrap → install → apply → prefs → history
- `dotfiles install [pkg...] [--jobs <N>] [--sequential]`, `uninstall`, `search`, `info`, `list`, `update`,
  `upgrade` (apt-like package ops; `--gate/--headless/--dry-run` on upgrade)
- `dotfiles bootstrap|apply|history|software-update|doctor`
- `dotfiles verify [--local-only]` — parallel read-only reference check: every
  apps.yaml formula/cask/tap/MAS id/gem/npm/pip/go module exists upstream and
  every symlink/dock reference resolves (exit non-zero on any miss)
- `dotfiles prefs apply|diff|validate`
- `dotfiles agent install|status|uninstall|tick` (LaunchAgent, gated upgrades)
- `dotfiles schema --kind <apps|prefs> [--write]`
