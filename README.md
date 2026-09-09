# Mnemos

**Your meetings, remembered — on your machine, queryable by your own AI tools.**

Mnemos records, transcribes, and summarizes your meetings locally, then organizes
what it learns into projects: decisions made, action items owed, questions still
open. It ships its own MCP server, so any MCP-capable agent — Claude Code, Claude
Desktop, or anything else that speaks MCP — can search your own meeting history
and answer questions grounded in what was actually said, without a transcript
ever leaving your machine.

[![License](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey.svg)](#status)

---

## Demo

https://github.com/user-attachments/assets/9b315085-3f63-425b-bee1-8b7d52c60d18

## Use cases

Mnemos doesn't care what the meeting was about — it cares that you'll need to
recall it later. A few concrete shapes this takes:

- **Fundraising.** Every investor call in one project — who said what about
  terms, which questions came up twice, what you told the last VC that
  contradicts what you're about to tell the next one.
- **A product launch.** Kickoff, pricing debate, marketing sync, the retro
  after — decisions and open questions accumulate in one place instead of
  scattering across whoever happened to take notes that day.
- **Hiring.** Screens and panel debriefs for a role, so "what did we actually
  agree the bar was" has an answer three interviews later.
- **Recurring 1:1s.** A running project per report — not a fresh blank page
  every week, but what you said you'd follow up on last time, still there.

## Why not just record in Zoom or Teams

Zoom and Teams recordings are real, but they're built around the call, not
around you. A recording is one file, owned by whoever's account started it,
sitting in that call's own silo — you can't ask it a question, and it doesn't
know about the other twelve meetings related to the same decision.

Mnemos is built around *you* instead: it captures your mic and system audio
directly, so it works the same whether the call is Zoom, Meet, Teams, a phone
on speaker, or a hallway conversation — no dependence on whichever platform's
host happened to click "record." What comes out isn't a video file to
re-watch, but a memory that accumulates across every one of those calls:
decisions and action items that carry from one conversation into the next,
queryable by the AI tools you already use, and never dependent on someone
else's account still existing.

## Why local-first

Meeting content is some of the most sensitive material a person generates —
strategy, compensation, disagreements, half-formed plans. Sending that to a
third-party server to get useful notes back is a real trade most meeting-notes
tools ask you to make. Mnemos doesn't ask you to make it: audio capture,
transcription, and storage all happen on your machine. The one thing that does
leave — the transcript text handed to your configured coding-agent CLI (Claude
today) to produce a summary and extract action items — is between you and
whichever agent you've already chosen to trust with your code.

The MCP server is the other half of the idea: your meeting history shouldn't be
a dead archive you have to remember to open. It should be something the AI tools
you already use can reach into — "what did we decide about pricing last month,"
answered by an agent that actually checked, not guessed.

## What it does today

- **Record and transcribe.** Captures your mic and system audio, transcribes
  locally (Parakeet), shows a live transcript while you're still in the
  meeting.
- **Summarize and extract.** After a recording ends, an agent turn produces a
  summary plus structured action items, decisions, and open questions — with
  each one traceable back to the moment in the transcript it came from.
- **Organize into projects.** Conversations can belong to a project; a
  project's memory (overview, scope drift, decisions, open questions) updates
  as new conversations land in it.
- **Chat, grounded in your meetings.** A persistent chat panel that can answer
  questions scoped to one conversation, one project, or everything — using the
  same MCP tools external agents get.
- **Query from outside the app.** `mnemos-mcp-server` is a second binary this
  repo builds, exposing `mnemos.search`, `mnemos.get_project_memory`,
  `mnemos.list_action_items`, and more over stdio MCP — point Claude Code or
  Claude Desktop at it and ask it about your meetings directly.

**Not yet built:** contacts (speaker identification beyond a session) and
calendar/external integrations are both present in the UI as explicit "coming
soon" placeholders, not silently missing. See [Status](#status) for the honest
platform/build picture.

---

## Install

No published release builds yet — see [Status](#status). For now, run from
source.

### Prerequisites

- Node 22 + pnpm 10 (`corepack enable`)
- Rust stable (`rustup`), MSRV 1.80
- macOS: Xcode Command Line Tools (the audio-capture sidecar is a Swift
  package, macOS-only for now — see [Status](#status))

### Run from source

```sh
pnpm install
pnpm dev            # == pnpm tauri dev — builds Rust, starts Vite, opens the window
```

No account, no login, no server, no API key to run the app itself. It talks to
whichever coding-agent CLI you already have installed (Claude today) to
produce summaries — nothing else calls out.

### Build an installable app yourself

```sh
pnpm tauri build
```

Produces an unsigned `.app`/`.dmg` (macOS) or installer (Windows) under
`src-tauri/target/release/bundle/`. Unsigned means your OS will warn on first
open — right-click → Open (macOS) or "Run anyway" (Windows SmartScreen) past
it once. Signed, notarized release builds are what
[`.github/workflows/release.yml`](.github/workflows/release.yml) produces from
a tagged release, once one exists.

## Checks

```sh
pnpm lint           # biome check
pnpm typecheck      # tsc -b (project references: app + node configs)
pnpm test           # vitest

cargo fmt --check   --manifest-path src-tauri/Cargo.toml
cargo clippy        --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test          --manifest-path src-tauri/Cargo.toml

cd src-python && pytest -q
```

Install the pre-commit hooks with `pnpm dlx lefthook install`.

## Layout

```
src/                 React frontend
  ipc/               the only module allowed to import from bindings/
  components/app/chat/  the chat panel — lives in the app shell, not a route
  routes/            one file per page (dashboard, recording, project, ...)
src-tauri/           Rust host (Tauri v2)
  src/commands/      one file per feature area
  src/bin/mnemos-mcp-server/  the standalone MCP server binary
  src/db/migrations/ SQLite schema — see AGENTS.md before touching this
src-python/          Python worker: transcription + memory-extraction jobs,
                     run as a long-lived subprocess of the Rust host
swift/mnemos-audio/  macOS audio-capture sidecar (SwiftPM), also a subprocess
bindings/tauri.ts    generated by tauri-specta; committed, CI diff-guarded
```

## Generated bindings

`bindings/tauri.ts` is regenerated on every debug boot and by
`cargo test specta_bindings`. Never hand-edit it — CI fails on a dirty diff.
Features import from `@/ipc`, never from `bindings/` directly.

## Status

Early, pre-1.0, single maintainer. macOS is the primary target and what's
tested day to day; Windows parity work is in progress (native audio capture
via WASAPI is implemented, but has seen less real-world testing than macOS).
Linux isn't a target yet — the audio-capture sidecar is Swift/macOS-specific
and there's no Linux capture path.

No signed release builds are published yet — build your own with
`pnpm tauri build` (see [Install](#install)). Bug reports and PRs are welcome;
see [CONTRIBUTING.md](CONTRIBUTING.md).

## Roadmap

Roughly in order. Issues and PRs on any of these are welcome.

0. **Windows build.**
1. **Floating recording indicator.** A small always-on-top pane so it's
   obvious you're being recorded — not just a state hidden inside the main
   window.
2. **Menu-bar/tray recording shortcut (macOS).** Start and stop without
   switching to the app at all.
3. **Diarization.** Tell speakers apart in the transcript, not just capture
   one merged stream.
4. **Speaker recognition.** Remember a voice across meetings — the
   prerequisite for the Contacts page actually doing something.
5. **Calendar integration.** The Integrations page is a placeholder today;
   this is what fills it in.
6. **Semantic search over your meeting history.** `mnemos.search` and chat
   currently match on stored text; a local vector index is what turns "what
   did we say about pricing" into a real semantic query instead of a keyword
   one, across every conversation and project you have.
7. **More coding-agent support beyond Claude** — Codex, OpenCode, Cline,
   Kiro, and others as chat/MCP runners, not just a single hardcoded one.

## Working on Mnemos

- [AGENTS.md](AGENTS.md) — architecture, non-obvious invariants, and things
  that look like bugs but aren't, for both human contributors and AI coding
  agents
- [CONTRIBUTING.md](CONTRIBUTING.md) — dev workflow, verification, PR
  expectations
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- [SECURITY.md](SECURITY.md) — how to report a vulnerability privately

## License

[AGPL-3.0](LICENSE). If you run a modified version of Mnemos as a network
service, you're required to make your modifications' source available to its
users — see the license for the exact terms.
