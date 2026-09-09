# Contributing to Mnemos

Thanks for taking a look at Mnemos.

## Setup

Follow [Prerequisites and Getting started](README.md#install) in the README
to get a working dev environment. This document only covers what's specific
to contributing — [AGENTS.md](AGENTS.md) is where the actual architecture,
invariants, and "looks like a bug, isn't" gotchas live. Read it before
touching `src-tauri/src/db/`, the MCP server binary's location, or anything
under `src-tauri/src/metrics/` — each has a real, non-obvious reason for
being shaped the way it is.

## The shape of a PR here

Three languages, three test suites, one repo. A PR that touches Rust,
Python, *and* frontend at once is usually a sign the change should be split
— it makes review harder and makes it unclear which suite actually caught a
regression. The common exception is a new IPC command: that's legitimately
one Rust command plus its generated TypeScript binding plus the frontend
call site, and splitting *that* across PRs would leave an intermediate
commit with a command nothing calls. Use judgment; the point is "don't
bundle unrelated changes," not "never touch two directories."

Two things that are easy to get wrong and will get a PR asked to change:

- **Comments that narrate history or cite documents outside this repo.**
  Explain why the code is shaped the way it is, not what it used to look
  like or what an internal planning doc said. See
  [AGENTS.md](AGENTS.md#non-negotiable-rules).
- **Editing a locked migration file.** If `src-tauri/src/db/migrations/`
  has any file listed in `LOCKED_MIGRATIONS` and your change touches its
  content, you want a new numbered file instead — see
  [AGENTS.md](AGENTS.md#migrations).

## Running the test suites

```sh
# Frontend (React / TypeScript)
pnpm test

# Rust host (src-tauri/)
cargo test --manifest-path src-tauri/Cargo.toml

# Python worker (src-python/)
cd src-python && pytest -q
```

Before opening a PR, also run the linters/typechecker — this is the same
set CI runs, so passing locally means CI passes:

```sh
pnpm lint && pnpm typecheck
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

Installing the pre-commit hooks (`pnpm dlx lefthook install`) runs the
relevant subset of these automatically on commit, so most of this becomes
"just commit" rather than a checklist to remember.

If you add or rename a `#[tauri::command]`, run `cargo test specta_bindings`
before committing — it regenerates `bindings/tauri.ts`, and CI fails on a
dirty diff between what you committed and what a fresh regen produces.

## Building a distributable release (maintainers only)

Contributors don't need any of this — it's only relevant if you're producing
a signed, notarized build to distribute (a DMG someone else will run).
Regular dev builds (`pnpm tauri dev`, or a `pnpm tauri build` without signing
env vars set) work fine without a certificate; the app just runs unsigned,
same as any other local build.

Background: `src-tauri/bundled/<platform>/` holds a locally-built Python
interpreter + dependencies (see `PACKAGING_DESIGN.md` section A), bundled
into the app via `bundle.resources` in `tauri.<platform>.conf.json`. This
directory is `.gitignore`d — it's built once per platform by hand today
(there's no CI pipeline yet) and isn't checked in.

To sign and notarize a macOS build:

1. Have a **Developer ID Application** certificate installed in your login
   Keychain (`security find-identity -v -p codesigning` should list it).
2. Set the signing identity as an environment variable — **not** in
   `tauri.macos.conf.json`, since that file is shared by every contributor
   and shouldn't hardcode one person's certificate:
   ```sh
   export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
   ```
3. Set notarization credentials — either an app-specific password:
   ```sh
   export APPLE_ID="you@example.com"
   export APPLE_PASSWORD="app-specific-password"   # generated at appleid.apple.com
   export APPLE_TEAM_ID="TEAMID"
   ```
   or an App Store Connect API key:
   ```sh
   export APPLE_API_KEY="key-id"
   export APPLE_API_ISSUER="issuer-id"
   export APPLE_API_KEY_PATH="/path/to/AuthKey_XXXX.p8"   # keep this outside the repo
   ```
4. If you changed anything under `src-python/`, refresh the copy inside the
   bundled interpreter — it is a snapshot, not a link, so edits otherwise
   build, sign and run without ever shipping:
   ```sh
   ./scripts/sync_python_bundle.sh
   ```
5. **Before** building, deep-sign every binary inside the bundled Python
   tree. Tauri's own signing step only reaches the app bundle and binaries
   it knows about (the main executable, `externalBin` sidecars) — it does
   not descend into arbitrary files under `Contents/Resources/`, which is
   exactly where the Python interpreter (and its ~230 compiled `.so`
   extension modules — numpy, scipy, onnxruntime, etc.) ends up. Skipping
   this step makes notarization fail with "The binary is not signed with a
   valid Developer ID certificate" for every one of those files:
   ```sh
   ./scripts/sign_python_bundle.sh src-tauri/bundled/python-macos-arm64 "$APPLE_SIGNING_IDENTITY"
   codesign --force --timestamp --options runtime --sign "$APPLE_SIGNING_IDENTITY" \
     src-tauri/binaries/mnemos-audio-aarch64-apple-darwin
   ```
   Re-run this any time the bundle is rebuilt (new/updated dependencies) —
   signatures don't survive a fresh `uv pip install --target`.
6. `pnpm tauri build --bundles dmg`.

The updater's signing key (the "Decrypting updater signing key" password
prompt during a build) is unrelated to any of the above — that's for signing
update artifacts so the app can verify an update came from a trusted source,
not for code-signing the app itself.

Windows installer builds (NSIS/WiX) currently fail on this project's own dev
VM for an environment reason unrelated to the app — see the "Implementation
note" entries in `PACKAGING_DESIGN.md` for the full writeup. A real
interactive Windows session (not an automated/Session-0 one) is expected to
work; nothing here is Mnemos-specific.

## Pull requests

- All three suites above pass, and `bindings/tauri.ts` is up to date if you
  touched an IPC command.
- Keep PRs focused — no unrelated formatting/reformatting churn riding along
  with a functional change. If a file genuinely needs reformatting, do that
  in its own PR.
- Describe *why* in the PR description, not just what changed — the same
  standard this repo holds code comments to. Link any relevant issue.
