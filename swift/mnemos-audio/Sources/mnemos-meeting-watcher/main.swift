// mnemos-meeting-watcher — long-lived background process, spawned once at
// app launch and kept alive for the process lifetime (not per-recording;
// see product_docs/MEETING_AUTO_DETECT_DESIGN.md "Lifecycle").
//
// Watches a fixed allow-list of native meeting-app bundle IDs for
// microphone-activity transitions via CoreAudio's public per-process
// property API (kAudioProcessPropertyIsRunningInput) and emits
// meeting_detected/meeting_ended over the same line-delimited JSON stdio
// protocol mnemos-audio uses. v1 scope is native apps only — no browser-tab
// detection (that would need AppleScript Automation permission per
// browser, cut from v1 by product decision).
//
// Detection mechanism (polling, not property listeners) and its debounce
// numbers were validated empirically against this exact API on 2026-09-04:
// a standalone spike tracked QuickTimePlayerX's mic state flipping 0->1->0
// across three real recordings with zero TCC prompt. Polling was chosen
// over AudioObjectAddPropertyListenerBlock because polling is what was
// actually verified; listener-based per-process watching is a possible
// follow-up optimization, not a requirement — at a 3s tick this costs
// nothing measurable.

import CoreAudio
import Foundation
import MnemosAudioKit

setvbuf(stdout, nil, _IONBF, 0)

let pollIntervalS: TimeInterval = 3.0
/// Consecutive confirming polls before a transition is reported — mirrors
/// OpenWhispr's own numbers (their source comments explain shorter windows
/// caught normal mic-permission-check blips in their testing). Not
/// independently re-derived here; carried over as a reasonable starting
/// point, adjustable once this ships and produces real telemetry.
let debounceConfirmations = 2

/// v1 scope: native meeting apps only. Keyed by bundle ID exactly as
/// CoreAudio reports it (kAudioProcessPropertyBundleID), not by display
/// name.
let knownMeetingApps: [String: String] = [
    "us.zoom.xos": "zoom",
    "com.microsoft.teams": "teams",
    "com.microsoft.teams2": "teams",
    "com.cisco.webexmeetingsapp": "webex",
    "com.apple.FaceTime": "facetime",
    "com.apple.QuickTimePlayerX": "zoom", // TEMP TEST ONLY — remove before any real build
]

struct AppState {
    var reportedActive = false
    var consecutiveActive = 0
    var consecutiveInactive = 0
}

var states: [String: AppState] = [:]  // keyed by bundle ID

func tick() {
    var seenBundleIDs = Set<String>()

    for process in allAudioProcessObjects() {
        guard let bundleID = readStringProperty(process, kAudioProcessPropertyBundleID),
              let service = knownMeetingApps[bundleID] else { continue }
        seenBundleIDs.insert(bundleID)

        let isRunningInput: UInt32 = readProperty(process, kAudioProcessPropertyIsRunningInput, UInt32(0)) ?? 0
        var state = states[bundleID] ?? AppState()

        if isRunningInput != 0 {
            state.consecutiveActive += 1
            state.consecutiveInactive = 0
            if !state.reportedActive, state.consecutiveActive >= debounceConfirmations {
                state.reportedActive = true
                emitEvent("meeting_detected", [
                    "service": service,
                    "bundle_id": bundleID,
                    "source": "app",
                    "at_ms": nowMs(),
                ])
            }
        } else {
            state.consecutiveInactive += 1
            state.consecutiveActive = 0
            if state.reportedActive, state.consecutiveInactive >= debounceConfirmations {
                state.reportedActive = false
                emitEvent("meeting_ended", [
                    "service": service,
                    "bundle_id": bundleID,
                    "at_ms": nowMs(),
                ])
            }
        }
        states[bundleID] = state
    }

    // A tracked app that quit entirely (no longer in the process list) is
    // not the same as "mic went quiet" — its object simply stops appearing.
    // Treat a vanished, previously-active app as an immediate end rather
    // than waiting on a poll that will never see it again.
    for (bundleID, state) in states where state.reportedActive && !seenBundleIDs.contains(bundleID) {
        var updated = state
        updated.reportedActive = false
        states[bundleID] = updated
        emitEvent("meeting_ended", [
            "service": knownMeetingApps[bundleID] ?? bundleID,
            "bundle_id": bundleID,
            "at_ms": nowMs(),
            "reason": "process_exited",
        ])
    }
}

emitEvent("started", ["at_ms": nowMs()])

while true {
    tick()
    Thread.sleep(forTimeInterval: pollIntervalS)
}
