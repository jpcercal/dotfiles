# dotfiles

Basically, here you can find the settings of OSX according to my preferences and automatic installation of applications from different sources. 

If you liked this idea then please don't forget to give me a star. =]

## How to run it?

Yeah, that's really simple, just run the following on your terminal:

```bash
make
```

It will run every job in order: `software_update`, `install_dependencies`, `install_apps`, `configure_apps`, `apply_preferences` and `update_history_commands`.

You can also run any job on its own:

```bash
make software_update
make install_dependencies
make install_apps
make configure_apps
make apply_preferences
make update_history_commands
make dotfiles_updater
make install_dotfiles_updater_agent
```

To skip one or more jobs while debugging, list them on the `SKIP_JOBS` environment variable (space or comma separated):

```bash
SKIP_JOBS="software_update" make
SKIP_JOBS="install_apps configure_apps" make
```

## dotfiles-updater

A daily (24h) system-update daemon with an interactive gate: every update requires
your explicit click — **there is no fully-automatic update path**.

```bash
make dotfiles_updater                # manual run in the terminal
make install_dotfiles_updater_agent  # install/refresh the LaunchAgent
```

- **Scheduling:** the LaunchAgent (`com.jpcercal.dotfiles.updater`) ticks every 6h and
  at login/boot (`RunAtLoad`), so runs missed while the Mac was off or asleep are
  caught up. A run is only offered when the last successful run is ≥ 24h old.
- **Dialog:** shown **at most once per day**; "Postpone until tomorrow" or leaving it
  unanswered defers to the next day. There are no postpone counters and no forced runs.
- **Pre-flight gates:** on AC power or battery ≥ 50%, network reachable, ≥ 10 GB free
  disk, no concurrent brew/mas process. Any failure silently skips to the next tick.
- **Scope:** brew formulae/casks, App Store (mas), rustup + cargo globals, composer
  global + audit, latest Node LTS via fnm (global packages migrated, older majors
  pruned), uv + pynvim/neovim, opencode, neovim vim-plug plugins, gem neovim, tmux TPM.
  macOS system updates are only *listed* and notified — never installed or rebooted.
- **State:** `~/.local/state/dotfiles-updater/state.json`
- **Reports:** `~/dotfiles/logs/dotfiles-updater/*.json` (per-step durations, old→new
  versions, failures, brew-deprecation + composer audit; 90-day retention)

**Pause the agent:**

```bash
launchctl bootout gui/$(id -u)/com.jpcercal.dotfiles.updater
```

**Resume the agent:**

```bash
make install_dotfiles_updater_agent
```

**Testing hooks:** `DFU_BATTERY_PCT` and `DFU_ON_AC=0|1` override the power gate in
all modes (e.g. `DFU_BATTERY_PCT=15 DFU_ON_AC=0 make dotfiles_updater`).
