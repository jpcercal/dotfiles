
export PATH := $(HOME)/.local/bin:/opt/homebrew/bin:$(HOME)/dotfiles/bin:$(HOME)/go/bin:$(PATH)

.PHONY: default
default: run

.PHONY: run
run:
	./scripts/run.sh

.PHONY: software_update
software_update:
	./scripts/software-update.sh

.PHONY: install_dependencies
install_dependencies:
	./scripts/install-dependencies.sh

.PHONY: install_apps
install_apps:
	./scripts/install-apps.sh

.PHONY: configure_apps
configure_apps:
	./scripts/configure-apps.sh

.PHONY: apply_preferences
apply_preferences:
	./scripts/apply-preferences.sh

.PHONY: update_history_commands
update_history_commands:
	./scripts/update-history-commands.sh

.PHONY: dotfiles_updater
dotfiles_updater:
	./scripts/dotfiles-updater.sh --foreground

.PHONY: install_dotfiles_updater_agent
install_dotfiles_updater_agent:
	-launchctl bootout gui/$$(id -u)/com.jpcercal.dotfiles.updater
	cp launchd/com.jpcercal.dotfiles.updater.plist $(HOME)/Library/LaunchAgents/
	launchctl bootstrap gui/$$(id -u) $(HOME)/Library/LaunchAgents/com.jpcercal.dotfiles.updater.plist
	launchctl enable gui/$$(id -u)/com.jpcercal.dotfiles.updater

# --- Rust (dotfiles binary) ---

.PHONY: build_dotfiles
build_dotfiles:
	cargo build --release

.PHONY: install_dotfiles
install_dotfiles:
	cargo build --release
	mkdir -p $(HOME)/.local/bin
	cp target/release/dotfiles $(HOME)/.local/bin/dotfiles
	@echo "installed $(HOME)/.local/bin/dotfiles — ensure it is on PATH"

.PHONY: dotfiles_upgrade
dotfiles_upgrade:
	./target/release/dotfiles upgrade

.PHONY: dotfiles_upgrade_headless
dotfiles_upgrade_headless:
	./target/release/dotfiles upgrade --headless

.PHONY: dotfiles_dry_run
dotfiles_dry_run:
	./target/release/dotfiles upgrade --dry-run

.PHONY: install_dotfiles_agent
install_dotfiles_agent:
	-launchctl bootout gui/$$(id -u)/com.jpcercal.dotfiles.updater
	mkdir -p $(HOME)/.local/bin
	cp target/release/dotfiles $(HOME)/.local/bin/dotfiles
	sed "s|__HOME__|$(HOME)|g" launchd/com.jpcercal.dotfiles.updater.rust.plist > /tmp/com.jpcercal.dotfiles.updater.plist
	cp /tmp/com.jpcercal.dotfiles.updater.plist $(HOME)/Library/LaunchAgents/com.jpcercal.dotfiles.updater.plist
	launchctl bootstrap gui/$$(id -u) $(HOME)/Library/LaunchAgents/com.jpcercal.dotfiles.updater.plist
	launchctl enable gui/$$(id -u)/com.jpcercal.dotfiles.updater
	@echo "Rust agent installed (dotfiles upgrade --gate)"
