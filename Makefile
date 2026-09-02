
export PATH := $(HOME)/.local/bin:/opt/homebrew/bin:$(HOME)/dotfiles/bin:$(PATH)

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

.PHONY: run
run:
	./scripts/run.sh

.PHONY: default
default: run
