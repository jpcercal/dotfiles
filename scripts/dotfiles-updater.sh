#!/usr/bin/env bash

set -uo pipefail

LABEL="com.jpcercal.dotfiles.updater"
STATE_DIR="$HOME/.local/state/dotfiles-updater"
STATE_FILE="$STATE_DIR/state.json"
LOCK_DIR="$STATE_DIR/lock"
LOG_DIR="$HOME/dotfiles/logs/dotfiles-updater"

MIN_BATTERY=50
MIN_DISK_GB=10
CADENCE=86400
DIALOG_COOLDOWN=86400
RETENTION_DAYS=90

export PATH="$HOME/.local/bin:/opt/homebrew/bin:/opt/homebrew/sbin:$HOME/dotfiles/bin:$HOME/.cargo/bin:$HOME/.opencode/bin:/usr/local/bin:/usr/bin:/bin"

# Test hooks honored in ALL modes: DFU_BATTERY_PCT, DFU_ON_AC=0|1

RUN_ID="$(date +%Y%m%d%H%M%S)-$$"
RUN_STARTED_EPOCH="$(date +%s)"
RUN_STARTED_ISO="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
REPORT_PATH="$LOG_DIR/$(date +%Y-%m-%dT%H-%M-%S).json"

iso_now() { date -u +%Y-%m-%dT%H:%M:%SZ; }

log() { echo "[$(iso_now)] $*"; }

# Timeboxed command execution (gtimeout from coreutils; plain run as fallback)
tmo() {
    local secs="$1"; shift
    if command -v gtimeout >/dev/null 2>&1; then
        gtimeout "$secs" "$@"
    else
        "$@"
    fi
}

# ------------------------------------------------------------------------------
# State file helpers
# ------------------------------------------------------------------------------

state_init() {
    mkdir -p "$STATE_DIR" "$LOG_DIR"
    if [[ ! -f "$STATE_FILE" ]]; then
        jq -n '{last_attempt_at:null,last_success_at:null,last_dialog_at:null,last_failed_steps:[],last_outcome:null}' > "$STATE_FILE"
    fi
}

state_get() {
    jq -r --arg k "$1" '.[$k] // empty' "$STATE_FILE" 2>/dev/null
}

state_set_str() {
    jq --arg k "$1" --arg v "$2" '.[$k] = $v' "$STATE_FILE" > "$STATE_FILE.tmp" && mv "$STATE_FILE.tmp" "$STATE_FILE"
}

state_set_num() {
    jq --arg k "$1" --argjson v "$2" '.[$k] = $v' "$STATE_FILE" > "$STATE_FILE.tmp" && mv "$STATE_FILE.tmp" "$STATE_FILE"
}

state_set_arr() {
    jq --arg k "$1" --argjson v "$2" '.[$k] = $v' "$STATE_FILE" > "$STATE_FILE.tmp" && mv "$STATE_FILE.tmp" "$STATE_FILE"
}

# ------------------------------------------------------------------------------
# Lock (mkdir-based, no flock on macOS)
# ------------------------------------------------------------------------------

lock_acquire() {
    if mkdir "$LOCK_DIR" 2>/dev/null; then
        echo "$$" > "$LOCK_DIR/pid"
        return 0
    fi
    local pid
    pid="$(cat "$LOCK_DIR/pid" 2>/dev/null || echo '')"
    if [[ -n "$pid" ]] && ! kill -0 "$pid" 2>/dev/null; then
        log "stale lock detected (pid $pid), reaping"
        rm -rf "$LOCK_DIR"
        lock_acquire
        return $?
    fi
    return 1
}

lock_release() { rm -rf "$LOCK_DIR"; }

on_exit() {
    if [[ -n "${DRY_MODE:-}" ]]; then
        return 0
    fi
    state_set_num last_attempt_at "$(date +%s)"
    lock_release
}

on_signal() {
    if [[ -n "${DRY_MODE:-}" ]]; then
        exit 1
    fi
    state_set_str last_outcome interrupted
    lock_release
    exit 1
}

trap on_exit EXIT
trap on_signal INT TERM

# ------------------------------------------------------------------------------
# Gates
# ------------------------------------------------------------------------------

battery_info() {
    local out
    out="$(pmset -g batt 2>/dev/null || true)"
    if echo "$out" | grep -q "AC Power"; then ON_AC=1; else ON_AC=0; fi
    BATTERY_PCT="$(echo "$out" | grep -oE '[0-9]{1,3}%' | head -1 | tr -d '%' || true)"
    [[ -z "$BATTERY_PCT" ]] && BATTERY_PCT=100
    [[ -n "${DFU_ON_AC:-}" ]] && ON_AC="$DFU_ON_AC"
    [[ -n "${DFU_BATTERY_PCT:-}" ]] && BATTERY_PCT="$DFU_BATTERY_PCT"
}

gate_power() {
    battery_info
    if [[ "$ON_AC" == "1" ]]; then
        GATE_POWER_REASON="ok: on AC power (battery ${BATTERY_PCT}%)"
        return 0
    fi
    if (( BATTERY_PCT >= MIN_BATTERY )); then
        GATE_POWER_REASON="ok: battery ${BATTERY_PCT}% (>= ${MIN_BATTERY}%)"
        return 0
    fi
    GATE_POWER_REASON="skip: battery ${BATTERY_PCT}% below ${MIN_BATTERY}% and not on AC"
    return 1
}

gate_network() {
    if curl -fsSI --max-time 5 https://formulae.brew.sh >/dev/null 2>&1 \
       && curl -fsSI --max-time 5 https://www.apple.com/library/test/success.html >/dev/null 2>&1; then
        GATE_NETWORK_REASON="ok: brew CDN and Apple CDN reachable"
        return 0
    fi
    GATE_NETWORK_REASON="skip: network unreachable"
    return 1
}

gate_disk() {
    FREE_DISK_GB="$(df -g / | awk 'NR==2 {print $4}')"
    if [[ -n "$FREE_DISK_GB" ]] && (( FREE_DISK_GB >= MIN_DISK_GB )); then
        GATE_DISK_REASON="ok: ${FREE_DISK_GB}GB free (>= ${MIN_DISK_GB}GB)"
        return 0
    fi
    GATE_DISK_REASON="skip: only ${FREE_DISK_GB:-unknown}GB free (need ${MIN_DISK_GB}GB)"
    return 1
}

gate_pkgmgr() {
    if pgrep -f '/(brew|mas)( |$)' >/dev/null 2>&1; then
        GATE_PKGMGR_REASON="skip: another brew/mas process is running"
        return 1
    fi
    GATE_PKGMGR_REASON="ok: no brew/mas process running"
    return 0
}

gate_schedule() {
    local now last_success
    now="$(date +%s)"
    last_success="$(state_get last_success_at)"
    if [[ -z "$last_success" || "$last_success" == "null" ]] || (( now - last_success >= CADENCE )); then
        GATE_SCHEDULE_REASON="ok: last success ${last_success:-never} (due)"
        return 0
    fi
    GATE_SCHEDULE_REASON="skip: ran successfully $(state_get last_success_at) (< ${CADENCE}s ago)"
    return 1
}

gate_dialog_cooldown() {
    local now last_dialog
    now="$(date +%s)"
    last_dialog="$(state_get last_dialog_at)"
    if [[ -z "$last_dialog" || "$last_dialog" == "null" ]] || (( now - last_dialog >= DIALOG_COOLDOWN )); then
        GATE_DIALOG_REASON="ok: no dialog in the last 24h"
        return 0
    fi
    GATE_DIALOG_REASON="skip: dialog already shown within 24h (once-per-day cap)"
    return 1
}

gate_env() {
    local f
    for f in gate_power gate_network gate_disk gate_pkgmgr; do
        if ! "$f"; then
            log "gate $f: $(print_gate_reason "$f")"
            return 1
        fi
        log "gate $f: $(print_gate_reason "$f")"
    done
    return 0
}

print_gate_reason() {
    case "$1" in
        gate_power) echo "$GATE_POWER_REASON" ;;
        gate_network) echo "$GATE_NETWORK_REASON" ;;
        gate_disk) echo "$GATE_DISK_REASON" ;;
        gate_pkgmgr) echo "$GATE_PKGMGR_REASON" ;;
        gate_schedule) echo "$GATE_SCHEDULE_REASON" ;;
        gate_dialog_cooldown) echo "$GATE_DIALOG_REASON" ;;
    esac
}

# ------------------------------------------------------------------------------
# Notifications & dialog
# ------------------------------------------------------------------------------

as_escape() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }

notify() {
    local title="$1" msg="$2" subtitle="${3:-}"
    if command -v terminal-notifier >/dev/null 2>&1; then
        terminal-notifier -title "$title" -message "$msg" \
            ${subtitle:+-subtitle "$subtitle"} -sound default 2>/dev/null || true
    else
        osascript -e "display notification \"$(as_escape "$msg")\" with title \"$(as_escape "$title")\"" 2>/dev/null || true
    fi
}

show_dialog() {
    local summary="$1"
    # Stamp BEFORE showing: covers dismiss/locked-screen/logout/never-answered.
    state_set_num last_dialog_at "$(date +%s)"
    local answer
    answer="$(osascript -l JavaScript "$(dirname "$0")/dotfiles-updater-dialog.jxa" "$summary" 2> /tmp/dotfiles-updater-dialog.err)"
    local osascript_status=$?
    printf 'dialog raw answer=[%s] status=%d err=[%s]\n' "$answer" "$osascript_status" "$(tr '\n' ' ' < /tmp/dotfiles-updater-dialog.err 2>/dev/null)" >> /tmp/dotfiles-updater-debug.log
    # Trim whitespace/newlines for robust comparison
    answer="$(printf '%s' "$answer" | tr -d '\r' | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//')"
    if (( osascript_status != 0 )); then
        return 1
    fi
    [[ "$answer" == "Update Now" ]]
}

# ------------------------------------------------------------------------------
# Probes (dry-run data for the dialog summary)
# ------------------------------------------------------------------------------

# section_add <title> <count> <status-when-updates> <status-when-empty> [items...]
SUMMARY_BUF=""
section_add() {
    local title="$1" count="$2" status_up="$3" status_empty="$4"; shift 4
    SUMMARY_BUF+="$title ($count)"$'\n'
    if (( count > 0 )); then
        SUMMARY_BUF+="=> $status_up"$'\n'
        local it
        for it in "$@"; do SUMMARY_BUF+="* $it"$'\n'; done
    else
        SUMMARY_BUF+="=> $status_empty"$'\n'
    fi
    SUMMARY_BUF+=$'\n'
}

probe_brew() {
    if ! command -v brew >/dev/null 2>&1; then
        section_add "Brew" 0 "Updates available" "Unavailable (not installed)"
        return
    fi
    local brew_json f_items=() c_items=()
    brew_json="$(brew outdated --json=v2 2>/dev/null || echo '{"formulae":[],"casks":[]}')"
    while IFS= read -r l; do f_items+=("$l"); done \
        < <(echo "$brew_json" | jq -r '.formulae[] | "\(.name) (\(.installed_versions[0]) → \(.current_version))"' 2>/dev/null)
    while IFS= read -r l; do c_items+=("$l"); done \
        < <(echo "$brew_json" | jq -r '.casks[] | "\(.name) (\(.installed_versions[0]) → \(.current_version))"' 2>/dev/null)
    section_add "Brew Formulae" "${#f_items[@]}" "Updates available" "No updates available" "${f_items[@]}"
    section_add "Brew Casks" "${#c_items[@]}" "Updates available" "No updates available" "${c_items[@]}"
}

probe_mas() {
    if ! command -v mas >/dev/null 2>&1; then
        section_add "MAS" 0 "Updates available" "Unavailable (not installed)"
        return
    fi
    local mas_out items=()
    if mas_out="$(mas outdated 2>/dev/null)"; then
        while IFS= read -r l; do
            items+=("$(echo "$l" | sed -E 's/^[0-9]+[[:space:]]+//; s/[[:space:]]+\(([^ ]+) -> ([^)]+)\)$/ (\1 → \2)/')")
        done < <(printf '%s\n' "$mas_out" | grep -E '^[0-9]+')
        section_add "MAS" "${#items[@]}" "Updates available" "No updates available" "${items[@]}"
    else
        section_add "MAS" 0 "Updates available" "Unavailable (App Store session)"
    fi
}

probe_rust() {
    if ! command -v rustup >/dev/null 2>&1; then
        section_add "Rust" 0 "Updates available" "Unavailable (not installed)"
        return
    fi
    local items=()
    while IFS= read -r l; do
        items+=("$(echo "$l" | sed -E 's/^[[:space:]]*//; s/ - Update available : ([^ ]+)([[:space:]]+\(.*\))? -> ([^ ]+)([[:space:]]+\(.*\))?/ (\1 → \3)/')")
    done < <(tmo 30 rustup check 2>/dev/null | grep -E "Update available")
    section_add "Rust" "${#items[@]}" "Updates available" "No updates available" "${items[@]}"
}

probe_node() {
    if ! command -v fnm >/dev/null 2>&1; then
        section_add "Node (fnm)" 0 "Updates available" "Unavailable (not installed)"
        return
    fi
    eval "$(fnm env 2>/dev/null)"
    local cur target nglobals items=() c l
    cur="$(fnm current 2>/dev/null || true)"
    target="$(tmo 30 fnm ls-remote --lts 2>/dev/null | tail -1 | sed -E 's/[[:space:]].*//' || true)"
    # npm outdated exits 1 when outdated; jq -s also flattens npm's stray docs
    nglobals="$(npm outdated -g --json 2>/dev/null | jq -s '.[0] // {}')"
    if [[ -n "$target" && -n "$cur" && "$cur" != "$target" && "$cur" != "system" && "$cur" != "default" ]]; then
        items+=("node ($cur → $target)")
    fi
    for p in $(echo "$nglobals" | jq -r 'keys[]' 2>/dev/null); do
        c="$(echo "$nglobals" | jq -r --arg p "$p" '.[$p].current')"
        l="$(echo "$nglobals" | jq -r --arg p "$p" '.[$p].latest')"
        items+=("$p ($c → $l)")
    done
    section_add "Node (fnm)" "${#items[@]}" "Updates available" "No updates available" "${items[@]}"
}

probe_python() {
    if ! command -v uv >/dev/null 2>&1; then
        section_add "Python (uv)" 0 "Updates available" "Unavailable (not installed)"
        return
    fi
    local uv_ver uv_latest py pip_check items=() pnv pnv_new nv nv_new
    uv_ver="$(uv --version 2>/dev/null | awk '{print $2}' || true)"
    uv_latest="$(curl -fsSL --max-time 8 https://pypi.org/pypi/uv/json 2>/dev/null | jq -r '.info.version' 2>/dev/null || true)"
    if [[ -n "$uv_ver" && -n "$uv_latest" && "$uv_ver" != "$uv_latest" ]]; then
        items+=("uv ($uv_ver → $uv_latest)")
    fi
    py="$(uv python find 2>/dev/null || true)"
    pip_check="$(uv pip install --dry-run -U --break-system-packages --python "$py" pynvim neovim 2>&1 || true)"
    pnv="$(uv pip list --python "$py" 2>/dev/null | awk '/^pynvim[[:space:]]/ {print $2}')"
    nv="$(uv pip list --python "$py" 2>/dev/null | awk '/^neovim[[:space:]]/ {print $2}')"
    pnv_new="$(echo "$pip_check" | sed -nE 's/^[[:space:]]*\+ pynvim==([^ ]+).*/\1/p' | head -1)"
    nv_new="$(echo "$pip_check" | sed -nE 's/^[[:space:]]*\+ neovim==([^ ]+).*/\1/p' | head -1)"
    if [[ -n "$pnv" && -n "$pnv_new" && "$pnv" != "$pnv_new" ]]; then
        items+=("pynvim ($pnv → $pnv_new)")
    fi
    if [[ -n "$nv" && -n "$nv_new" && "$nv" != "$nv_new" ]]; then
        items+=("neovim ($nv → $nv_new)")
    fi
    section_add "Python (uv)" "${#items[@]}" "Updates available" "No updates available" "${items[@]}"
}

probe_opencode() {
    if ! command -v opencode >/dev/null 2>&1; then
        section_add "opencode" 0 "Updates available" "Unavailable (not installed)"
        return
    fi
    local cur latest items=()
    cur="$(opencode --version 2>/dev/null | head -1 || true)"
    latest="$(curl -fsSL --max-time 8 https://registry.npmjs.org/opencode-ai/latest 2>/dev/null | jq -r '.version' 2>/dev/null || true)"
    if [[ -n "$cur" && -n "$latest" && "$cur" != "$latest" ]]; then
        items+=("opencode ($cur → $latest)")
    fi
    section_add "opencode" "${#items[@]}" "Updates available" "No updates available" "${items[@]}"
}

probe_nvim() {
    if ! command -v nvim >/dev/null 2>&1; then
        section_add "Neovim Plugins" 0 "Checked at run time" "Unavailable (not installed)"
        return
    fi
    local items=()
    while IFS= read -r l; do
        items+=("$(echo "$l" | sed -E "s/^[[:space:]]*Plug[[:space:]]*'([^']+)'.*/\1/; s/^[[:space:]]*Plug[[:space:]]*\"([^\"]+)\".*/\1/")")
    done < <(grep -E '^[[:space:]]*Plug[[:space:]]+["'"'"']' "$HOME/.vimrc" 2>/dev/null)
    section_add "Neovim Plugins" "${#items[@]}" "Checked at run time (results in the JSON report)" "No updates available" "${items[@]}"
}

probe_gem() {
    if ! command -v gem >/dev/null 2>&1; then
        section_add "Gem" 0 "Updates available" "Unavailable (not installed)"
        return
    fi
    local line items=()
    line="$(tmo 30 gem outdated 2>/dev/null | grep -E '^neovim ' || true)"
    if [[ -n "$line" ]]; then
        items+=("$(echo "$line" | sed -E 's/^([^ ]+) \(([^ ]+) < ([^)]+)\).*/\1 (\2 → \3)/')")
    fi
    section_add "Gem" "${#items[@]}" "Updates available" "No updates available" "${items[@]}"
}

probe_macos() {
    local items=()
    while IFS= read -r l; do
        items+=("$(echo "$l" | sed -E 's/^\* Label: //')")
    done < <(tmo 60 softwareupdate --list 2>/dev/null | grep '^\* Label:')
    section_add "macOS" "${#items[@]}" "Updates available (install manually via System Settings)" "No updates available" "${items[@]}"
}

probe_tpm() {
    if [[ ! -d "$HOME/.tmux/plugins" ]]; then
        return
    fi
    local items=()
    for d in "$HOME/.tmux/plugins"/*; do
        [[ -d "$d/.git" ]] && items+=("$(basename "$d")")
    done
    if (( ${#items[@]} > 0 )); then
        section_add "Tmux TPM" "${#items[@]}" "Checked at run time — $(IFS=,; echo "${items[*]}")" "No plugins"
    fi
}

probe_summary() {
    SUMMARY_BUF=""
    probe_brew
    probe_mas
    probe_rust
    probe_node
    probe_python
    probe_opencode
    probe_nvim
    probe_gem
    probe_macos
    probe_tpm
    printf '%s' "$SUMMARY_BUF" | sed 's/[[:space:]]*$//'
}

# ------------------------------------------------------------------------------
# Pipeline runner
# ------------------------------------------------------------------------------

STEP_CODE=0
STEP_LOG=""
STEP_DURATION=0

run_step() {
    local name="$1"; shift
    local start end
    STEP_LOG="$LOG_DIR/$RUN_ID.$name.log"
    start="$(date +%s)"
    # Use line-buffered output so live logs appear immediately (brew buffers when piped).
    local _buf=""
    if command -v gstdbuf >/dev/null 2>&1; then _buf="gstdbuf -oL -eL";
    elif command -v stdbuf >/dev/null 2>&1; then _buf="stdbuf -oL -eL"; fi
    # In foreground (user's terminal) use `script` to give brew a real pty so
    # sudo's Password: prompt appears and accepts typing — unless SUDO_ASKPASS
    # is set (brew cask upgrades), then use the GUI prompt instead.
    if [[ -t 0 && -e /dev/tty ]]; then
        if [[ -n "${SUDO_ASKPASS:-}" ]]; then
            {
                printf '\n▶ %s  %s\n' "$name" "$(date '+%H:%M:%S')"
                printf '# %s\n' "$*"
                if [[ -n "$_buf" ]]; then $_buf "$@" 2>&1; else "$@" 2>&1; fi
            } 2>&1 | ${_buf:+$_buf }tee "$STEP_LOG" | ${_buf:+$_buf }tee -a "$LOG_DIR/$RUN_ID.combined.log"
            STEP_CODE=${PIPESTATUS[0]}
        else
            {
                printf '\n▶ %s  %s\n' "$name" "$(date '+%H:%M:%S')"
                printf '# %s\n' "$*"
                if command -v script >/dev/null 2>&1; then
                    script -q -F /dev/null "$@" 2>&1
                elif [[ -n "$_buf" ]]; then
                    $_buf "$@" 2>&1 < /dev/tty
                else
                    "$@" 2>&1 < /dev/tty
                fi
            } 2>&1 | ${_buf:+$_buf }tee "$STEP_LOG" | ${_buf:+$_buf }tee -a "$LOG_DIR/$RUN_ID.combined.log"
            STEP_CODE=${PIPESTATUS[0]}
        fi
    else
        {
            printf '\n▶ %s  %s\n' "$name" "$(date '+%H:%M:%S')"
            printf '# %s\n' "$*"
            if [[ -n "$_buf" ]]; then $_buf "$@" 2>&1; else "$@" 2>&1; fi
        } 2>&1 | ${_buf:+$_buf }tee "$STEP_LOG" | ${_buf:+$_buf }tee -a "$LOG_DIR/$RUN_ID.combined.log"
        STEP_CODE=${PIPESTATUS[0]}
    fi
    end="$(date +%s)"
    STEP_DURATION=$((end - start))
}

emit_step() {
    # $1=name $2=status $3=updated_json $4=failed_json $5=note
    mkdir -p "$RUN_TMP/steps"
    jq -n \
        --arg name "$1" --arg status "$2" \
        --argjson duration "$STEP_DURATION" \
        --argjson updated "$3" --argjson failed "$4" \
        --arg note "${5:-}" --arg raw_log "$RUN_ID.$1.log" \
        '{name:$name,status:$status,duration_seconds:$duration,updated:$updated,failed:$failed,note:$note,raw_log:$raw_log}' \
        > "$RUN_TMP/steps/$1.json"
}

parse_brew_upgraded() {
    awk '/^==> Upgrading / { n=$3 }
         /->/ && $0 ~ /^  / { s=$0; sub(/^[ \t]*/, "", s); split(s, a, " -> ");
             gsub(/[ \t]+$/, "", a[2]); print n "\t" a[1] "\t" a[2] }' "$1" \
        | while IFS=$'\t' read -r n o t; do
            jq -n --arg name "$n" --arg from "$o" --arg to "$t" \
                '{name:$name,from:$from,to:$to}'
          done | jq -s '. // []'
}

step_brew() {
    SUDO_ASKPASS="$HOME/dotfiles/scripts/dotfiles-askpass.sh" run_step brew bash -c 'HOMEBREW_NO_COLOR=1 HOMEBREW_NO_ASK=1 brew update && HOMEBREW_NO_COLOR=1 HOMEBREW_NO_ASK=1 brew upgrade -y && HOMEBREW_NO_COLOR=1 HOMEBREW_NO_ASK=1 brew upgrade --cask --greedy=false -y && brew autoremove && brew cleanup'
    local updated
    updated="$(parse_brew_upgraded "$STEP_LOG")"
    if (( STEP_CODE == 0 )); then
        emit_step brew success "$updated" '[]' ""
    else
        emit_step brew failed "$updated" '[]' "brew exited ${STEP_CODE}"
    fi
}

step_rtk() {
    local note=""
    if ! command -v rtk >/dev/null 2>&1; then
        STEP_DURATION=0
        emit_step rtk-repatch skipped '[]' '[]' "rtk not installed"
        return
    fi
    if [[ ! -s "$RUN_TMP/rtk-changed" ]]; then
        STEP_DURATION=0
        emit_step rtk-repatch skipped '[]' '[]' "rtk version unchanged"
        return
    fi
    run_step rtk-repatch rtk init -g --opencode --auto-patch
    if (( STEP_CODE == 0 )); then
        emit_step rtk-repatch success '[]' '[]' "opencode re-patched"
    else
        emit_step rtk-repatch failed '[]' '[]' "rtk init exited ${STEP_CODE}"
    fi
}

step_mas() {
    if ! command -v mas >/dev/null 2>&1; then
        STEP_DURATION=0
        emit_step mas skipped '[]' '[]' "mas not installed"
        return
    fi
    local before="$RUN_TMP/mas-before.txt" after="$RUN_TMP/mas-after.txt"
    if ! mas outdated > "$before" 2>&1; then
        STEP_DURATION=0
        emit_step mas skipped '[]' '[]' "App Store session unavailable"
        return
    fi
    run_step mas mas upgrade
    mas outdated > "$after" 2>&1 || true
    local updated
    updated="$(comm -23 \
        <(sed -E 's/^[0-9]+[[:space:]]+//; s/[[:space:]]+\([0-9.]+\)$//' "$before" | sort) \
        <(sed -E 's/^[0-9]+[[:space:]]+//; s/[[:space:]]+\([0-9.]+\)$//' "$after" | sort) \
        | while read -r n; do jq -n --arg name "$n" '{name:$name}'; done | jq -s '. // []')"
    if (( STEP_CODE == 0 )); then
        emit_step mas success "$updated" '[]' ""
    else
        emit_step mas failed "$updated" '[]' "mas exited ${STEP_CODE}"
    fi
}

step_rust() {
    if ! command -v rustup >/dev/null 2>&1; then
        STEP_DURATION=0
        emit_step rust skipped '[]' '[]' "rustup not installed"
        return
    fi
    run_step rust rustup update
    if (( STEP_CODE != 0 )); then
        emit_step rust failed '[]' '[]' "rustup exited ${STEP_CODE}"
        return
    fi
    local updated="[]" note=""
    if cargo install-update --list >/dev/null 2>&1; then
        local log2="$LOG_DIR/$RUN_ID.rust-cargo.log"
        local start2 end2 code2
        start2="$(date +%s)"
        cargo install-update -a > "$log2" 2>&1
        code2=$?
        end2="$(date +%s)"
        STEP_DURATION=$((end2 - start2))
        if (( code2 == 0 )); then
            updated="$(awk '/Updating / {print $2}' "$log2" | while read -r n; do jq -n --arg name "$n" '{name:$name}'; done | jq -s '. // []')"
            note=""
        else
            emit_step rust failed "$updated" '[]' "cargo install-update exited ${code2}"
            return
        fi
    else
        note="cargo-update not installed, cargo globals skipped"
    fi
    emit_step rust success "$updated" '[]' "$note"
}

step_php() {
    if ! command -v composer >/dev/null 2>&1; then
        STEP_DURATION=0
        emit_step php skipped '[]' '[]' "composer not installed"
        return
    fi
    run_step php composer global update --no-interaction
    local status="failed" note="composer exited ${STEP_CODE}"
    if (( STEP_CODE == 0 )); then status="success"; note=""; fi
    emit_step php "$status" '[]' '[]' "$note"
    # audit (separate block, best-effort)
    if composer global audit --format=json > "$RUN_TMP/composer-audit.json" 2>/dev/null; then
        COMPOSER_AUDIT_JSON="$(cat "$RUN_TMP/composer-audit.json")"
    else
        COMPOSER_AUDIT_JSON='{"error":"composer global audit unavailable or failed"}'
    fi
}

step_node() {
    if ! command -v fnm >/dev/null 2>&1; then
        STEP_DURATION=0
        emit_step node-fn skipped '[]' '[]' "fnm not installed"
        return
    fi
    eval "$(fnm env 2>/dev/null)"
    local npm_globals old_default
    npm_globals="$(npm ls -g --depth=0 --json 2>/dev/null | jq -r '.dependencies | keys[]' 2>/dev/null | tr '\n' ' ')"
    old_default="$(fnm current 2>/dev/null || true)"
    local logf="$LOG_DIR/$RUN_ID.node-fn.log"
    local start end code
    start="$(date +%s)"
    {
        printf '# fnm install --lts\n'
        fnm install --lts
        printf '# fnm default lts-latest\n'
        fnm default lts-latest
        eval "$(fnm env 2>/dev/null)"
        fnm use lts-latest
        printf '# npm install -g %s\n' "$npm_globals"
        if [[ -n "$npm_globals" ]]; then
            npm install -g $npm_globals
        fi
    } > "$logf" 2>&1
    code=$?
    end="$(date +%s)"
    STEP_DURATION=$((end - start))
    local updated=() pruned=()
    if (( code == 0 )); then
        local new_default
        new_default="$(fnm current 2>/dev/null || true)"
        if [[ -n "$new_default" && "$new_default" != "$old_default" && "$new_default" != "default" && "$new_default" != "system" ]]; then
            updated+=("{\"name\":\"node\",\"from\":\"${old_default:-n/a}\",\"to\":\"$new_default\"}")
        fi
        for v in $(fnm list 2>/dev/null | grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+' | sort -u); do
            if [[ -n "$new_default" && "$v" != "$new_default" && "$v" != "$old_default" ]]; then
                if fnm uninstall "$v" >> "$logf" 2>&1; then
                    pruned+=("{\"name\":\"node@${v}\",\"to\":\"removed\"}")
                fi
            fi
        done
        updated+=("${pruned[@]}")
        local updated_json="[]"
        (( ${#updated[@]} > 0 )) && updated_json="[$(IFS=,; echo "${updated[*]}")]"
        emit_step node-fn success "$updated_json" '[]' ""
    else
        emit_step node-fn failed '[]' '[]' "fnm/npm exited ${code}"
    fi
}

step_python() {
    if ! command -v uv >/dev/null 2>&1; then
        STEP_DURATION=0
        emit_step python-uv skipped '[]' '[]' "uv not installed"
        return
    fi
    run_step python-uv bash -c 'uv self update || true; uv python install; uv pip install --system --break-system-packages -U --python "$(uv python find)" pynvim neovim'
    if (( STEP_CODE == 0 )); then
        emit_step python-uv success '[]' '[]' "uv self-update failure ignored (brew-managed)"
    else
        emit_step python-uv failed '[]' '[]' "uv exited ${STEP_CODE}"
    fi
}

step_opencode() {
    if ! command -v opencode >/dev/null 2>&1; then
        STEP_DURATION=0
        emit_step opencode skipped '[]' '[]' "opencode not installed"
        return
    fi
    run_step opencode opencode upgrade
    local updated="[]"
    if (( STEP_CODE == 0 )) && grep -qiE 'up to date|already' "$STEP_LOG"; then
        updated='[]'
    elif (( STEP_CODE == 0 )); then
        updated="$(jq -n '[{name:"opencode"}]')"
    fi
    if (( STEP_CODE == 0 )); then
        emit_step opencode success "$updated" '[]' ""
    else
        emit_step opencode failed '[]' '[]' "opencode upgrade exited ${STEP_CODE}"
    fi
}

step_nvim_plugins() {
    if ! command -v nvim >/dev/null 2>&1; then
        STEP_DURATION=0
        emit_step neovim-plugins skipped '[]' '[]' "nvim not installed"
        return
    fi
    run_step neovim-plugins nvim --headless +PlugUpdate +qa
    if (( STEP_CODE == 0 )); then
        emit_step neovim-plugins success '[]' '[]' ""
    else
        emit_step neovim-plugins failed '[]' '[]' "nvim exited ${STEP_CODE}"
    fi
}

step_gem() {
    if ! command -v gem >/dev/null 2>&1; then
        STEP_DURATION=0
        emit_step gem skipped '[]' '[]' "gem not installed"
        return
    fi
    run_step gem gem update neovim --no-document
    if (( STEP_CODE == 0 )); then
        emit_step gem success '[]' '[]' ""
    else
        emit_step gem failed '[]' '[]' "gem exited ${STEP_CODE}"
    fi
}

step_tpm() {
    if [[ ! -x /opt/homebrew/opt/tpm/share/tpm/bin/update_plugins || ! -d "$HOME/.tmux/plugins" ]]; then
        STEP_DURATION=0
        emit_step tmux-tpm skipped '[]' '[]' "TPM or ~/.tmux/plugins not present"
        return
    fi
    run_step tmux-tpm /opt/homebrew/opt/tpm/share/tpm/bin/update_plugins all
    if (( STEP_CODE == 0 )); then
        emit_step tmux-tpm success '[]' '[]' ""
    else
        emit_step tmux-tpm failed '[]' '[]' "update_plugins exited ${STEP_CODE}"
    fi
}

step_macos() {
    local sw_out sw_cnt
    sw_out="$(softwareupdate --list 2>/dev/null || true)"
    sw_cnt="$(printf '%s\n' "$sw_out" | grep -c '^\* Label:' || true)"
    local note="no macOS updates pending"
    if (( sw_cnt > 0 )); then
        note="${sw_cnt} macOS updates pending (install manually via System Settings)"
        notify dotfiles-updater "macOS updates available — open System Settings to install" "$note"
    fi
    STEP_DURATION=0
    emit_step macos success '[]' '[]' "$note"
}

step_audit_brew() {
    # Deprecated formulae audit (separate block, best-effort, slow-ish)
    local formulae
    formulae="$(brew list --formula 2>/dev/null | tr '\n' ' ')"
    if [[ -n "$formulae" ]]; then
        BREW_DEPRECATED_JSON="$(brew info --json=v2 $formulae 2>/dev/null | jq -c '[.formulae[] | select(.deprecated == true) | .name]' 2>/dev/null || echo '[]')"
    else
        BREW_DEPRECATED_JSON='[]'
    fi
}

# ------------------------------------------------------------------------------
# Pipeline orchestration
# ------------------------------------------------------------------------------

run_pipeline() {
    local trigger="$1"
    RUN_TMP="$(mktemp -d "$LOG_DIR/run.XXXXXX")"
    BREW_DEPRECATED_JSON='[]'
    COMPOSER_AUDIT_JSON='{"error":"composer audit not run"}'
    : > "$LOG_DIR/$RUN_ID.combined.log"
    rm -f "$LOG_DIR/$RUN_ID.done"
    local progress_pid=""
    if [[ "$trigger" == "foreground" ]]; then
        osascript -l JavaScript "$(dirname "$0")/dotfiles-updater-progress.jxa" "$LOG_DIR" "$RUN_ID" &
        progress_pid=$!
    fi

    notify dotfiles-updater "System update started in the background" "tap to see progress in ~/dotfiles/logs/dotfiles-updater"

    # caffeinate: keep the machine awake for the duration of this PID
    caffeinate -ims -w "$$" &
    local caff_pid=$!

    local rtk_before rtk_after
    rtk_before="$(command -v rtk >/dev/null 2>&1 && rtk --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || echo '')"

    step_brew
    notify dotfiles-updater "brew finished" "step 1 of 12"

    rtk_after="$(command -v rtk >/dev/null 2>&1 && rtk --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || echo '')"
    if [[ -n "$rtk_before" && -n "$rtk_after" && "$rtk_before" != "$rtk_after" ]]; then
        echo changed > "$RUN_TMP/rtk-changed"
    fi
    step_rtk
    notify dotfiles-updater "rtk checked" "step 2 of 12"

    step_mas
    notify dotfiles-updater "App Store finished" "step 3 of 12"

    step_rust
    notify dotfiles-updater "rust finished" "step 4 of 12"

    step_php
    notify dotfiles-updater "composer finished" "step 5 of 12"

    step_node
    notify dotfiles-updater "node (fnm) finished" "step 6 of 12"

    step_python
    notify dotfiles-updater "python (uv) finished" "step 7 of 12"

    step_opencode
    notify dotfiles-updater "opencode finished" "step 8 of 12"

    step_nvim_plugins
    notify dotfiles-updater "neovim plugins finished" "step 9 of 12"

    step_gem
    notify dotfiles-updater "gem finished" "step 10 of 12"

    step_tpm
    notify dotfiles-updater "tmux finished" "step 11 of 12"

    step_macos
    notify dotfiles-updater "macOS check finished" "step 12 of 12"

    step_audit_brew

    : > "$LOG_DIR/$RUN_ID.done"
    # progress window (if any) will notice .done and stay open until user closes it (or 60s)
    kill "$caff_pid" 2>/dev/null || true

    local steps_json failed_names status overall_status
    steps_json="$(jq -s '.' "$RUN_TMP"/steps/*.json 2>/dev/null || echo '[]')"
    failed_names="$(echo "$steps_json" | jq -c '[.[] | select(.status == "failed") | .name]')"
    status="success"
    if (( $(echo "$steps_json" | jq '[.[] | select(.status == "failed")] | length') > 0 )); then
        status="partial"
    fi
    overall_status="$status"

    if [[ "$status" == "success" ]]; then
        state_set_num last_success_at "$(date +%s)"
        state_set_arr last_failed_steps '[]'
        state_set_str last_outcome success
    else
        state_set_arr last_failed_steps "$failed_names"
        state_set_str last_outcome partial
    fi

    # final report
    local updated_count failed_count
    updated_count="$(echo "$steps_json" | jq '[.[].updated | length] | add')"
    failed_count="$(echo "$steps_json" | jq '[.[] | select(.status == "failed")] | length')"
    jq -n \
        --arg schema "dotfiles-updater@1" \
        --arg run_id "$RUN_ID" \
        --arg trigger "$trigger" \
        --arg started_at "$RUN_STARTED_ISO" \
        --arg finished_at "$(iso_now)" \
        --argjson duration_seconds "$(( $(date +%s) - RUN_STARTED_EPOCH ))" \
        --arg status "$overall_status" \
        --argjson on_ac "$ON_AC" \
        --argjson battery_pct "${BATTERY_PCT:-100}" \
        --argjson free_disk_gb "${FREE_DISK_GB:-0}" \
        --argjson steps "$steps_json" \
        --argjson brew_deprecated "$BREW_DEPRECATED_JSON" \
        --argjson composer_audit "$COMPOSER_AUDIT_JSON" \
        '{schema:$schema,run_id:$run_id,trigger:$trigger,started_at:$started_at,finished_at:$finished_at,duration_seconds:$duration_seconds,status:$status,environment:{on_ac_power:($on_ac==1),battery_pct:$battery_pct,free_disk_gb:$free_disk_gb},steps:$steps,audit:{brew_deprecated:$brew_deprecated,composer:$composer_audit}}' \
        > "$REPORT_PATH"

    rm -rf "$RUN_TMP"

    notify dotfiles-updater \
        "Update finished (${status}) — ${updated_count} updated, ${failed_count} failed, took $(( ( $(date +%s) - RUN_STARTED_EPOCH ) / 60 ))min" \
        "report: $REPORT_PATH"
}

# ------------------------------------------------------------------------------
# Dry-run
# ------------------------------------------------------------------------------

dry_run() {
    battery_info
    FREE_DISK_GB="$(df -g / | awk 'NR==2 {print $4}')"
    log "dry-run: no state, lock or filesystem mutation"

    echo ""
    echo "Gates:"
    echo "  power:      $(gate_power; echo "$GATE_POWER_REASON")"
    echo "  network:    $(gate_network; echo "$GATE_NETWORK_REASON")"
    echo "  disk:       $(gate_disk; echo "$GATE_DISK_REASON")"
    echo "  pkgmgr:     $(gate_pkgmgr; echo "$GATE_PKGMGR_REASON")"
    echo "  schedule:   $(gate_schedule; echo "$GATE_SCHEDULE_REASON")"
    echo "  dialog cap: $(gate_dialog_cooldown; echo "$GATE_DIALOG_REASON")"
    echo ""

    local summary
    summary="$(probe_summary)"
    echo "Dialog preview:"
    echo "--------------------------------------------------------------------------------"
    printf '%s\n' "$summary"
    echo "--------------------------------------------------------------------------------"
    echo ""

    echo "Report skeleton:"
    jq -n \
        --arg schema "dotfiles-updater@1" \
        --arg run_id "$RUN_ID" \
        --arg trigger "dry_run" \
        --arg started_at "$RUN_STARTED_ISO" \
        --argjson on_ac "$ON_AC" \
        --argjson battery_pct "${BATTERY_PCT:-100}" \
        --argjson free_disk_gb "${FREE_DISK_GB:-0}" \
        '{schema:$schema,run_id:$run_id,trigger:$trigger,started_at:$started_at,status:"pending",environment:{on_ac_power:($on_ac==1),battery_pct:$battery_pct,free_disk_gb:$free_disk_gb},steps:[],audit:{brew_deprecated:[],composer:null}}'
}

# ------------------------------------------------------------------------------
# Mode dispatch
# ------------------------------------------------------------------------------

usage() {
    cat <<EOF
Usage: dotfiles-updater.sh [--gate|--foreground|--dry-run|--help]

  --gate        LaunchAgent tick: silent pre-flight checks; shows the dialog
                at most once per 24h; every update requires an explicit click.
  --foreground  Manual run in a terminal (default): gates print reasons and
                ask before proceeding; then shows the dialog.
  --dry-run     Show gate results, dialog content and report skeleton only.
                Zero mutation of system or state.

State:   $STATE_FILE
Logs:    $LOG_DIR
EOF
}

main() {
    local mode="${1:---foreground}"

    if [[ "$mode" != "--dry-run" ]]; then
        state_init
        find "$LOG_DIR" -name '*.json' -mtime "+${RETENTION_DAYS}" -delete 2>/dev/null || true
    fi

    case "$mode" in
        --dry-run)
            DRY_MODE=1
            dry_run
            exit 0
            ;;
        --gate)
            if ! gate_schedule; then
                log "$GATE_SCHEDULE_REASON"
                exit 0
            fi
            if ! lock_acquire; then
                log "another run in progress, exiting"
                exit 0
            fi
            if ! gate_env; then
                exit 0
            fi
            if ! gate_dialog_cooldown; then
                log "$GATE_DIALOG_REASON"
                exit 0
            fi
            local summary
            summary="$(probe_summary)"
            if [[ -z "$summary" ]]; then
                summary="Nothing looks outdated; this will refresh indexes and caches."
            fi
            summary+=$'\n\n'
            summary+="This runs in the background (you will be notified). Proceed?"
            if ! show_dialog "$summary"; then
                state_set_str last_outcome postponed
                log "user postponed"
                exit 0
            fi
            run_pipeline gate
            ;;
        --foreground)
            if ! lock_acquire; then
                echo "error: another run is in progress (lock: $LOCK_DIR)" >&2
                exit 1
            fi
            local failed_gates=0
            for f in gate_power gate_network gate_disk gate_pkgmgr; do
                if ! "$f"; then
                    failed_gates=1
                    echo "gate $f: $(print_gate_reason "$f")"
                fi
            done
            if (( failed_gates == 1 )); then
                read -r -p "Some pre-flight gates failed. Proceed anyway? [y/N] " answer
                if [[ "$answer" != "y" && "$answer" != "Y" ]]; then
                    echo "aborted"
                    exit 0
                fi
            fi
            local summary
            echo "Checking for updates (this can take a few seconds)…"
            summary="$(probe_summary)"
            if [[ -z "$summary" ]]; then
                summary="Nothing looks outdated; this will refresh indexes and caches."
            fi
            echo ""
            echo "What will be updated:"
            echo "--------------------------------------------------------------------------------"
            printf '%s\n' "$summary"
            echo "--------------------------------------------------------------------------------"
            summary+=$'\n\n'
            summary+="This runs in the background (you will be notified). Proceed?"
            if ! show_dialog "$summary"; then
                state_set_str last_outcome postponed
                echo "postponed — will ask again tomorrow"
                exit 0
            fi
            run_pipeline foreground
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            usage
            exit 1
            ;;
    esac
}

main "$@"
