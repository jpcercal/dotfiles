#!/usr/bin/env bash

# Dependencies
source $(dirname $0)/support-keep-alive.sh
source $(dirname $0)/support-print.sh

# ------------------------------------------------------------------------------

print::title "Patch ZSH History Commands"
print::title_paragraph "This script will update the entries of zsh history commands."
print::title_paragraph "It will do that by inserting any commands found on the \`~/dotfiles/commands.yaml\` file onto a sqlite database."
print::title_paragraph "This local database will later be used by zsh history in order to show all the possible commands that got executed allowing you to filter out and select commands easily on a zsh session."

# ------------------------------------------------------------------------------

ATUIN_HISTORY_DB=~/.local/share/atuin/history.db

if [[ ! -f ${ATUIN_HISTORY_DB} ]]; then
    print::info "The atuin history database \"${ATUIN_HISTORY_DB}\" does not exist yet, start the atuin daemon once before running this job. Skipping."
    exit 0
fi

COMMANDS=$(cat $(dirname $0)/../commands.yaml)

homedir=$(eval echo ~$USER)
hostname=$(hostname):$(whoami)
session=${ATUIN_SESSION}

# ------------------------------------------------------------------------------

for section in $(yq -r '. | keys' <<< "$COMMANDS" | awk '{ print $2}'); do
    section_description=$(yq -r ".[\"${section}\"].description" <<< "$COMMANDS")

    print::section "${section}"
    print::section_paragraph "${section_description}"

    for i in $(seq 0 $(($(yq -r ".[\"${section}\"].commands | length" <<< "$COMMANDS") - 1))); do
        description=$(yq -r ".[\"${section}\"].commands[${i}].description" <<< "$COMMANDS")
        command=$(yq -r ".[\"${section}\"].commands[${i}].command" <<< "$COMMANDS" | tr '\n' ' ' | tr -s ' ' | sed 's/"/\\"/g' | sed 's/ *$//g')

        sql=$(cat <<EOF
.parameter init
.parameter set @id        "$(ulid)"
.parameter set @timestamp "$(gdate +%s%N)"
.parameter set @duration  "-1"
.parameter set @exit      "-1"
.parameter set @command   "${command}"
.parameter set @cwd       "${homedir}"
.parameter set @session   "${session}"
.parameter set @hostname  "${hostname}"

BEGIN TRANSACTION; 

INSERT INTO history (
    id, 
    timestamp, 
    duration, 
    exit, 
    command, 
    cwd, 
    session, 
    hostname,
    deleted_at
) SELECT 
    @id, 
    @timestamp, 
    @duration, 
    @exit, 
    @command, 
    @cwd, 
    @session, 
    @hostname,
    NULL
WHERE NOT EXISTS (
    SELECT id FROM history WHERE command = @command
); 

COMMIT;
EOF
)
        print::info "${description}"
        echo "\$ ${command}"
        echo "${sql}"
        echo "${sql}" | sqlite3 ${ATUIN_HISTORY_DB}
    done
done

# ------------------------------------------------------------------------------
# Clean-up the dirty command used to execute this script

cat <<EOF | sqlite3 ${ATUIN_HISTORY_DB}
.parameter init
.parameter set @command "%%dotfiles/scripts/$(basename $0)"
.parameter set @session "${session}"

BEGIN TRANSACTION; 

DELETE FROM history 
WHERE 
    command like @command 
AND session = @session;

COMMIT;
EOF
