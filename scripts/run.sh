#!/usr/bin/env bash

# Dependencies
source $(dirname $0)/support-print.sh

JOBS=(
    "software_update"
    "install_dependencies"
    "install_apps"
    "configure_apps"
    "apply_preferences"
    "update_history_commands"
)

SKIPPED_JOBS=" $(echo ${SKIP_JOBS:-} | tr ',' ' ') "

for job in "${JOBS[@]}"; do
    if [[ "${SKIPPED_JOBS}" == *" ${job} "* ]]; then
        print::section "Skipping job \"${job}\""
        print::info "Remove \"${job}\" from the SKIP_JOBS environment variable (space or comma separated) to run it."
        continue
    fi

    ./scripts/${job//_/-}.sh
done
