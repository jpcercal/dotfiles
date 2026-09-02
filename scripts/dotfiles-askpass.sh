#!/bin/bash
# SUDO_ASKPASS helper — GUI dialog when brew needs sudo (e.g. discord cask).
# sudo invokes this as: askpass "Password:"  (prompt in $1) and sets SUDO_COMMAND.
set -u
PROMPT="${1:-Password:}"
# SUDO_COMMAND is set by sudo; fallback to parent command
CMD="${SUDO_COMMAND:-$(ps -o command= -p $PPID 2>/dev/null | sed -E 's/^[[:space:]]*//; s/[[:space:]]*$//' | head -1)}"
CMD="${CMD:-sudo operation}"
# Derive a human reason from the command
REASON="manage system services and install to /Applications"
case "$CMD" in
  *discord*|*Discord*) REASON="upgrade the Discord cask (removing launchctl service com.discord.discord.ShipIt and installing to /Applications)" ;;
  *docker*|*Docker*) REASON="upgrade Docker Desktop (managing system services and privileged helpers)" ;;
  *google-chrome*|*chrome*) REASON="upgrade Google Chrome (installing to /Applications and managing system services)" ;;
  *cask*|*brew*upgrade*) REASON="upgrade Homebrew casks that require system modifications" ;;
esac
# Escape for AppleScript string (backslash and quote)
esc() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }
PROMPT_ESC="$(esc "$PROMPT")"
CMD_ESC="$(esc "$CMD")"
REASON_ESC="$(esc "$REASON")"
osascript <<APPLESCRIPT
tell application "System Events"
  activate
  display dialog "dotfiles-updater needs your password to allow:\n\n\"$CMD_ESC\"\n\nThis is required to $REASON_ESC.\n\nPlease enter your password to proceed:" default answer "" with title "dotfiles-updater — sudo required" with hidden answer buttons {"Cancel", "OK"} default button "OK" with icon caution
  return text returned of result
end tell
APPLESCRIPT
