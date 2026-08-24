#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
target="aarch64-apple-darwin"
bridge="$project_root/dist/$target/grok-codex-bridge"
launcher_app="$project_root/dist/$target/Grok Codex Switch.app"
launcher_executable="$launcher_app/Contents/MacOS/Grok Codex Switch"
installed_root="${HOME:?HOME is required}/Library/Application Support/grok-codex-bridge"
installed_bridge="$installed_root/bin/grok-codex-bridge"
installed_launcher="$installed_root/bin/Grok Codex Switch.app"
installed_overlay="$installed_launcher/Contents/Resources/grok-codex-bridge-overlay.md"
codex_home="${CODEX_HOME:-"$HOME/.codex"}"
native_catalog="$codex_home/models_cache.json"
grok_overlay="$project_root/Grok.md"
chatgpt_codex="/Applications/ChatGPT.app/Contents/Resources/codex"

usage() {
    printf '%s\n' "usage: $0 grok|native" >&2
    printf '%s\n' "  grok    route Grok models to xAI and Native GPT models to OpenAI" >&2
    printf '%s\n' "  native  route every saved task through Native OpenAI compatibility" >&2
    exit 2
}

fail() {
    printf '%s\n' "error: $1" >&2
    exit 1
}

require_platform() {
    if [ "$(uname -s)" != "Darwin" ] || [ "$(uname -m)" != "arm64" ]; then
        fail "grok-codex migration requires macOS arm64"
    fi
}

ensure_materialized() {
    stale=0
    if [ ! -x "$bridge" ] || [ ! -x "$launcher_executable" ]; then
        stale=1
    else
        for source_file in \
            "$project_root/Cargo.toml" \
            "$project_root/Cargo.lock" \
            "$project_root/rust-toolchain.toml"; do
            if [ "$source_file" -nt "$bridge" ]; then
                stale=1
            fi
        done
        if [ -n "$(find "$project_root/src" -type f -newer "$bridge" -print -quit)" ]; then
            stale=1
        fi
        if [ -n "$(find "$project_root/scripts/macos-switch-launcher" -type f -newer "$launcher_executable" -print -quit)" ]; then
            stale=1
        fi
        if [ "$project_root/Grok.md" -nt "$launcher_executable" ]; then
            stale=1
        fi
    fi

    if [ "$stale" -eq 1 ]; then
        fail "materialized runtime is missing or stale; run ./scripts/materialize-macos.sh before install or update"
    fi
    if [ ! -f "$bridge" ] || [ -L "$bridge" ] || [ ! -x "$bridge" ]; then
        fail "materialized bridge is missing or unsafe: $bridge"
    fi
    if [ ! -d "$launcher_app" ] || [ -L "$launcher_app" ] \
        || [ ! -f "$launcher_executable" ] || [ -L "$launcher_executable" ] \
        || [ ! -x "$launcher_executable" ]; then
        fail "materialized switch launcher is missing or unsafe: $launcher_app"
    fi
}

installation_current() {
    installed_launcher_executable="$installed_launcher/Contents/MacOS/Grok Codex Switch"
    installed_launcher_info="$installed_launcher/Contents/Info.plist"
    [ -f "$installed_overlay" ] && [ ! -L "$installed_overlay" ] \
        && /usr/bin/cmp -s "$bridge" "$installed_bridge" \
        && /usr/bin/cmp -s "$launcher_executable" "$installed_launcher_executable" \
        && /usr/bin/cmp -s "$launcher_app/Contents/Info.plist" "$installed_launcher_info" \
        && /usr/bin/cmp -s "$launcher_app/Contents/Resources/grok-codex-bridge-overlay.md" "$installed_overlay"
}

resolve_chatgpt_upstream() {
    if [ ! -x "$chatgpt_codex" ]; then
        fail "ChatGPT.app Codex executable is unavailable: $chatgpt_codex"
    fi
    if ! login_status=$("$chatgpt_codex" login status 2>&1); then
        fail "unable to determine the ChatGPT.app Codex login route"
    fi
    case "$login_status" in
        "Logged in using ChatGPT")
            native_upstream="https://chatgpt.com/backend-api/codex"
            ;;
        *)
            fail "ChatGPT.app Codex is not logged in using ChatGPT; picker activation was not attempted"
            ;;
    esac
}

verify_native_inputs() {
    if [ ! -f "$native_catalog" ] || [ -L "$native_catalog" ]; then
        fail "authoritative Native Codex catalog is missing or unsafe: $native_catalog"
    fi
    if [ ! -f "$grok_overlay" ] || [ -L "$grok_overlay" ]; then
        fail "Grok overlay is missing or unsafe: $grok_overlay"
    fi
}

ensure_installation() {
    resolve_chatgpt_upstream
    verify_native_inputs
    ensure_materialized

    created_install=0
    rollback_created_install() {
        exit_code=$?
        trap - 0 1 2 15
        if [ "$exit_code" -ne 0 ] && [ "$created_install" -eq 1 ]; then
            "$bridge" uninstall >/dev/null 2>&1 || true
            printf '%s\n' "rollback: removed the incomplete bridge installation" >&2
        fi
        exit "$exit_code"
    }
    trap rollback_created_install 0 1 2 15

    if [ -e "$installed_root" ] || [ -L "$installed_root" ]; then
        if [ ! -d "$installed_root" ] || [ -L "$installed_root" ]; then
            fail "installed bridge root is unsafe: $installed_root"
        fi
        if [ ! -f "$installed_bridge" ] || [ -L "$installed_bridge" ] || [ ! -x "$installed_bridge" ]; then
            fail "installed bridge binary is missing or unsafe: $installed_bridge"
        fi
        installed_launcher_executable="$installed_launcher/Contents/MacOS/Grok Codex Switch"
        if [ ! -d "$installed_launcher" ] || [ -L "$installed_launcher" ] \
            || [ ! -f "$installed_launcher_executable" ] \
            || [ -L "$installed_launcher_executable" ] \
            || [ ! -x "$installed_launcher_executable" ]; then
            if [ -e "$installed_root/state/picker-managed-state.json" ] \
                || [ -L "$installed_root/state/picker-managed-state.json" ]; then
                fail "installed switch launcher is missing while picker state is active: $installed_launcher"
            fi
            if [ "$("$installed_bridge" service status)" != "service not_loaded" ]; then
                fail "installed switch launcher is missing while the bridge service is active"
            fi
            "$installed_bridge" uninstall
            "$bridge" install --source-launcher "$launcher_app"
            created_install=1
        fi
        if ! /usr/bin/codesign --verify --deep --strict "$installed_launcher" 2>/dev/null; then
            fail "installed switch launcher signature is invalid: $installed_launcher"
        fi
    else
        "$bridge" install --source-launcher "$launcher_app"
        created_install=1
        if ! /usr/bin/codesign --verify --deep --strict "$installed_launcher" 2>/dev/null; then
            fail "installed switch launcher signature is invalid: $installed_launcher"
        fi
    fi

    trap - 0 1 2 15
}

schedule_switch() {
    target_mode=$1
    switch_log="$installed_root/logs/mode-switch.log"
    if [ -L "$switch_log" ]; then
        fail "mode switch log is unsafe: $switch_log"
    fi

    printf '%s mode switch requested at %s\n' \
        "$target_mode" "$(/bin/date '+%Y-%m-%d %H:%M:%S %z')" >>"$switch_log"
    if [ "$target_mode" = "native" ]; then
        /usr/bin/open -g "$installed_launcher" --args "$bridge" switch \
            --native-catalog "$native_catalog" \
            --native-upstream-base-url "$native_upstream" \
            --grok-overlay "$grok_overlay" \
            --native-compatibility \
            --replacement-script "$project_root/scripts/replace-installed-bridge.sh" \
            --replacement-launcher "$launcher_app"
    else
        /usr/bin/open -g "$installed_launcher" --args "$bridge" switch \
            --native-catalog "$native_catalog" \
            --native-upstream-base-url "$native_upstream" \
            --grok-overlay "$grok_overlay" \
            --replacement-script "$project_root/scripts/replace-installed-bridge.sh" \
            --replacement-launcher "$launcher_app"
    fi
    printf '%s\n' "$target_mode mode switch handed off to native launcher"
    printf '%s\n' "estimated completion time: approximately 15-20 seconds"
    printf '%s\n' "ChatGPT.app will quit gracefully and relaunch automatically"
    printf '%s\n' "transition log: $switch_log"
}

activate_grok() {
    ensure_installation
    if installation_current; then
        exec "$installed_bridge" mode grok
    fi
    "$bridge" auth ensure
    "$bridge" catalog refresh --config "$installed_root/config/bridge.toml"
    schedule_switch grok
}

activate_native() {
    ensure_installation
    if installation_current; then
        exec "$installed_bridge" mode native
    fi
    schedule_switch native
}

if [ "$#" -ne 1 ]; then
    usage
fi

require_platform
case "$1" in
    grok) activate_grok ;;
    native) activate_native ;;
    *) usage ;;
esac
