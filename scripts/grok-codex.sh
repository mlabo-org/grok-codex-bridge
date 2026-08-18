#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
target="aarch64-apple-darwin"
bridge="$project_root/dist/$target/grok-codex-bridge"
materialize_command="./scripts/materialize-macos.sh"

if [ "$(uname -s)" != "Darwin" ] || [ "$(uname -m)" != "arm64" ]; then
    printf '%s\n' "error: grok-codex requires macOS arm64" >&2
    exit 1
fi

if [ ! -x "$bridge" ]; then
    printf '%s\n' "error: prebuilt bridge is missing; run $materialize_command" >&2
    exit 1
fi

for source_file in \
    "$project_root/Cargo.toml" \
    "$project_root/Cargo.lock" \
    "$project_root/rust-toolchain.toml"; do
    if [ "$source_file" -nt "$bridge" ]; then
        printf '%s\n' "error: prebuilt bridge is stale; run $materialize_command" >&2
        exit 1
    fi
done

newer_source=$(find "$project_root/src" -type f -newer "$bridge" -print -quit)
if [ -n "$newer_source" ]; then
    printf '%s\n' "error: prebuilt bridge is stale; run $materialize_command" >&2
    exit 1
fi

launch_codex=1
if [ "${1:-}" = "--activate-only" ]; then
    launch_codex=0
    shift
fi

if [ "$launch_codex" -eq 1 ]; then
    codex_binary=$(command -v codex || true)
    if [ -z "$codex_binary" ] || [ ! -x "$codex_binary" ]; then
        printf '%s\n' "error: codex executable is unavailable" >&2
        exit 1
    fi
fi

install_root="${HOME:?HOME is required}/Library/Application Support/grok-codex-bridge"
rollback_new_install=0

rollback() {
    exit_code=$?
    trap - 0 1 2 15
    if [ "$rollback_new_install" -eq 1 ]; then
        "$bridge" service uninstall >/dev/null 2>&1 || true
        "$bridge" uninstall >/dev/null 2>&1 || true
        printf '%s\n' "error: activation failed; the new bridge installation was rolled back" >&2
    fi
    exit "$exit_code"
}
trap rollback 0 1 2 15

if [ -e "$install_root" ] || [ -L "$install_root" ]; then
    "$bridge" doctor
else
    "$bridge" install
    rollback_new_install=1
    "$bridge" doctor
fi

if ! service_state=$("$bridge" service status); then
    printf '%s\n' "error: unable to determine bridge service state" >&2
    exit 1
fi

case "$service_state" in
    "service loaded")
        ;;
    "service not_loaded")
        "$bridge" service install
        ;;
    *)
        printf '%s\n' "error: bridge service is not healthy" >&2
        exit 1
        ;;
esac

if [ "$("$bridge" service status)" != "service loaded" ]; then
    printf '%s\n' "error: bridge service did not reach loaded state" >&2
    exit 1
fi

rollback_new_install=0
trap - 0 1 2 15
printf '%s\n' "grok-codex ready: profile grok-bridge"

if [ "$launch_codex" -eq 0 ]; then
    exit 0
fi

exec "$codex_binary" --profile grok-bridge "$@"
