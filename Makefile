
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
