#!/bin/sh
set -eu

usage() {
    printf '%s\n' "usage: $0 NEW_BINARY NEW_LAUNCHER_APP [--native-compatibility]" >&2
    exit 2
}

fail() {
    printf '%s\n' "error: $1" >&2
    exit 1
}

wait_for_service_state() {
    bridge_binary=$1
    expected_state=$2
    attempts=0
    while [ "$attempts" -lt 100 ]; do
        if observed_state=$("$bridge_binary" service status 2>/dev/null); then
            if [ "$observed_state" = "$expected_state" ]; then
                return 0
            fi
            case "$observed_state" in
                "service loaded"|"service not_loaded") ;;
                *) return 1 ;;
            esac
        else
            return 1
        fi
        attempts=$((attempts + 1))
        [ "$attempts" -ge 100 ] || /bin/sleep 0.1
    done
    return 1
}

native_compatibility=0
if [ "$#" -eq 3 ] && [ "$3" = "--native-compatibility" ]; then
    native_compatibility=1
elif [ "$#" -ne 2 ]; then
    usage
fi

if [ "$(uname -s)" != "Darwin" ] || [ "$(uname -m)" != "arm64" ]; then
    fail "bridge replacement currently supports macOS arm64 only"
fi

new_parent=$(CDPATH= cd -- "$(dirname -- "$1")" && pwd -P)
new_binary="$new_parent/$(basename -- "$1")"
if [ ! -f "$new_binary" ] || [ -L "$new_binary" ] || [ ! -x "$new_binary" ]; then
    fail "new bridge must be a regular executable file"
fi
case "$(/usr/bin/file -b "$new_binary")" in
    *Mach-O*arm64*) ;;
    *) fail "new bridge must be a macOS arm64 Mach-O executable" ;;
esac

new_launcher_parent=$(CDPATH= cd -- "$(dirname -- "$2")" && pwd -P)
new_launcher="$new_launcher_parent/$(basename -- "$2")"
new_launcher_executable="$new_launcher/Contents/MacOS/Grok Codex Switch"
new_overlay="$new_launcher/Contents/Resources/grok-codex-bridge-overlay.md"
if [ ! -d "$new_launcher" ] || [ -L "$new_launcher" ] \
    || [ ! -f "$new_launcher_executable" ] || [ -L "$new_launcher_executable" ] \
    || [ ! -x "$new_launcher_executable" ] \
    || [ ! -f "$new_overlay" ] || [ -L "$new_overlay" ]; then
    fail "new launcher must be a complete regular app bundle"
fi
install_root="${HOME:?HOME is required}/Library/Application Support/grok-codex-bridge"
installed_binary="$install_root/bin/grok-codex-bridge"
installed_launcher="$install_root/bin/Grok Codex Switch.app"
if [ ! -f "$installed_binary" ] || [ -L "$installed_binary" ] || [ ! -x "$installed_binary" ]; then
    fail "installed bridge is missing or unsafe: $installed_binary"
fi
if [ "$new_binary" = "$installed_binary" ]; then
    fail "new bridge must not be the installed bridge path"
fi
if [ ! -d "$installed_launcher" ] || [ -L "$installed_launcher" ]; then
    fail "installed launcher is missing or unsafe: $installed_launcher"
fi

new_version=$("$new_binary" version)
if [ "$native_compatibility" -eq 0 ]; then
    "$new_binary" auth ensure
fi
service_state=$("$installed_binary" service status)
if [ "$service_state" != "service loaded" ]; then
    fail "installed bridge service is not loaded: $service_state"
fi

binary_dir="$install_root/bin"
staged_binary="$binary_dir/.grok-codex-bridge.new.$$"
rollback_binary="$binary_dir/.grok-codex-bridge.rollback.$$"
staged_launcher="$binary_dir/.Grok Codex Switch.app.new.$$"
rollback_launcher="$binary_dir/.Grok Codex Switch.app.rollback.$$"
stopped=0
replaced=0
launcher_replaced=0

cleanup() {
    exit_code=$?
    trap - 0 1 2 15

    if [ "$exit_code" -ne 0 ]; then
        if { [ "$replaced" -eq 1 ] && [ -f "$rollback_binary" ]; } \
            || { [ "$launcher_replaced" -eq 1 ] && [ -d "$rollback_launcher" ]; }; then
            if "$installed_binary" service uninstall >/dev/null 2>&1 \
                && wait_for_service_state "$installed_binary" "service not_loaded"; then
                if [ "$replaced" -eq 1 ] && [ -f "$rollback_binary" ]; then
                    /bin/mv -f "$rollback_binary" "$installed_binary"
                    /bin/chmod 755 "$installed_binary"
                fi
                if [ "$launcher_replaced" -eq 1 ] && [ -d "$rollback_launcher" ]; then
                    [ ! -e "$installed_launcher" ] || /bin/rm -rf "$installed_launcher"
                    /bin/mv "$rollback_launcher" "$installed_launcher"
                fi
                if "$installed_binary" service install >/dev/null 2>&1 \
                    && wait_for_service_state "$installed_binary" "service loaded"; then
                    printf '%s\n' "rollback: restored the previous bridge runtime and restarted its service" >&2
                else
                    printf '%s\n' "rollback error: previous runtime was restored but its service did not restart" >&2
                fi
            else
                printf '%s\n' "rollback error: unable to restore the previous bridge" >&2
            fi
        elif [ "$stopped" -eq 1 ]; then
            if "$installed_binary" service install >/dev/null 2>&1 \
                && wait_for_service_state "$installed_binary" "service loaded"; then
                printf '%s\n' "rollback: restarted the previous bridge service" >&2
            else
                printf '%s\n' "rollback error: previous bridge service did not restart" >&2
            fi
        fi
    fi

    [ ! -e "$staged_binary" ] || /bin/rm -f "$staged_binary"
    [ ! -e "$staged_launcher" ] || /bin/rm -rf "$staged_launcher"
    if [ "$exit_code" -eq 0 ]; then
        [ ! -e "$rollback_binary" ] || /bin/rm -f "$rollback_binary"
        [ ! -e "$rollback_launcher" ] || /bin/rm -rf "$rollback_launcher"
    fi
    exit "$exit_code"
}

trap cleanup 0
trap 'exit 129' 1
trap 'exit 130' 2
trap 'exit 143' 15

/usr/bin/install -m 755 "$new_binary" "$staged_binary"
/usr/bin/install -m 755 "$installed_binary" "$rollback_binary"
/usr/bin/ditto "$new_launcher" "$staged_launcher"
/usr/bin/cmp -s "$new_binary" "$staged_binary" || fail "staged bridge differs from the new binary"
/usr/bin/xattr -cr "$staged_launcher"
if ! /usr/bin/codesign --verify --deep --strict "$staged_launcher" 2>/dev/null; then
    fail "staged launcher signature is invalid"
fi

"$installed_binary" service uninstall
stopped=1
if ! wait_for_service_state "$installed_binary" "service not_loaded"; then
    fail "bridge service did not reach not_loaded state before the bounded deadline"
fi

/bin/mv -f "$staged_binary" "$installed_binary"
replaced=1
/bin/chmod 755 "$installed_binary"
/bin/mv "$installed_launcher" "$rollback_launcher"
launcher_replaced=1
/bin/mv "$staged_launcher" "$installed_launcher"
/usr/bin/cmp -s "$new_binary" "$installed_binary" || fail "installed bridge differs from the new binary"
if ! /usr/bin/codesign --verify --deep --strict "$installed_launcher" 2>/dev/null; then
    fail "installed launcher signature is invalid after replacement"
fi
installed_version=$("$installed_binary" version)
if [ "$installed_version" != "$new_version" ]; then
    fail "installed bridge version does not match the new binary"
fi

"$installed_binary" service install
if ! wait_for_service_state "$installed_binary" "service loaded"; then
    fail "new bridge service did not reach loaded state before the bounded deadline"
fi
service_state="service loaded"
if [ "$native_compatibility" -eq 1 ]; then
    "$installed_binary" doctor --native-compatibility
else
    "$installed_binary" doctor
fi

printf '%s\n' "bridge replaced: $installed_binary"
printf '%s\n' "launcher replaced: $installed_launcher"
printf '%s\n' "version: $installed_version"
printf '%s\n' "$service_state"
