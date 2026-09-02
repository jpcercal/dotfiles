#!/usr/bin/env bash

# Dependencies
source $(dirname $0)/support-require-sudo.sh
source $(dirname $0)/support-keep-alive.sh
source $(dirname $0)/support-print.sh

# ------------------------------------------------------------------------------

print::title "Install Dependencies"
print::title_paragraph "This script will make the installation of dependencies."
print::title_paragraph "This step is mandatory and required by the other jobs to run successfully."

print::section "Installing Homebrew"
print::section_paragraph "Grab a cup of coffee and relax, this script does not expect you to give any data input."

print::command "/bin/bash -c \"\$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"" "Installing homebrew." "0"
echo "y" | /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

print::command "(echo; echo 'eval \"$(/opt/homebrew/bin/brew shellenv)\"') >> ~/.zprofile && eval \"$(/opt/homebrew/bin/brew shellenv)\"" "Adding homebrew to the \$PATH environment variable."
print::command "attempt=0; success=1; while [ \$attempt -lt 6 ]; do if brew update; then success=0; break; fi; attempt=\$((attempt + 1)); sleep 15; done; [ \$success -eq 0 ]"
print::command "brew install yq"

print::section "Adding Homebrew Third-Party Repositories"
print::section_paragraph "The brew tap command adds more repositories to the list of formulae that Homebrew tracks, updates, and installs from."
print::section_paragraph "A tap is Homebrew-speak for a Git repository containing additional formulae."

for tapBase64Encoded in $(yq -r '.install.brew.taps .[] | @base64' apps.yaml); do
    print::command "brew tap $(echo ${tapBase64Encoded} | yq '. | @base64d')"
done

for tapBase64Encoded in $(yq -r '.install.brew.taps .[] | @base64' apps.yaml); do
    tap=$(echo ${tapBase64Encoded} | yq '. | @base64d')

    case "${tap}" in
        homebrew/*) ;;
        *) print::command "brew trust ${tap}" ;;
    esac
done
