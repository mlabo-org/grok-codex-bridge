#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

if [ "$(uname -s)" != "Darwin" ] || [ "$(uname -m)" != "arm64" ]; then
    printf '%s\n' "error: materialization currently supports macOS arm64 only" >&2
    exit 1
fi

target="aarch64-apple-darwin"
binary_name="grok-codex-bridge"
build_root=$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/grok-codex-materialize.XXXXXX")
cargo_target_dir="$build_root/cargo"
source_binary="$cargo_target_dir/$target/release/$binary_name"
destination_dir="$project_root/dist/$target"
destination_binary="$build_root/$binary_name"
launcher_source="$project_root/scripts/macos-switch-launcher/main.swift"
launcher_info="$project_root/scripts/macos-switch-launcher/Info.plist"
launcher_app="$build_root/Grok Codex Switch.app"
launcher_contents="$launcher_app/Contents"
launcher_executable="$launcher_contents/MacOS/Grok Codex Switch"
launcher_resources="$launcher_contents/Resources"
verification_root="$build_root/verification"
verification_app="$verification_root/Grok Codex Switch.app"

cleanup() {
    /bin/rm -rf "$build_root"
}
trap cleanup 0
trap 'exit 129' 1
trap 'exit 130' 2
trap 'exit 143' 15

cd "$project_root"
cargo build --release --locked --target "$target" --target-dir "$cargo_target_dir"
install -m 755 "$source_binary" "$destination_binary"
mkdir -p "$launcher_contents/MacOS"
mkdir -p "$launcher_resources"
/usr/bin/swiftc "$launcher_source" -module-cache-path "$build_root/swift-module-cache" -o "$launcher_executable"
install -m 644 "$launcher_info" "$launcher_contents/Info.plist"
install -m 644 "$project_root/Grok.md" "$launcher_resources/grok-codex-bridge-overlay.md"
case "$(/usr/bin/file -b "$destination_binary")" in
    *Mach-O*arm64*) ;;
    *)
        printf '%s\n' "error: materialized bridge is not a macOS arm64 Mach-O executable" >&2
        exit 1
        ;;
esac
case "$(/usr/bin/file -b "$launcher_executable")" in
    *Mach-O*arm64*) ;;
    *)
        printf '%s\n' "error: materialized switch launcher is not a macOS arm64 Mach-O executable" >&2
        exit 1
        ;;
esac
/usr/bin/xattr -cr "$launcher_app"
/usr/bin/codesign --force --deep --sign - "$launcher_app"
/usr/bin/xattr -cr "$launcher_app"
mkdir -p "$verification_root"
/usr/bin/ditto "$launcher_app" "$verification_app"
/usr/bin/xattr -cr "$verification_app"
/usr/bin/codesign --verify --deep --strict "$verification_app"
/usr/bin/cmp "$project_root/Grok.md" "$launcher_resources/grok-codex-bridge-overlay.md"

# Publish only the complete, verified pair; Cargo's configured or pre-existing
# output trees are never used as the copy source or deleted by this operation.
mkdir -p "$destination_dir"
install -m 755 "$destination_binary" "$destination_dir/$binary_name"
/bin/rm -rf "$destination_dir/Grok Codex Switch.app"
/usr/bin/ditto "$launcher_app" "$destination_dir/Grok Codex Switch.app"
"$destination_dir/$binary_name" --version
printf '%s\n' "$destination_dir/$binary_name"
printf '%s\n' "$destination_dir/Grok Codex Switch.app"
