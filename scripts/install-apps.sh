#!/usr/bin/env bash

# Dependencies
source $(dirname $0)/support-keep-alive.sh
source $(dirname $0)/support-print.sh
source $(dirname $0)/support-require-sudo.sh

# ------------------------------------------------------------------------------

print::title "Install Apps"
print::title_paragraph "This script will make the installation of applications."
print::title_paragraph "It will download and install all the apps you love that are defined on the \`apps.yaml\` file defined on the root folder of this repository."
print::title_paragraph "I was wondering here... did you finish drinking your cup of coffee? Probably you did not, so go do something else while I keep working hard here on this process."
print::title_paragraph "Be aware that this step depends a lot on your internet connection, it might be it will take some minutes to complete, of course you can check the progress of it anytime you want."

print::command "brew update"

# ------------------------------------------------------------------------------

print::section "Installing Homebrew Formulas"
print::section_paragraph "Install software based on homebrew formulas."

for formulaBase64Encoded in $(yq -r '.install.brew.formulas .[] | @base64' apps.yaml); do
    print::command "brew install $(echo ${formulaBase64Encoded} | yq '. | @base64d')"
done

for commandBase64Encoded in $(yq -r '.install.brew.customCommands .[] | @base64' apps.yaml); do
    print::command "$(echo ${commandBase64Encoded} | yq '. | @base64d')"
done

# ------------------------------------------------------------------------------

print::section "Installing Homebrew Casks"
print::section_paragraph "Install software based on homebrew casks."
print::section_paragraph "A cask is a homebrew package definition that installs macOS native applications."

for caskBase64Encoded in $(yq -r '.install.brew.casks .[] | @base64' apps.yaml); do
    print::command "brew install --cask $(echo ${caskBase64Encoded} | yq '. | @base64d')"
done

# ------------------------------------------------------------------------------

print::section "Ruby Gems"
print::section_paragraph "Installing local or remote ruby gems." 

for gemBase64Encoded in $(yq -r '.install.gem.rubygems .[] | @base64' apps.yaml); do
    print::command "gem install $(echo ${gemBase64Encoded} | yq '. | @base64d')"
done

# ------------------------------------------------------------------------------

print::section "NPM Packages"
print::section_paragraph "Installing local or remote NPM packages globally." 

for packageBase64Encoded in $(yq -r '.install.npm.global.packages .[] | @base64' apps.yaml); do
    print::command "npm install -g $(echo ${packageBase64Encoded} | yq '. | @base64d')"
done

# ------------------------------------------------------------------------------

print::section "MAS (Mac Apple Store)"
print::section_paragraph "Installing macOS Apple Store applications." 

for i in $(seq 0 $(($(yq -r '.install.mas.apps | length' apps.yaml) - 1))); do
    appId=$(yq -r ".install.mas.apps[${i}].id" apps.yaml)
    appName=$(yq -r ".install.mas.apps[${i}].name" apps.yaml)
    print::command "mas install ${appId}" "Installing \"${appName}\""
done
