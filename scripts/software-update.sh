#!/usr/bin/env bash

# NOTE: This script is intentionally MANUAL-ONLY. Its `--restart` flag reboots the
# machine and must never be driven by the dotfiles-updater LaunchAgent.

# Dependencies
source $(dirname $0)/support-require-sudo.sh
source $(dirname $0)/support-keep-alive.sh
source $(dirname $0)/support-print.sh

# ------------------------------------------------------------------------------

print::title "Software Update"
print::title_paragraph "Install all appropriate macOS software updates, automatically restart (or shut down) if required to complete installation process."
print::title_paragraph "Note that, if after installing a software update it requires a restart to complete the process, then once your machine reboots you will have to run the script manually again. Keep calm as this is the only step that requires you to be in front of your machine."

print::section "Verifying and Updating macOS"
print::section_paragraph "Verbose mode will be enabled, so you can follow the installation report status in here."

print::command "sudo softwareupdate --install --restart --all --agree-to-license --verbose"
