#!/usr/bin/env sh
# The Tauri bundler resolves this package's second `[[bin]]` by its
# hyphenated name (`mnemos-mcp-server`), but the target is declared
# underscored (`mnemos_mcp_server`) because the Windows NSIS bundler
# resolves it the *other* way — see the `[[bin]]` comment in Cargo.toml.
# Cargo only ever emits the underscored file, so bundling fails on macOS
# with "Failed to copy binary from .../mnemos-mcp-server: does not exist"
# unless a hyphenated copy is staged first.
#
# This ran silently on a stale pre-rename artifact until it was deleted,
# which meant builds were shipping an out-of-date MCP server binary under
# the hyphenated name. Staging the copy from the current build makes both
# names the same current code. Runtime discovery (`mcp_shared.rs`) only
# ever looks for the underscored one.
set -eu
profile="${1:-release}"
dir="$(cd "$(dirname "$0")/.." && pwd)/src-tauri/target/$profile"
src="$dir/mnemos_mcp_server"
[ -f "$src" ] || { echo "no $src — build it first" >&2; exit 1; }
cp -f "$src" "$dir/mnemos-mcp-server"
echo "staged $dir/mnemos-mcp-server from $src"
