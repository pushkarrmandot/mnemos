#!/usr/bin/env bash
# Deep-signs every Mach-O binary inside a bundled Python interpreter tree
# with a real Developer ID identity + secure timestamp + hardened runtime.
#
# Why this exists: Tauri's own app-level codesign step only reaches the app
# bundle and binaries it knows about (the main executable, externalBin
# sidecars). It does not descend into arbitrary files under
# `Contents/Resources/` — which is exactly where `bundle.resources` places
# the whole `python-macos-arm64/` tree (PACKAGING_DESIGN.md section A/B).
# That tree is full of compiled extension modules (numpy, scipy, onnxruntime,
# etc.) that ship unsigned or signed by whoever built the wheel, not by us.
# Apple's notarizer requires every Mach-O in the submitted bundle to carry a
# valid Developer ID signature with a secure timestamp, so those fail
# notarization until they're re-signed here — before `pnpm tauri build`
# copies the tree into Resources and does its own outer sign+notarize pass.
#
# Hardened runtime (`--options runtime`, required for notarization) also has
# to be paired with an entitlements file here, not just on the main app:
# the bundled interpreter is spawned as its own process, so it needs its own
# `disable-library-validation` (to dlopen the extension modules) and
# `allow-unsigned-executable-memory` (mlx JIT-compiles Metal kernels). Signed
# with hardened runtime but no entitlements, transcription fails at runtime
# in a packaged build while working fine in dev.
#
# The entitlements argument is REQUIRED, not optional. It was optional once,
# and omitting it produced exactly the failure described above: 234 binaries
# stamped with hardened runtime and an empty entitlement set, a bundled
# interpreter the kernel SIGKILLs the moment numba's LLVM JIT (or mlx's
# Metal JIT) executes a generated page — "SIGKILL (Code Signature Invalid)",
# CODESIGNING / Invalid Page — and a packaged app whose transcription is
# dead while dev keeps working. There is no case where signing this tree
# with `--options runtime` and no entitlements is correct, so the argument
# is now mandatory rather than a step that can be silently skipped.
#
# Usage: scripts/sign_python_bundle.sh <bundle-dir> "<Developer ID identity>" <entitlements.plist>
set -euo pipefail

usage="usage: sign_python_bundle.sh <bundle-dir> <identity> <entitlements.plist>"
BUNDLE_DIR="${1:?$usage}"
IDENTITY="${2:?$usage}"
ENTITLEMENTS="${3:?$usage — entitlements are required, see the header}"

if [ ! -f "$ENTITLEMENTS" ]; then
  echo "error: entitlements file not found: $ENTITLEMENTS" >&2
  exit 1
fi

sign_args=(--force --timestamp --options runtime --sign "$IDENTITY"
           --entitlements "$ENTITLEMENTS")

if [ ! -d "$BUNDLE_DIR" ]; then
  echo "error: $BUNDLE_DIR is not a directory" >&2
  exit 1
fi

count=0
# Candidates: anything with the executable bit, or a .so/.dylib extension —
# then filter to actual Mach-O so we don't waste a codesign call (or fail)
# on shell scripts / text files that also happen to be +x.
while IFS= read -r -d '' f; do
  if file -b "$f" | grep -q "Mach-O"; then
    codesign "${sign_args[@]}" "$f"
    count=$((count + 1))
  fi
done < <(find "$BUNDLE_DIR" \( -perm -u+x -o -name "*.so" -o -name "*.dylib" \) -type f -print0)

echo "signed $count Mach-O binaries under $BUNDLE_DIR"
