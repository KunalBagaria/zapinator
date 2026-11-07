#!/bin/bash
# NOTCH-ZAP-INATOR installer, served at https://zapinator.kunalbagaria.com
#
#   curl -fsSL https://zapinator.kunalbagaria.com | bash                       install or update
#   curl -fsSL https://zapinator.kunalbagaria.com | bash -s -- --keep-chime    same, but leave macOS's charging chime on
#   curl -fsSL https://zapinator.kunalbagaria.com | bash -s -- --uninstall     remove it again
#
# Installs ~/Applications/NotchZapInator.app and a LaunchAgent that starts it
# at login. No sudo. Needs an Apple Silicon MacBook with a notch, macOS 13+.
set -euo pipefail

BASE_URL="${ZAPINATOR_URL:-https://zapinator.kunalbagaria.com}"
APP="$HOME/Applications/NotchZapInator.app"
LABEL=com.kunalbagaria.zapinator
TMP_DIR=""
trap '[[ -z "$TMP_DIR" ]] || rm -rf "$TMP_DIR"' EXIT

if [[ -t 1 ]]; then BOLT=$'\033[1;36m⚡\033[0m' ERR=$'\033[1;31merror:\033[0m'; else BOLT='⚡' ERR='error:'; fi
say() { printf '%s %s\n' "$BOLT" "$*"; }
die() { printf '%s %s\n' "$ERR" "$*" >&2; exit 1; }

check_mac() {
    [[ "$(uname -s)" == Darwin ]] || die "NOTCH-ZAP-INATOR only runs on macOS."
    [[ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" == 1 ]] ||
        die "NOTCH-ZAP-INATOR needs an Apple Silicon Mac (every MacBook with a notch is one)."
    local major
    major="$(sw_vers -productVersion | cut -d. -f1)"
    (( major >= 13 )) || die "NOTCH-ZAP-INATOR needs macOS 13 or later (this is $(sw_vers -productVersion))."
}

install() {
    local keep_chime="$1" tmp latest version sha geometry
    check_mac

    TMP_DIR="$(mktemp -d)"
    tmp="$TMP_DIR"

    latest="$(curl -fsSL "$BASE_URL/latest")" || die "couldn't reach $BASE_URL"
    version="${latest%% *}"
    sha="${latest##* }"
    [[ "$version" =~ ^[0-9][0-9A-Za-z.+-]*$ && "$sha" =~ ^[0-9a-f]{64}$ ]] ||
        die "unexpected reply from $BASE_URL/latest"

    say "Downloading NOTCH-ZAP-INATOR $version"
    curl -fL --progress-bar -o "$tmp/app.tar.gz" "$BASE_URL/releases/NotchZapInator-$version.tar.gz"
    echo "$sha  $tmp/app.tar.gz" | shasum -a 256 -c - >/dev/null 2>&1 ||
        die "the download's checksum doesn't match; try again"
    tar -xzf "$tmp/app.tar.gz" -C "$tmp"
    [[ -d "$tmp/NotchZapInator.app" ]] || die "the download doesn't contain NotchZapInator.app"
    codesign --verify --deep --strict "$tmp/NotchZapInator.app" 2>/dev/null ||
        die "the app's code signature doesn't verify"

    say "Installing to $APP"
    local setup="$tmp/NotchZapInator.app/Contents/Resources/agent.sh"
    if [[ "$keep_chime" == 1 ]]; then
        /bin/bash "$setup" install --into "$HOME/Applications" --keep-chime
    else
        /bin/bash "$setup" install --into "$HOME/Applications"
    fi

    if [[ -d /Applications/NotchZapInator.app ]]; then
        say "Note: there's another copy in /Applications (Homebrew?). This one now runs the show."
    fi
    geometry="$("$APP/Contents/MacOS/notch-zap-inator" --print-geometry 2>/dev/null || true)"
    if [[ "$geometry" != *notched* ]]; then
        say "No notched display right now (lid closed?). It stays quiet until there is one."
    fi

    echo
    say "NOTCH-ZAP-INATOR $version is armed. Plug in your charger."
    echo "   Preview: $APP/Contents/MacOS/notch-zap-inator --demo"
    echo "   Remove:  curl -fsSL $BASE_URL | bash -s -- --uninstall"
}

uninstall() {
    if [[ -f "$APP/Contents/Resources/agent.sh" ]]; then
        /bin/bash "$APP/Contents/Resources/agent.sh" uninstall
    else
        launchctl bootout "gui/$(id -u)/$LABEL" >/dev/null 2>&1 || true
        rm -f "$HOME/Library/LaunchAgents/$LABEL.plist"
    fi
    rm -rf "$APP"
    say "NOTCH-ZAP-INATOR removed. The notch is just a notch again."
}

usage() {
    cat <<EOF
NOTCH-ZAP-INATOR installer

  curl -fsSL $BASE_URL | bash                       install or update
  curl -fsSL $BASE_URL | bash -s -- --keep-chime    same, but leave macOS's charging chime on
  curl -fsSL $BASE_URL | bash -s -- --uninstall     remove it again
EOF
}

# Everything runs from here, so a half-downloaded script does nothing.
main() {
    local action=install keep_chime=0
    for arg in "$@"; do
        case $arg in
            --uninstall) action=uninstall ;;
            --keep-chime) keep_chime=1 ;;
            -h | --help) usage; return ;;
            *) die "unknown option: $arg (try --keep-chime or --uninstall)" ;;
        esac
    done
    if [[ "$action" == uninstall ]]; then
        uninstall
    else
        install "$keep_chime"
    fi
}

main "$@"
