# Windows dev setup

Everything a contributor needs to build and run Mnemos **from source** on
Windows. This is a developer bootstrap, not an end-user installer — a real
user should eventually be able to download a signed `.msi` and double-click
it with none of this. That packaged installer doesn't exist yet (see
"What's still missing" below); until it does, this is the path.

## Quick start

Open PowerShell **as Administrator** (installers need elevation), then:

```powershell
cd windows-dev-setup
.\setup.ps1
```

The script is idempotent — safe to re-run if it fails partway through, it
skips anything already installed. It installs, in order:

1. **Rust** (via `rustup`)
2. **Visual Studio Build Tools**, C++ workload only — required for the MSVC
   linker. This is the slow step (multi-GB download, 15–40+ min).
3. **LLVM/clang** — **ARM64 hosts only** (Windows-on-ARM devices, or a
   Parallels/UTM VM on Apple Silicon). The `ring` crate (pulled in via
   Tauri's TLS stack) needs `clang` to assemble on
   `aarch64-pc-windows-msvc`; standard x64 Windows doesn't hit this, `ring`
   ships pregenerated x86_64 assembly. Skipped automatically on x64.
4. **Node.js LTS + pnpm**
5. **Python 3.12 + `uv`** (the worker's dependency/venv manager)
6. **PowerShell execution policy** — set to `RemoteSigned` machine-wide if it
   was still the default `Restricted`. Without this, typing `claude` (or
   `pnpm`, or any other globally-installed npm CLI) in PowerShell fails with
   "running scripts is disabled on this system" — npm generates a `.ps1`
   shim for every global install, and PowerShell prefers it over the
   `.cmd`/`.exe` sitting right next to it, so this bites every npm global
   tool, not just Claude Code.
7. **Claude Code CLI** (`npm install -g @anthropic-ai/claude-code`) — the app
   shells out to this as its agent runner. Installing it doesn't log you in;
   run `claude` once in a fresh terminal afterward and follow the prompt (it
   opens a browser to authorize your account).
8. **Project dependencies** — `pnpm install`, `uv sync` in `src-python/`,
   `cargo build` in `src-tauri/`.

When it finishes, open a **new** terminal window (so the PATH changes it made
actually apply) and run:

```powershell
pnpm tauri dev
```

This starts the Vite dev server and launches the app pointed at it. Running
the compiled `.exe` directly (`target\debug\mnemos-tauri.exe`) without
`pnpm tauri dev`'s dev server running first will show a blank
"can't reach this page" window — the app is trying to load
`http://localhost:1420`, which only exists while the dev server is up.

## Known gap: `pyaudiowpatch` has no ARM64 wheel

If you're on a Windows-on-ARM machine (or an Apple-Silicon VM), `uv sync`
will fail resolving `pyaudiowpatch` — it ships wheels for `win32`/`win_amd64`
only. This blocks the *entire* `uv sync`, not just audio capture, so nothing
in `src-python/` installs at all by default.

Workaround to develop everything except live microphone/system audio capture:

```powershell
cd src-python
uv pip install structlog numpy -e . --no-deps
```

This installs the worker package and its non-audio dependencies directly,
skipping the incompatible one. `capture/wasapi.py` imports `pyaudiowpatch`
lazily (only when a recording actually starts), so the worker starts up and
everything else — onboarding, chat, project memory, transcription-from-an-
existing-file — works normally. Recording itself will fail with an import
error until you're on a real x64 machine or `pyaudiowpatch` ships an ARM64
wheel. See `product_docs/WINDOWS_PARITY_AUDIT.md` for the full context.

## What's still missing (don't expect these to work yet)

- **Real Windows transcription.** The Parakeet backend on Windows is an
  unimplemented stub (`_ParakeetCppBackend`) — you'll see
  `parakeet.warm_up.failed: No module named 'parakeet_cpp'` in the worker
  log. This is expected, not a setup mistake.
- **A packaged installer.** `pnpm tauri build` will compile, but no Python
  interpreter or dependencies are bundled into the output — running the
  built app outside a dev checkout won't find its worker. Bundling this
  (`externalBin`/`resources` in `tauri.conf.json`) is unfinished work.
- Anything else tracked in `product_docs/WINDOWS_PARITY_AUDIT.md`.

## Verified against

This script and the workarounds above were exercised end-to-end against a
real Windows 11 ARM64 VM (Parallels Desktop on Apple Silicon) — not just
inferred from reading code. `product_docs/WINDOWS_PARITY_AUDIT.md` tracks
what's been confirmed on real hardware vs. what's still unverified,
particularly on x64 Windows, which this VM cannot exercise.
