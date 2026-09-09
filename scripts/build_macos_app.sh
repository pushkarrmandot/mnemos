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
#   - A DMG is not an updater artifact. `tauri-plugin-updater` fetches a
#     `latest.json` naming a `.app.tar.gz` and its detached signature; a
#     release carrying only a DMG is installable but invisible to every
#     already-installed copy of the app, with no error anywhere.
#
# Every one of those has cost a debugging session. Run this instead of the
# individual steps.
#
# Usage:
#   scripts/build_macos_app.sh [--install] [--skip-python-sign] [--skip-swift]
#                              [--skip-updater]
#
#   --install            ditto the built .app into /Applications (quits a
#                        running instance first)
#   --skip-python-sign   skip re-signing the 234 Mach-O binaries in the
#                        bundled interpreter (~1 min). Safe ONLY if the
#                        bundle has not changed since the last signed build.
#   --skip-swift         skip rebuilding the Swift sidecar.
#   --skip-updater       build the DMG only, skipping the updater artifacts
#                        and the signing key they need. For fast local
#                        iteration — NEVER for a build you intend to ship.
#
# Requires APPLE_SIGNING_IDENTITY.
#
# Updater artifacts are produced by DEFAULT, not on an opt-in flag: the two
# failure modes are not symmetric. Forgetting the flag on a local test build
# fails loudly and immediately ("no private key"); forgetting it on a release
# ships a version nobody can update to, and nothing says so until users are
# stranded on an old build. So the default is the safe one and the shortcut
# is explicit. They need TAURI_SIGNING_PRIVATE_KEY (the contents of
# ~/.tauri/mnemos-updater.key, or a path to it) plus
# TAURI_SIGNING_PRIVATE_KEY_PASSWORD.
#
# Notarization is performed only when APPLE_ID + APPLE_PASSWORD +
# APPLE_TEAM_ID are all exported (APPLE_PASSWORD being an app-specific
# password, not the account one). Without them the build still succeeds and
# is signed, but Gatekeeper refuses it on every Mac other than this one —
# fine for local testing, useless for distribution.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"

INSTALL=0
SKIP_PYTHON_SIGN=0
SKIP_SWIFT=0
SKIP_UPDATER=0
for arg in "$@"; do
  case "$arg" in
    --install) INSTALL=1 ;;
    --skip-python-sign) SKIP_PYTHON_SIGN=1 ;;
    --skip-swift) SKIP_SWIFT=1 ;;
    --skip-updater) SKIP_UPDATER=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

: "${APPLE_SIGNING_IDENTITY:?set APPLE_SIGNING_IDENTITY, e.g. export APPLE_SIGNING_IDENTITY=\"Developer ID Application: Your Name (TEAMID)\"}"

# Single source of truth: `tauri.conf.json` carries no `version` key, so
# Tauri reads it from here — and so must every path below. It used to be
# spelled `0.1.0` inline in the DMG path, which meant the first version bump
# would have failed with "expected DMG not found" against a DMG that had in
# fact built correctly.
VERSION="$(awk -F'"' '/^version = "/ { print $2; exit }' src-tauri/Cargo.toml)"
[ -n "$VERSION" ] || { echo "could not read version from src-tauri/Cargo.toml" >&2; exit 1; }

# Fail before the ~10-minute build rather than after it: the signing key is
# only consulted at bundling time, at the very end.
if [ "$SKIP_UPDATER" -eq 0 ] && [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  cat >&2 <<'MSG'
TAURI_SIGNING_PRIVATE_KEY is not set, and updater artifacts are on by default.

  export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/mnemos-updater.key)"
  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="..."

Or pass --skip-updater for a DMG-only local build (never for a release).
MSG
  exit 1
fi

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

# `app` is what makes Tauri emit the updater artifacts (`createUpdaterArtifacts`
# in tauri.conf.json only takes effect for an updater-capable target — `dmg`
# is not one). The .app is built for the DMG regardless, so adding it costs
# only the tar + signature.
step "Building and bundling the app"
if [ "$SKIP_UPDATER" -eq 0 ]; then
  pnpm tauri build --bundles app,dmg
else
  echo "updater artifacts skipped (--skip-updater) — DMG only, NOT shippable"
  pnpm tauri build --bundles dmg
fi

if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
  echo "notarization: submitted by the bundler (see its log above)"
else
  echo "NOTE: not notarized (APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID unset)."
  echo "      Gatekeeper will refuse this build on any other Mac."
fi

DMG="src-tauri/target/release/bundle/dmg/Mnemos_${VERSION}_aarch64.dmg"
[ -f "$DMG" ] || { echo "expected DMG not found at $DMG" >&2; exit 1; }

# The bundler notarizes and staples the .app, then wraps it in a DMG — and
# the DMG itself never gets a pass, so it ships as "Unnotarized Developer
# ID". The app inside launches fine once installed (its own ticket is
# stapled), but Gatekeeper warns when the downloaded disk image is *opened*,
# which is the first thing a new user does. Verified against a real build:
#   spctl on the .app  -> accepted, source=Notarized Developer ID
#   spctl on the .dmg  -> rejected, source=Unnotarized Developer ID
# Submitting the DMG is quick, since Apple already knows its contents.
if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
  step "Notarizing and stapling the DMG"
  xcrun notarytool submit "$DMG" \
    --apple-id "$APPLE_ID" \
    --password "$APPLE_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" \
    --wait
  xcrun stapler staple "$DMG"
  # Belt and braces: a stapled ticket that Gatekeeper still rejects means
  # the DMG would warn on every download, and finding that out from a user
  # report rather than here is the whole failure this step exists to avoid.
  spctl -a -t install -vvv "$DMG" 2>&1 | sed 's/^/  /'
  xcrun stapler validate "$DMG" >/dev/null \
    && echo "DMG: notarized and stapled" \
    || { echo "FAIL: DMG has no stapled ticket after notarization" >&2; exit 1; }
else
  echo "NOTE: DMG not notarized (APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID unset)."
fi

# 6. The updater manifest. Tauri produces the tarball and its detached
#    signature but not the `latest.json` that names them — that is the file
#    `tauri.conf.json`'s `endpoints` actually fetches, so without it the
#    plugin gets a 404 on every check and reports "no update available".
if [ "$SKIP_UPDATER" -eq 0 ]; then
  step "Writing the updater manifest (latest.json)"
  MACOS_BUNDLE_DIR="src-tauri/target/release/bundle/macos"
  TARBALL="$MACOS_BUNDLE_DIR/Mnemos.app.tar.gz"
  [ -f "$TARBALL" ] || { echo "expected updater tarball not found at $TARBALL" >&2; exit 1; }
  [ -f "$TARBALL.sig" ] || { echo "expected signature not found at $TARBALL.sig" >&2; exit 1; }

  # The download URL is derived from the configured update endpoint rather
  # than written out again here: two hardcoded copies of the repo path is
  # how you end up publishing a manifest pointing at a repo that does not
  # exist, which is exactly the bug the endpoint itself had.
  OUT="src-tauri/target/release/bundle/latest.json" \
  VERSION="$VERSION" TARBALL="$TARBALL" \
  python3 - <<'PYEOF'
import json, os, pathlib, datetime

conf = json.load(open("src-tauri/tauri.conf.json"))
endpoint = conf["plugins"]["updater"]["endpoints"][0]
marker = "/releases/latest/download/latest.json"
if not endpoint.endswith(marker):
    raise SystemExit(f"unexpected updater endpoint shape: {endpoint!r}")
repo = endpoint[: -len(marker)]

version = os.environ["VERSION"]
tarball = pathlib.Path(os.environ["TARBALL"])
manifest = {
    "version": version,
    "notes": f"Mnemos {version}",
    "pub_date": datetime.datetime.now(datetime.timezone.utc)
    .replace(microsecond=0)
    .isoformat()
    .replace("+00:00", "Z"),
    "platforms": {
        # Apple Silicon only, matching the sidecars and the bundled
        # interpreter. An Intel Mac finds no entry and is told there is no
        # update, which is true — there is no build for it.
        "darwin-aarch64": {
            "signature": pathlib.Path(f"{tarball}.sig").read_text().strip(),
            "url": f"{repo}/releases/download/v{version}/{tarball.name}",
        }
    },
}
out = pathlib.Path(os.environ["OUT"])
out.write_text(json.dumps(manifest, indent=2) + "\n")
print(f"wrote {out} -> {manifest['platforms']['darwin-aarch64']['url']}")
PYEOF
fi

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

if [ "$SKIP_UPDATER" -eq 0 ]; then
  echo
  echo "Upload ALL FOUR to the GitHub release, with these exact names —"
  echo "the manifest's URL and the plugin's endpoint both depend on them:"
  echo "  $DMG"
  echo "  $TARBALL"
  echo "  $TARBALL.sig"
  echo "  src-tauri/target/release/bundle/latest.json"
  echo
  echo "The release tag must be exactly v${VERSION}, and the release must be"
  echo "published (not a draft) and NOT marked pre-release — /releases/latest/"
  echo "resolves to neither, so the updater would 404 on both."
fi
