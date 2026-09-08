#!/usr/bin/env bash
# Refreshes the `mnemos_worker` source inside the bundled interpreter tree.
#
# Why this exists: `src-tauri/bundled/python-<platform>/.../site-packages/
# mnemos_worker/` is a *copy*, not a link, so edits under `src-python/` do
# not reach a packaged build until someone re-copies them. That fails
# silently — the app builds, signs and runs, just with the old worker code.
# It cost a full build/install/test cycle to notice a fix "not working" that
# had simply never shipped.
#
# Run this after any change under `src-python/` and before `pnpm tauri
# build`. Idempotent.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/src-python/mnemos_worker"

shopt -s nullglob
targets=("$ROOT"/src-tauri/bundled/python-*/lib/python*/site-packages)
if [ ${#targets[@]} -eq 0 ]; then
  echo "no bundled interpreter found — nothing to sync (build one first)" >&2
  exit 0
fi

for site in "${targets[@]}"; do
  rm -rf "${site:?}/mnemos_worker"
  cp -R "$SRC" "$site/"
  # Stale bytecode from the dev tree would ship alongside the sources.
  find "$site/mnemos_worker" -name "__pycache__" -type d -prune -exec rm -rf {} + 2>/dev/null || true
  echo "synced -> $site/mnemos_worker"
done
