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

- **No shell scripts anywhere.** All logic is Rust. Never add `.sh` files,
  inline shell-outs in build scripts, or shell one-liners as a substitute
  for real implementation.
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
  manifest/   apps.yaml + commands.yaml types, validation, JSON Schema
  backends/   PackageBackend trait + brew/cask/mas/gem/npm/pip/cargo/go/composer + toolchains + bootstrap
  prefs/      declarative preferences engine (defaults/exec/builtins, apply/diff)
  core/       upgrade pipeline state machine (gates, probes, steps, reports)
  dotfiles/   the CLI binary (+ egui GUI behind the default `gui` feature)
  testkit/    test fixtures (stub binaries with argv recording)
schema/       generated JSON Schemas (committed, CI-enforced freshness)
```

## CLI surface (orientation)

- `dotfiles sync [--only <job>] [--skip <jobs>] [--sandbox]` — full pipeline:
  bootstrap → install → apply → prefs → history
- `dotfiles install [pkg...]`, `remove`, `search`, `info`, `list`, `update`,
  `upgrade` (apt-like package ops; `--gate/--headless/--dry-run` on upgrade)
- `dotfiles bootstrap|apply|history|software-update|doctor`
- `dotfiles prefs apply|diff|validate`
- `dotfiles agent install|status|uninstall|tick` (LaunchAgent, gated upgrades)
- `dotfiles schema --kind <apps|prefs> [--write]`
