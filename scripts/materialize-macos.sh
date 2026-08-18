#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

if [ "$(uname -s)" != "Darwin" ] || [ "$(uname -m)" != "arm64" ]; then
    printf '%s\n' "error: materialization currently supports macOS arm64 only" >&2
    exit 1
fi

target="aarch64-apple-darwin"
binary_name="grok-codex-bridge"
source_binary="$project_root/target/$target/release/$binary_name"
destination_dir="$project_root/dist/$target"
destination_binary="$destination_dir/$binary_name"

cd "$project_root"
cargo build --release --locked --target "$target"
mkdir -p "$destination_dir"
install -m 755 "$source_binary" "$destination_binary"

printf '%s\n' "$destination_binary"

