#!/usr/bin/env bash
# Mutes / restores macOS's own "plugged in" chime, so the NOTCH-ZAP-INATOR is
# the only thing you hear when the charger goes in.
#
#   scripts/chime.sh mute      remember the current setting, then mute the chime
#   scripts/chime.sh restore   put back whatever was there before `mute`
#   scripts/chime.sh status    show the current setting
#
# The chime is played by PowerChime.app (launchd job com.apple.powerchime),
# which checks the `ChimeOnNoHardware` default each time it starts. launchd
# starts it on demand on power-source changes, so restarting it is enough.
set -euo pipefail

DOMAIN=com.apple.PowerChime
KEY=ChimeOnNoHardware
STATE_DIR="$HOME/Library/Application Support/NotchZapInator"
SAVED="$STATE_DIR/chime-before"

restart_chime() {
    launchctl kickstart -k "gui/$(id -u)/com.apple.powerchime" >/dev/null 2>&1 \
        || killall PowerChime >/dev/null 2>&1 \
        || true
}

current() {
    defaults read "$DOMAIN" "$KEY" 2>/dev/null || echo unset
}

case "${1:-}" in
    mute)
        mkdir -p "$STATE_DIR"
        # Only remember the *original* setting, so reinstalling doesn't
        # overwrite it with our own "muted".
        [[ -f "$SAVED" ]] || current > "$SAVED"
        defaults write "$DOMAIN" "$KEY" -bool true
        restart_chime
        echo "macOS charging chime: muted"
        ;;
    restore)
        if [[ -f "$SAVED" ]]; then
            case "$(cat "$SAVED")" in
                unset) defaults delete "$DOMAIN" "$KEY" >/dev/null 2>&1 || true ;;
                1) defaults write "$DOMAIN" "$KEY" -bool true ;;
                *) defaults write "$DOMAIN" "$KEY" -bool false ;;
            esac
            rm -f "$SAVED"
            rmdir "$STATE_DIR" 2>/dev/null || true
            restart_chime
            echo "macOS charging chime: restored"
        fi
        ;;
    status)
        case "$(current)" in
            1) echo "macOS charging chime: muted" ;;
            *) echo "macOS charging chime: on" ;;
        esac
        ;;
    *)
        echo "usage: $0 mute|restore|status" >&2
        exit 2
        ;;
esac
