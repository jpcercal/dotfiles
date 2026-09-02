#!/usr/bin/env bash

# Dependencies
source $(dirname $0)/support-require-sudo.sh
source $(dirname $0)/support-keep-alive.sh
source $(dirname $0)/support-print.sh

# ------------------------------------------------------------------------------

print::title "Configure Apps"
print::title_paragraph "This script will bring some love to you and so apply the configuration to the apps that are now installed."
print::title_paragraph "We are almost finished with everything, how was the coffee?"

# ------------------------------------------------------------------------------

print::section "Changing the default terminal"
print::section_paragraph "Now we are going to change the default terminal so you will always have zsh as being one of your best friends in here."

print::info "Below you are going to see what's the current terminal assigned to your user \"${USER}\":"
print::command "dscl . -read /Users/${USER} UserShell"
print::command "sudo dscl . -create /Users/${USER} UserShell \"$(brew --prefix)/bin/zsh\""
print::command "dscl . -read /Users/${USER} UserShell"

# ------------------------------------------------------------------------------

print::section "Creating symbolic links"
print::section_paragraph "It's time to create symbolic links to the configuration files with your home user folder because we should not reinvent the wheel, right?"

for dirBase64Encoded in $(yq -r '.config.mkdir .[] | @base64' apps.yaml); do
    print::command "mkdir -p $(echo ${dirBase64Encoded} | yq '. | @base64d')"
done

for i in $(seq 0 $(($(yq -r '.config.symbolic_links | length' apps.yaml) - 1))); do
    from=$(yq -r ".config.symbolic_links[${i}].from.relative_path" apps.yaml)
    to=$(yq -r ".config.symbolic_links[${i}].to.absolute_path" apps.yaml)

    if [[ -e ${to} ]]; then
        print::command "mv ${to} ${to}.$(date +%Y.%m.%d).bkp"
    fi

    print::command "ln -sF $(pwd)/${from} ${to}"
done

# ------------------------------------------------------------------------------

print::section "Reorganizing the Dock"
print::section_paragraph "Let's increase your productivity a bit more by setting a custom applications order on the macOS Dock."

if [[ $(yq -r '.config.dockutil._before.reset' apps.yaml) = "true" ]]
then 
    print::command "defaults delete com.apple.dock"
    print::command "killall Dock"
fi

if [[ $(yq -r '.config.dockutil._before.removeAll' apps.yaml) = "true" ]]
then 
    print::command "dockutil --remove all"
fi

for i in $(seq 0 $(($(yq -r '.config.dockutil.add | length' apps.yaml) - 1))); do
    app=$(yq -r ".config.dockutil.add[${i}].app" apps.yaml)
    after=$(yq -r ".config.dockutil.add[${i}].after" apps.yaml)
    print::command "dockutil --add \"${app}\" --after \"${after}\""
done

print::command "killall Dock"
