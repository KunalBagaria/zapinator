#!/usr/bin/env bash
# Starts / stops the NOTCH-ZAP-INATOR LaunchAgent for the app bundle this
# script lives in (NotchZapInator.app/Contents/Resources/agent.sh). Shared by
# the curl installer, the Homebrew cask and `make install`.
#
#   agent.sh install [--into DIR] [--keep-chime]
#       write the LaunchAgent, (re)start the app, mute macOS's own charging
#       chime. With --into, first copy this app bundle into DIR and set up
#       that copy instead.
#   agent.sh uninstall
#       stop the app, remove the LaunchAgent, restore the chime
set -euo pipefail

LABEL=com.kunalbagaria.zapinator
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP="$(cd "$HERE/../.." && pwd)"
EXE="$APP/Contents/MacOS/notch-zap-inator"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
LOG="$HOME/Library/Logs/NotchZapInator.log"
DOMAIN="gui/$(id -u)"

stop_agent() {
    launchctl bootout "$DOMAIN/$LABEL" >/dev/null 2>&1 || true
}

# The app is ad-hoc signed, not notarized, so macOS kills anything started
# from a quarantined copy (Homebrew quarantines its downloads; curl doesn't).
# That's also why the installers run this script with /bin/bash rather than
# executing it directly.
unquarantine() {
    xattr -dr com.apple.quarantine "$1" 2>/dev/null || true
}

# The executable the installed LaunchAgent points at, if there is one.
agent_exe() {
    [[ -f "$PLIST" ]] && plutil -extract ProgramArguments.0 raw -o - "$PLIST" 2>/dev/null || true
}

install() {
    local keep_chime=0 into="" target
    while (( $# )); do
        case $1 in
            --keep-chime) keep_chime=1 ;;
            --into) into="${2:?agent.sh: --into needs a folder}"; shift ;;
            *) echo "agent.sh: unknown option: $1" >&2; exit 2 ;;
        esac
        shift
    done

    if [[ -n "$into" ]]; then
        mkdir -p "$into"
        target="$(cd "$into" && pwd)/NotchZapInator.app"
        if [[ "$target" != "$APP" ]]; then
            stop_agent # the running copy may be the one being replaced
            rm -rf "$target"
            ditto "$APP" "$target"
            unquarantine "$target"
            echo "installed: $target"
            if (( keep_chime )); then
                exec /bin/bash "$target/Contents/Resources/agent.sh" install --keep-chime
            fi
            exec /bin/bash "$target/Contents/Resources/agent.sh" install
        fi
    fi

    [[ -x "$EXE" ]] || { echo "agent.sh: no executable at $EXE" >&2; exit 1; }
    unquarantine "$APP"

    mkdir -p "$HOME/Library/LaunchAgents" "$HOME/Library/Logs"
    stop_agent
    sed -e "s|__LABEL__|$LABEL|g" -e "s|__EXE__|$EXE|g" -e "s|__LOG__|$LOG|g" \
        "$HERE/launchagent.plist" > "$PLIST"
    plutil -lint "$PLIST" >/dev/null

    # launchd can take a moment to forget a job it just booted out.
    local tries=0
    until launchctl bootstrap "$DOMAIN" "$PLIST" 2>/dev/null; do
        if (( ++tries >= 5 )); then
            launchctl bootstrap "$DOMAIN" "$PLIST" # once more, showing the error
            exit 1
        fi
        sleep 1
    done
    echo "agent: running $APP (starts at login; log: $LOG)"

    if (( keep_chime )); then
        echo "macOS charging chime: left as it is"
    else
        /bin/bash "$HERE/chime.sh" mute
    fi
}

uninstall() {
    # Another copy (say, Homebrew's and the curl installer's) may own the agent
    # now. Leave that one, and its chime setting, alone.
    local owner
    owner=$(agent_exe)
    if [[ -n "$owner" && "$owner" != "$EXE" && -x "$owner" ]]; then
        echo "agent: owned by $owner, leaving it running"
        return
    fi
    stop_agent
    rm -f "$PLIST"
    echo "agent: stopped and removed"
    /bin/bash "$HERE/chime.sh" restore
}

case "${1:-}" in
    install) shift; install "$@" ;;
    uninstall) uninstall ;;
    *) echo "usage: $0 install [--into DIR] [--keep-chime] | uninstall" >&2; exit 2 ;;
esac
