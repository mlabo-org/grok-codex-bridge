#!/bin/sh
set -eu

usage() {
    printf '%s\n' "usage: $0 NEW_BINARY" >&2
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

if [ "$#" -ne 1 ]; then
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

install_root="${HOME:?HOME is required}/Library/Application Support/grok-codex-bridge"
installed_binary="$install_root/bin/grok-codex-bridge"
if [ ! -f "$installed_binary" ] || [ -L "$installed_binary" ] || [ ! -x "$installed_binary" ]; then
    fail "installed bridge is missing or unsafe: $installed_binary"
fi
if [ "$new_binary" = "$installed_binary" ]; then
    fail "new bridge must not be the installed bridge path"
fi

new_version=$("$new_binary" version)
"$new_binary" auth ensure
service_state=$("$installed_binary" service status)
if [ "$service_state" != "service loaded" ]; then
    fail "installed bridge service is not loaded: $service_state"
fi

binary_dir="$install_root/bin"
staged_binary="$binary_dir/.grok-codex-bridge.new.$$"
rollback_binary="$binary_dir/.grok-codex-bridge.rollback.$$"
stopped=0
replaced=0

cleanup() {
    exit_code=$?
    trap - 0 1 2 15

    if [ "$exit_code" -ne 0 ]; then
        if [ "$replaced" -eq 1 ] && [ -f "$rollback_binary" ]; then
            if "$installed_binary" service uninstall >/dev/null 2>&1 \
                && wait_for_service_state "$installed_binary" "service not_loaded" \
                && /bin/mv -f "$rollback_binary" "$installed_binary"; then
                /bin/chmod 755 "$installed_binary"
                if "$installed_binary" service install >/dev/null 2>&1 \
                    && wait_for_service_state "$installed_binary" "service loaded"; then
                    printf '%s\n' "rollback: restored the previous bridge and restarted its service" >&2
                else
                    printf '%s\n' "rollback error: previous bridge was restored but its service did not restart" >&2
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
    if [ "$exit_code" -eq 0 ]; then
        [ ! -e "$rollback_binary" ] || /bin/rm -f "$rollback_binary"
    fi
    exit "$exit_code"
}

trap cleanup 0
trap 'exit 129' 1
trap 'exit 130' 2
trap 'exit 143' 15

/usr/bin/install -m 755 "$new_binary" "$staged_binary"
/usr/bin/install -m 755 "$installed_binary" "$rollback_binary"
/usr/bin/cmp -s "$new_binary" "$staged_binary" || fail "staged bridge differs from the new binary"

"$installed_binary" service uninstall
stopped=1
if ! wait_for_service_state "$installed_binary" "service not_loaded"; then
    fail "bridge service did not reach not_loaded state before the bounded deadline"
fi

/bin/mv -f "$staged_binary" "$installed_binary"
replaced=1
/bin/chmod 755 "$installed_binary"
/usr/bin/cmp -s "$new_binary" "$installed_binary" || fail "installed bridge differs from the new binary"
installed_version=$("$installed_binary" version)
if [ "$installed_version" != "$new_version" ]; then
    fail "installed bridge version does not match the new binary"
fi

"$installed_binary" service install
if ! wait_for_service_state "$installed_binary" "service loaded"; then
    fail "new bridge service did not reach loaded state before the bounded deadline"
fi
service_state="service loaded"
"$installed_binary" doctor

printf '%s\n' "bridge replaced: $installed_binary"
printf '%s\n' "version: $installed_version"
printf '%s\n' "$service_state"
