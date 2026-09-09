import AVFoundation
import CoreAudio
import Foundation
import MnemosAudioKit

// MARK: - Backend protocol

/// A source of *system* audio — what the other participants are saying, as
/// opposed to the microphone.
///
/// Two implementations exist because the mechanism changed across macOS
/// versions, not because we want the choice: `ProcessTapSource` (Core Audio
/// process taps, macOS 14.2+) and `ScreenCaptureKitSource` (everything
/// older). See `product_docs/SYSTEM_AUDIO_CAPTURE_DESIGN.md` for why the tap
/// is strongly preferred where available.
protocol SystemAudioSource: AnyObject {
    /// Reported on the `started` event so we can tell from telemetry which
    /// path a given recording actually used.
    var backendName: String { get }
    func start() throws
    func stop()
}

enum SystemAudioError: Error, CustomStringConvertible {
    /// This macOS build has no process-tap API. Caller should fall back.
    case unsupported(String)
    /// The tap API exists but the user has not granted "System Audio
    /// Recording". Caller should fall back rather than fail the recording.
    case permissionDenied(String)
    case setupFailed(String)

    var description: String {
        switch self {
        case .unsupported(let m): return "unsupported: \(m)"
        case .permissionDenied(let m): return "permission_denied: \(m)"
        case .setupFailed(let m): return "setup_failed: \(m)"
        }
    }
}

// MARK: - Core Audio helpers
//
// propertyAddress/readProperty/readStringProperty/currentProcessAudioObjectID/
// asbdEqual now live in MnemosAudioKit (shared with mnemos-meeting-watcher).

/// True if some process other than us is currently playing audio.
///
/// This is what makes "the tap is producing silence" actionable. Silence on
/// its own is ambiguous — nobody talking looks exactly like a broken tap. But
/// silence *while another process is actively playing output* is not
/// ambiguous at all: that audio should be reaching us and isn't.
private func otherProcessIsPlayingAudio(excluding own: AudioObjectID?) -> Bool {
    for process in allAudioProcessObjects() where process != own {
        if let running: UInt32 = readProperty(process, kAudioProcessPropertyIsRunningOutput, UInt32(0)),
           running != 0 {
            return true
        }
    }
    return false
}

// MARK: - Core Audio process tap

/// System audio via a Core Audio process tap (macOS 14.2+).
///
/// Why this exists at all: the ScreenCaptureKit path routes through
/// `replayd`, a singleton daemon shared with every other screen-capturing app
/// on the machine. It tears down all of a client's streams with
/// `-[RPClient stopAllStreamsWithError:]` — delivered to us as a
/// `didStopWithError(nil)` we can neither predict nor prevent — which killed
/// real recordings mid-meeting. A tap runs against the HAL in-process, so
/// that daemon is not in the picture, and it needs only the narrower
/// "System Audio Recording" permission instead of full Screen Recording.
///
/// The whole reason this class is more than a hundred lines is device
/// changes. A tap that is set up once and never re-examined works fine until
/// someone puts on AirPods, at which point the mixdown format can change
/// underneath it and the cached converter starts producing garbage. Of the
/// several open-source tap implementations surveyed, none handled this; the
/// one shipping product that did is the one that works in the field.
@available(macOS 14.2, *)
final class ProcessTapSource: SystemAudioSource {
    let backendName = "core_audio_tap"

    private let onPCM: (Data) -> Void
    private let onFatal: (String) -> Void

    /// Serialises setup, teardown, rebuild *and* listener callbacks, so none
    /// of them can interleave. Every mutable field below is touched only
    /// here.
    private let queue = DispatchQueue(label: "mnemos.audio.tap")
    /// Marks `queue` so `onQueue` can tell whether it is already running
    /// there. Without this, any path that reaches `stop()` from inside a
    /// queue callback deadlocks on `queue.sync` — which is exactly what the
    /// startup probe did: timer fires on `queue` -> Session's callback ->
    /// `stop()` -> `queue.sync` -> `__DISPATCH_WAIT_FOR_QUEUE__` -> SIGTRAP.
    private let queueKey = DispatchSpecificKey<UInt8>()

    private func onQueue<T>(_ body: () -> T) -> T {
        if DispatchQueue.getSpecific(key: queueKey) != nil { return body() }
        return queue.sync(execute: body)
    }

    private var tapID = AudioObjectID(kAudioObjectUnknown)
    private var aggregateID = AudioObjectID(kAudioObjectUnknown)
    private var ioProcID: AudioDeviceIOProcID?
    private var sourceFormat: AVAudioFormat?
    private var sourceASBD: AudioStreamBasicDescription?
    private let resampler = Resampler()

    /// Registered listeners, kept so they can be removed by identity —
    /// `AudioObjectRemovePropertyListenerBlock` matches on the block object,
    /// so the exact same reference has to come back.
    private var listeners: [(AudioObjectID, AudioObjectPropertyAddress, AudioObjectPropertyListenerBlock)] = []

    private var stopped = false

    /// Set once the tap has delivered at least one non-zero sample.
    ///
    /// A missing "System Audio Recording" grant does **not** surface as an
    /// error: `AudioHardwareCreateProcessTap` succeeds, the IOProc fires at
    /// the normal rate, and every sample is zero. Without a check for that,
    /// a denied permission would silently record an hour of nothing.
    ///
    /// Silence is genuinely ambiguous — nobody talking looks identical — so
    /// this is deliberately a *one-shot startup* probe and never a running
    /// watchdog. The asymmetry is what makes it safe: if we wrongly conclude
    /// the tap is dead we fall back to ScreenCaptureKit, which is what
    /// shipped before, so a false positive costs nothing. A false negative
    /// (staying on a silent tap) costs the user their meeting.
    /// True the moment the IOProc delivers its first buffer, zero-valued
    /// or not. Distinct from `sawAudio` below: a tap that never fires at all
    /// is unambiguously broken and needs no further evidence, whereas a tap
    /// that fires but delivers only zeros is legitimately indistinguishable
    /// from a silent room and needs the playback check before concluding
    /// anything.
    private var sawAnyBuffer = false
    private var sawAudio = false
    private var probeTimer: DispatchSourceTimer?
    /// Long enough that a brief natural pause at the start of a meeting
    /// cannot trip it, short enough to salvage the recording.
    private let silentStartupProbe: TimeInterval = 12

    private var rebuildsInWindow = 0
    private var windowStart = Date()

    /// A device change can produce several notifications at once. Listeners
    /// are removed before a rebuild is queued, but this guards the window
    /// between a notification arriving and that removal taking effect.
    private var rebuildPending = false

    /// Give up after this many rebuilds inside `stabilityWindow`. A machine
    /// whose audio stack is genuinely broken should surface as a clear error
    /// rather than silently respinning forever.
    private let maxRebuildsPerWindow = 20
    private let stabilityWindow: TimeInterval = 300

    /// Called when the startup probe concludes the tap is producing nothing.
    /// The session uses this to abandon the tap and fall back.
    private let onSilentStartup: (String) -> Void

    init(
        onPCM: @escaping (Data) -> Void,
        onFatal: @escaping (String) -> Void,
        onSilentStartup: @escaping (String) -> Void
    ) {
        self.onPCM = onPCM
        self.onFatal = onFatal
        self.onSilentStartup = onSilentStartup
        queue.setSpecific(key: queueKey, value: 1)
    }

    func start() throws {
        var setupError: Error?
        onQueue {
            do {
                try setup()
            } catch {
                setupError = error
            }
        }
        if let setupError { throw setupError }
    }

    func stop() {
        onQueue {
            stopped = true
            teardown()
        }
    }

    // MARK: Setup

    private func setup() throws {
        let description = CATapDescription(
            monoGlobalTapButExcludeProcesses: currentProcessAudioObjectID().map { [$0] } ?? []
        )
        description.name = "Mnemos System Audio"
        description.isPrivate = true
        // Never mute what the user is listening to. `CATapUnmuted` taps the
        // mixdown while leaving playback audible; the other modes would
        // silence the meeting for the person recording it.
        description.muteBehavior = .unmuted

        logErr("tap.desc mono=\(description.isMono) exclusive=\(description.isExclusive)"
            + " mixdown=\(description.isMixdown) private=\(description.isPrivate)"
            + " processes=\(description.processes)")

        var newTapID = AudioObjectID(kAudioObjectUnknown)
        let tapStatus = AudioHardwareCreateProcessTap(description, &newTapID)
        guard tapStatus == noErr else {
            // There is no API to query tap permission — the documented way
            // to find out is to try. `kAudioHardwareIllegalOperationError`
            // is what a missing "System Audio Recording" grant looks like.
            if tapStatus == kAudioHardwareIllegalOperationError {
                throw SystemAudioError.permissionDenied(
                    "AudioHardwareCreateProcessTap: illegal operation (system audio permission not granted)"
                )
            }
            throw SystemAudioError.setupFailed("AudioHardwareCreateProcessTap: OSStatus \(tapStatus)")
        }
        tapID = newTapID

        guard let tapUID = readStringProperty(tapID, kAudioTapPropertyUID) else {
            teardown()
            throw SystemAudioError.setupFailed("could not read kAudioTapPropertyUID")
        }

        // An aggregate device with *no* real sub-devices — the tap is its
        // only member. This is what makes the tap readable through the
        // ordinary device I/O API.
        let aggregateUID = UUID().uuidString
        let aggregateDescription: [String: Any] = [
            kAudioAggregateDeviceNameKey: "Mnemos Aggregate Audio Device",
            kAudioAggregateDeviceUIDKey: aggregateUID,
            kAudioAggregateDeviceIsPrivateKey: true,
            // Root cause of the tap delivering zero buffers in every real
            // test, verified against the SDK header rather than guessed:
            // a non-zero value here means "AudioDeviceStart waits until a
            // tapped process begins receiving its *first* audio" — the
            // IOProc does not fire at all until that transition happens.
            // For a global tap meant to run for an entire meeting and
            // capture whatever plays (including nothing, including audio
            // that was already playing before the tap existed), that is
            // exactly wrong: it produced silence when something was
            // already playing, and never fired a single buffer when
            // nothing happened to start playing during the recording.
            // false is what the one verified-working reference
            // implementation (OpenWhispr's shipping macos-audio-tap.swift)
            // uses.
            kAudioAggregateDeviceTapAutoStartKey: false,
            kAudioAggregateDeviceSubDeviceListKey: [],
            kAudioAggregateDeviceTapListKey: [[kAudioSubTapUIDKey: tapUID]],
        ]
        var newAggregateID = AudioObjectID(kAudioObjectUnknown)
        let aggregateStatus = AudioHardwareCreateAggregateDevice(
            aggregateDescription as CFDictionary,
            &newAggregateID
        )
        guard aggregateStatus == noErr else {
            teardown()
            throw SystemAudioError.setupFailed(
                "AudioHardwareCreateAggregateDevice: OSStatus \(aggregateStatus)"
            )
        }
        aggregateID = newAggregateID

        // The aggregate is not immediately usable; it comes alive
        // asynchronously. Polling here rather than starting I/O optimistically
        // avoids a race where the first buffers are silence.
        var alive = false
        for _ in 0..<20 {
            if let value: UInt32 = readProperty(aggregateID, kAudioDevicePropertyDeviceIsAlive, UInt32(0)),
               value != 0 {
                alive = true
                break
            }
            Thread.sleep(forTimeInterval: 0.1)
        }
        guard alive else {
            teardown()
            throw SystemAudioError.setupFailed("aggregate device never became alive")
        }

        guard let asbd = readTapFormat() else {
            teardown()
            throw SystemAudioError.setupFailed("could not read kAudioTapPropertyFormat")
        }
        var mutableASBD = asbd
        guard let format = AVAudioFormat(streamDescription: &mutableASBD) else {
            teardown()
            throw SystemAudioError.setupFailed("tap format is not representable as an AVAudioFormat")
        }
        sourceASBD = asbd
        sourceFormat = format
        logErr("tap.format sampleRate=\(asbd.mSampleRate) channels=\(asbd.mChannelsPerFrame)"
            + " bitsPerChannel=\(asbd.mBitsPerChannel) bytesPerFrame=\(asbd.mBytesPerFrame)"
            + " formatFlags=\(asbd.mFormatFlags)")

        let ioStatus = AudioDeviceCreateIOProcIDWithBlock(
            &ioProcID,
            aggregateID,
            queue
        ) { [weak self] _, inInputData, _, _, _ in
            self?.handleInput(inInputData)
        }
        guard ioStatus == noErr, ioProcID != nil else {
            teardown()
            throw SystemAudioError.setupFailed("AudioDeviceCreateIOProcIDWithBlock: OSStatus \(ioStatus)")
        }

        let startStatus = AudioDeviceStart(aggregateID, ioProcID)
        guard startStatus == noErr else {
            teardown()
            throw SystemAudioError.setupFailed("AudioDeviceStart: OSStatus \(startStatus)")
        }

        addListeners()
        startSilentStartupProbe()
    }

    /// Detects only the unambiguous failure: the IOProc never fired at all.
    ///
    /// Deliberately does *not* also try to judge whether ordinary silence
    /// (buffers arriving, all zero) means the tap is broken. An earlier
    /// version of this method did — it required proof that another process
    /// was audibly playing before treating silence as a failure, on the
    /// theory that a meeting where only the user has spoken so far looks
    /// identical to a broken tap. That reasoning was sound but the practice
    /// wasn't: it shipped a real production bug (a 1-minute recording with
    /// nothing else playing got an empty `system.wav`, because "nothing is
    /// proven broken yet" kept it waiting), and follow-up testing against a
    /// genuinely denied permission showed the underlying macOS behavior
    /// here is itself non-deterministic across otherwise-identical runs —
    /// no amount of client-side cleverness reliably distinguishes "quiet
    /// room" from "silently failing" from content alone. None of the three
    /// production apps referenced when building this (Otter, Granola,
    /// OpenWhispr) attempt to; they trust the tap/aggregate creation call's
    /// own status code, which is the synchronous check already in `setup()`
    /// above, and stop there.
    ///
    /// Ordinary silence is now simply trusted. The cost of an undetected
    /// silent tap is bounded elsewhere: `process_conversation.py` treats a
    /// zero-frame or all-silent stream as "nothing to transcribe from this
    /// source," not as a fatal error, so the failure mode this leaves is
    /// "one recording is missing the other side's audio," never "the
    /// recording is lost."
    private func startSilentStartupProbe() {
        probeTimer?.cancel()
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now() + silentStartupProbe)
        timer.setEventHandler { [weak self] in
            guard let self else { return }
            self.probeTimer?.cancel()
            self.probeTimer = nil
            guard !self.stopped, !self.sawAnyBuffer else { return }

            let message = "tap delivered zero buffers in \(Int(self.silentStartupProbe))s"
            // Off our own queue: the callback tears this object down, and
            // running that inline would re-enter `queue`.
            DispatchQueue.global().async { [weak self] in
                guard let self, !self.stopped else { return }
                self.onSilentStartup(message)
            }
        }
        timer.resume()
        probeTimer = timer
    }

    private func readTapFormat() -> AudioStreamBasicDescription? {
        var address = propertyAddress(kAudioTapPropertyFormat)
        var asbd = AudioStreamBasicDescription()
        var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
        let status = AudioObjectGetPropertyData(tapID, &address, 0, nil, &size, &asbd)
        return status == noErr ? asbd : nil
    }

    // MARK: Audio path

    private func handleInput(_ bufferList: UnsafePointer<AudioBufferList>) {
        sawAnyBuffer = true
        guard let sourceFormat else { return }
        guard let pcm = AVAudioPCMBuffer(pcmFormat: sourceFormat, bufferListNoCopy: bufferList) else {
            return
        }
        guard let data = resampler.convert(pcm) else { return }
        if !sawAudio, data.contains(where: { $0 != 0 }) {
            sawAudio = true
        }
        onPCM(data)
    }

    // MARK: Device-change listeners

    /// Four listeners on three objects. Only two conditions actually cause a
    /// rebuild — the aggregate reporting dead, and the tap's audio format
    /// genuinely changing. The other two are advisory: they tell us *when to
    /// look*, not what to do.
    ///
    /// That distinction is the whole design. Rebuilding on every
    /// default-output change would tear down healthy capture every time
    /// someone plugs in a monitor; ignoring format changes would leave a
    /// converter configured for a format that no longer arrives, which is
    /// silent corruption rather than a visible failure.
    private func addListeners() {
        register(aggregateID, kAudioDevicePropertyDeviceIsAlive) { [weak self] in
            guard let self else { return }
            let alive: UInt32 = readProperty(self.aggregateID, kAudioDevicePropertyDeviceIsAlive, UInt32(1)) ?? 1
            if alive == 0 {
                self.scheduleRebuild(reason: "aggregate_device_died")
            } else {
                self.checkFormatChanged()
            }
        }
        register(aggregateID, kAudioDevicePropertyNominalSampleRate) { [weak self] in
            self?.checkFormatChanged()
        }
        register(AudioObjectID(kAudioObjectSystemObject), kAudioHardwarePropertyDefaultOutputDevice) {
            [weak self] in
            self?.checkFormatChanged()
        }
        register(tapID, kAudioTapPropertyFormat) { [weak self] in
            self?.checkFormatChanged()
        }
    }

    private func register(
        _ objectID: AudioObjectID,
        _ selector: AudioObjectPropertySelector,
        _ handler: @escaping () -> Void
    ) {
        var address = propertyAddress(selector)
        // The block variant delivers on a queue we choose. The non-block
        // variant would call us on an internal HAL thread, where doing
        // anything non-trivial is asking for trouble.
        let block: AudioObjectPropertyListenerBlock = { _, _ in handler() }
        let status = AudioObjectAddPropertyListenerBlock(objectID, &address, queue, block)
        if status == noErr {
            listeners.append((objectID, address, block))
        } else {
            logErr("tap: failed to add listener for \(selector): OSStatus \(status)")
        }
    }

    private func removeListeners() {
        for (objectID, address, block) in listeners {
            var address = address
            AudioObjectRemovePropertyListenerBlock(objectID, &address, queue, block)
        }
        listeners.removeAll()
    }

    /// The gate. Re-reads the tap's format and does nothing at all unless it
    /// actually differs from the one the converter was built for — which is
    /// the overwhelmingly common case, since most device changes keep the
    /// same 48kHz mixdown.
    private func checkFormatChanged() {
        guard !stopped, !rebuildPending else { return }
        guard let current = readTapFormat() else {
            // Reading the format failed. That itself means the tap is no
            // longer usable, so rebuild rather than carry on blind.
            scheduleRebuild(reason: "tap_format_unreadable")
            return
        }
        guard let previous = sourceASBD else { return }
        if asbdEqual(previous, current) { return }
        scheduleRebuild(
            reason: "tap_format_changed"
                + " (\(previous.mSampleRate)Hz/\(previous.mChannelsPerFrame)ch"
                + " -> \(current.mSampleRate)Hz/\(current.mChannelsPerFrame)ch)"
        )
    }

    // MARK: Rebuild

    private func scheduleRebuild(reason: String) {
        guard !stopped, !rebuildPending else { return }
        rebuildPending = true
        // Remove listeners *before* the rebuild is queued: a single device
        // change can fire several notifications, and without this each would
        // queue its own rebuild.
        removeListeners()

        if Date().timeIntervalSince(windowStart) > stabilityWindow {
            rebuildsInWindow = 0
            windowStart = Date()
        }
        rebuildsInWindow += 1
        guard rebuildsInWindow <= maxRebuildsPerWindow else {
            rebuildPending = false
            let message = "system audio rebuild budget exhausted"
                + " (\(rebuildsInWindow) in \(Int(stabilityWindow))s, last: \(reason))"
            DispatchQueue.global().async { [weak self] in
                guard let self, !self.stopped else { return }
                self.onFatal(message)
            }
            return
        }

        emitEvent("system_audio_rebuild", ["reason": reason, "attempt": rebuildsInWindow])
        queue.async { [weak self] in
            guard let self, !self.stopped else { return }
            self.teardown()
            do {
                try self.setup()
                self.rebuildPending = false
            } catch {
                self.rebuildPending = false
                // Setup failed on the way back up. Try once more via the
                // normal path — the budget above is what stops this looping.
                self.scheduleRebuild(reason: "rebuild_setup_failed: \(error)")
            }
        }
    }

    /// Order matters: listeners first (so nothing fires against objects we
    /// are dismantling), then stop I/O, then destroy tap and aggregate.
    private func teardown() {
        probeTimer?.cancel()
        probeTimer = nil
        removeListeners()
        if let ioProcID {
            if aggregateID != AudioObjectID(kAudioObjectUnknown) {
                AudioDeviceStop(aggregateID, ioProcID)
                AudioDeviceDestroyIOProcID(aggregateID, ioProcID)
            }
            self.ioProcID = nil
        }
        if tapID != AudioObjectID(kAudioObjectUnknown) {
            AudioHardwareDestroyProcessTap(tapID)
            tapID = AudioObjectID(kAudioObjectUnknown)
        }
        if aggregateID != AudioObjectID(kAudioObjectUnknown) {
            AudioHardwareDestroyAggregateDevice(aggregateID)
            aggregateID = AudioObjectID(kAudioObjectUnknown)
        }
        sourceFormat = nil
        sourceASBD = nil
    }
}
