#!/usr/bin/env bash
# Full macOS build: Swift sidecar -> signing -> Tauri bundle -> DMG, and
# optionally install to /Applications.
#
# This exists because the sequence has ordering constraints that are not
# obvious and fail in ways that are hard to attribute:
#
#   - The bundled Python must be deep-signed *with entitlements* before
#     `tauri build` copies it into Resources. Signed with `--options runtime`
#     and no entitlements, the app builds and launches fine and then the
#     worker is SIGKILLed the moment numba's LLVM JIT runs — "SIGKILL (Code
#     Signature Invalid)", with transcription dead in the packaged app while
#     dev keeps working.
#   - `src-python/` is *copied* into the bundled interpreter, not linked, so
#     Python edits silently do not ship unless synced first.
#   - The Tauri bundler resolves the MCP server binary by its hyphenated
#     name while cargo emits the underscored one, so a hyphenated copy has to
#     be staged after compiling and before bundling.
#   - The Swift sidecar has no build step in the Tauri pipeline at all; a
#     stale binary in `src-tauri/binaries/` ships silently.
#
# Every one of those has cost a debugging session. Run this instead of the
# individual steps.
#
# Usage:
#   scripts/build_macos_app.sh [--install] [--skip-python-sign] [--skip-swift]
#
#   --install            ditto the built .app into /Applications (quits a
#                        running instance first)
#   --skip-python-sign   skip re-signing the 234 Mach-O binaries in the
#                        bundled interpreter (~1 min). Safe ONLY if the
#                        bundle has not changed since the last signed build.
#   --skip-swift         skip rebuilding the Swift sidecar.
#
# Requires APPLE_SIGNING_IDENTITY. Notarization is NOT performed here — it
# needs APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID (or an API key) and is only
# required for builds distributed to other machines, not local testing.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"

INSTALL=0
SKIP_PYTHON_SIGN=0
SKIP_SWIFT=0
for arg in "$@"; do
  case "$arg" in
    --install) INSTALL=1 ;;
    --skip-python-sign) SKIP_PYTHON_SIGN=1 ;;
    --skip-swift) SKIP_SWIFT=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

: "${APPLE_SIGNING_IDENTITY:?set APPLE_SIGNING_IDENTITY, e.g. export APPLE_SIGNING_IDENTITY=\"Developer ID Application: Your Name (TEAMID)\"}"

BUNDLE_DIR="src-tauri/bundled/python-macos-arm64"
ENTITLEMENTS="src-tauri/Entitlements.plist"
SIDECAR_DEST="src-tauri/binaries/mnemos-audio-aarch64-apple-darwin"
WATCHER_DEST="src-tauri/binaries/mnemos-meeting-watcher-aarch64-apple-darwin"

step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

# 1. Swift sidecar + meeting watcher. Nothing in the Tauri pipeline builds
#    either, so without this step a source change to either one simply does
#    not reach the app. One `swift build` produces both executable targets
#    (they share the MnemosAudioKit package) — no separate build per binary.
if [ "$SKIP_SWIFT" -eq 0 ]; then
  step "Building Swift audio sidecar + meeting watcher"
  (cd swift/mnemos-audio && swift build -c release)
  SWIFT_BIN_DIR="$(cd swift/mnemos-audio && swift build -c release --show-bin-path)"
  cp "$SWIFT_BIN_DIR/mnemos-audio" "$SIDECAR_DEST"
  cp "$SWIFT_BIN_DIR/mnemos-meeting-watcher" "$WATCHER_DEST"
  codesign --force --timestamp --options runtime \
    --entitlements "$ENTITLEMENTS" \
    --sign "$APPLE_SIGNING_IDENTITY" "$SIDECAR_DEST"
  codesign --force --timestamp --options runtime \
    --entitlements "$ENTITLEMENTS" \
    --sign "$APPLE_SIGNING_IDENTITY" "$WATCHER_DEST"
  echo "sidecar + watcher staged and signed"
fi

# 2. Ship current Python. The bundled interpreter holds a *copy* of
#    src-python/mnemos_worker.
step "Syncing Python worker into the bundled interpreter"
./scripts/sync_python_bundle.sh

# 3. Deep-sign the interpreter, entitlements included (the script now
#    requires them; see its header for what happens without).
if [ "$SKIP_PYTHON_SIGN" -eq 0 ]; then
  step "Signing bundled Python (234 Mach-O binaries, ~1 min)"
  ./scripts/sign_python_bundle.sh "$BUNDLE_DIR" "$APPLE_SIGNING_IDENTITY" "$ENTITLEMENTS"
else
  echo "skipping bundled-Python signing (--skip-python-sign)"
fi

# 4. Compile the MCP server first so the hyphenated alias can be staged
#    before bundling. Doing this inside `tauri build` is too late: the
#    bundler looks for the alias immediately after compiling.
step "Building MCP server and staging its bundler alias"
cargo build --release --manifest-path src-tauri/Cargo.toml --bin mnemos_mcp_server
./scripts/stage_mcp_server_alias.sh

step "Building and bundling the app"
pnpm tauri build --bundles dmg

DMG="src-tauri/target/release/bundle/dmg/Mnemos_0.1.0_aarch64.dmg"
[ -f "$DMG" ] || { echo "expected DMG not found at $DMG" >&2; exit 1; }

if [ "$INSTALL" -eq 1 ]; then
  step "Installing to /Applications"
  pkill -f "Mnemos.app/Contents/MacOS/mnemos-tauri" 2>/dev/null || true
  sleep 2
  hdiutil detach /Volumes/Mnemos >/dev/null 2>&1 || true
  MNT="$(hdiutil attach -nobrowse -readonly "$DMG" | grep -o '/Volumes/.*$' | tail -1)"
  rm -rf /Applications/Mnemos.app
  ditto "$MNT/Mnemos.app" /Applications/Mnemos.app
  hdiutil detach "$MNT" >/dev/null

  step "Verifying the installed app"
  codesign --verify --deep --strict /Applications/Mnemos.app
  echo "signature: valid"
  # The entitlement whose absence breaks transcription in a way that only
  # shows up at runtime, long after the build reported success.
  for probe in bin/python3.12 lib/python3.12/site-packages/llvmlite/binding/libllvmlite.dylib; do
    n=$(codesign -d --entitlements - \
      "/Applications/Mnemos.app/Contents/Resources/python/$probe" 2>/dev/null \
      | grep -c "allow-unsigned-executable-memory" || true)
    [ "$n" -ge 1 ] || { echo "FAIL: $probe is missing JIT entitlements" >&2; exit 1; }
  done
  echo "bundled Python: JIT entitlements present"
  echo
  echo "installed: /Applications/Mnemos.app"
else
  echo
  echo "built: $DMG"
  echo "(pass --install to install it)"
fi
