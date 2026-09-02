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
print::command "brew update"
print::command "brew install yq"
