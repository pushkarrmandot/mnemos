// mnemos-audio — per-recording-session macOS capture sidecar.
//
// Spawned by Rust (LLD-02 §8) for the lifetime of one recording. Captures
// mic (AVAudioEngine) and system audio (ScreenCaptureKit) into two 16kHz
// mono 16-bit PCM WAV files, flushing every 500ms so a crash loses at most
// half a second per source (LLD-03 §4.1). Talks line-delimited JSON over
// stdio (BACKEND §3, LLD-03 §3.3) — no bidirectional streaming, ~9 message
// kinds, so Content-Length framing (used by the Python worker) would be
// overkill here.
//
// Protocol version bumps only on a required-param change or a renamed
// method (BACKEND §4 "Versioning of IPC contracts").

import AVFoundation
import CoreGraphics
import Foundation
import MnemosAudioKit
import ScreenCaptureKit

let sidecarVersion = "1.0.0"
let protocolVersion: UInt32 = 1
let sampleRate: Double = 16_000
let flushIntervalMs: UInt64 = 500
let levelIntervalMs: UInt64 = 100
let noSignalWarningSeconds: Double = 30.0
let noSignalDbThreshold: Float = -60.0

// MARK: - stdout / stderr
//
// emit/logErr/emitEvent/emitErrorEvent/nowMs now live in MnemosAudioKit
// (shared with mnemos-meeting-watcher).

// MARK: - WAV chunk writer (mono 16-bit PCM @ 16kHz)

/// Accumulates resampled PCM in memory and flushes to disk on a fixed
/// cadence (LLD-03 §4.1 — "500ms is picked over 1000ms because the
/// live-transcription poll cadence is 5s"). A dedicated serial queue per
/// writer keeps mic/system writes from blocking each other.
final class ChunkedWavWriter {
    private let queue: DispatchQueue
    private let path: String
    private var fileHandle: FileHandle?
    private var pending = Data()
    private(set) var bytesWritten: UInt64 = 0
    private var headerWritten = false

    init(path: String, label: String) {
        self.path = path
        self.queue = DispatchQueue(label: "mnemos.audio.writer.\(label)")
    }

    /// Opens the file and writes a placeholder WAV header (patched with the
    /// real data size on `close()`).
    ///
    /// Uses the POSIX `open()` syscall directly rather than
    /// `FileHandle(forWritingAtPath:)`: the Foundation initializer just
    /// returns `nil` on any failure and discards `errno`, which made every
    /// open failure here indistinguishable from genuine disk exhaustion.
    /// `errno` is what lets `start()`'s catch tell "actually out of space"
    /// apart from "permissions/missing directory/too many open files/etc.".
    func open() throws {
        FileManager.default.createFile(atPath: path, contents: nil)
        let fd = Foundation.open(path, O_WRONLY)
        guard fd >= 0 else {
            let code = errno
            let reason = String(cString: strerror(code))
            if code == ENOSPC {
                throw CaptureError.diskFull("no space left on device: \(path) (\(reason))")
            }
            throw CaptureError.disk("could not open \(path) for writing: \(reason) (errno \(code))")
        }
        let handle = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        fileHandle = handle
        handle.write(wavHeader(dataBytes: 0))
        headerWritten = true
    }

    func append(_ samples: Data) {
        queue.sync {
            pending.append(samples)
        }
    }

    /// Writes and fsyncs whatever has accumulated since the last flush, then
    /// patches the RIFF/data chunk sizes in place so the file is a valid,
    /// playable WAV even if the process dies before a graceful `close()` —
    /// a `wave`/AVFoundation reader trusts the header's declared size, not
    /// just the bytes physically present. Returns bytes flushed this call
    /// (0 if nothing pending).
    @discardableResult
    func flush() -> Int {
        queue.sync {
            guard !pending.isEmpty, let handle = fileHandle else { return 0 }
            let chunk = pending
            pending.removeAll(keepingCapacity: true)
            handle.seekToEndOfFile()
            handle.write(chunk)
            bytesWritten += UInt64(chunk.count)
            patchHeader(handle)
            do {
                try handle.synchronize()
            } catch {
                logErr("fsync failed for \(path): \(error)")
            }
            return chunk.count
        }
    }

    /// Rewrites the two size fields in the already-written header, leaving
    /// the write cursor at end-of-file for the next append.
    private func patchHeader(_ handle: FileHandle) {
        let dataBytes = UInt32(truncatingIfNeeded: bytesWritten)
        handle.seek(toFileOffset: 4)
        handle.write(uint32LE(36 + dataBytes))
        handle.seek(toFileOffset: 40)
        handle.write(uint32LE(dataBytes))
        handle.seekToEndOfFile()
    }

    func close() {
        flush()
        queue.sync {
            guard let handle = fileHandle else { return }
            try? handle.close()
            fileHandle = nil
        }
    }

    private func wavHeader(dataBytes: UInt32) -> Data {
        let byteRate: UInt32 = UInt32(sampleRate) * 2 // mono * 16-bit
        var header = Data()
        header.append(contentsOf: Array("RIFF".utf8))
        header.append(uint32LE(36 + dataBytes))
        header.append(contentsOf: Array("WAVE".utf8))
        header.append(contentsOf: Array("fmt ".utf8))
        header.append(uint32LE(16)) // PCM fmt chunk size
        header.append(uint16LE(1)) // PCM
        header.append(uint16LE(1)) // mono
        header.append(uint32LE(UInt32(sampleRate)))
        header.append(uint32LE(byteRate))
        header.append(uint16LE(2)) // block align
        header.append(uint16LE(16)) // bits per sample
        header.append(contentsOf: Array("data".utf8))
        header.append(uint32LE(dataBytes))
        return header
    }
}

func uint32LE(_ v: UInt32) -> Data {
    var le = v.littleEndian
    return Data(bytes: &le, count: 4)
}

func uint16LE(_ v: UInt16) -> Data {
    var le = v.littleEndian
    return Data(bytes: &le, count: 2)
}

enum CaptureError: Error {
    case permissionDenied(String)
    case micDisconnected(String)
    /// Any other failure to open/write the capture file — permissions,
    /// missing directory, too many open files, etc. Distinct from
    /// `diskFull`, which is reserved for a real `ENOSPC`.
    case disk(String)
    case diskFull(String)
    case deviceLost(String)
    case encoder(String)
}

// MARK: - Resampling helper

/// Converts an arbitrary-format `AVAudioPCMBuffer` to 16kHz mono Int16 PCM
/// bytes via `AVAudioConverter`. One converter per source (mic vs. system)
/// since source formats can differ.
final class Resampler {
    private var converter: AVAudioConverter?
    private let outFormat: AVAudioFormat

    init() {
        outFormat = AVAudioFormat(
            commonFormat: .pcmFormatInt16,
            sampleRate: sampleRate,
            channels: 1,
            interleaved: true
        )!
    }

    func convert(_ buffer: AVAudioPCMBuffer) -> Data? {
        if converter == nil || converter?.inputFormat != buffer.format {
            converter = AVAudioConverter(from: buffer.format, to: outFormat)
        }
        guard let converter else { return nil }

        let ratio = outFormat.sampleRate / buffer.format.sampleRate
        let outCapacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 16
        guard let outBuffer = AVAudioPCMBuffer(pcmFormat: outFormat, frameCapacity: outCapacity) else {
            return nil
        }

        var error: NSError?
        var consumed = false
        let status = converter.convert(to: outBuffer, error: &error) { _, outStatus in
            if consumed {
                outStatus.pointee = .noDataNow
                return nil
            }
            consumed = true
            outStatus.pointee = .haveData
            return buffer
        }

        if status == .error {
            logErr("resample error: \(error?.localizedDescription ?? "unknown")")
            return nil
        }
        guard let channelData = outBuffer.int16ChannelData else { return nil }
        let frameCount = Int(outBuffer.frameLength)
        return Data(bytes: channelData[0], count: frameCount * 2)
    }
}

/// RMS-in-dBFS over a buffer of Int16 samples, for level metering.
func rmsDb(_ data: Data) -> Float {
    guard data.count >= 2 else { return -96.0 }
    let sampleCount = data.count / 2
    var sumSquares: Double = 0
    data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) in
        let samples = raw.bindMemory(to: Int16.self)
        for s in samples {
            let normalized = Double(s) / Double(Int16.max)
            sumSquares += normalized * normalized
        }
    }
    let rms = (sumSquares / Double(sampleCount)).squareRoot()
    if rms <= 0 { return -96.0 }
    return Float(20.0 * log10(rms))
}

// MARK: - Session

/// Drives one recording session end to end: opens both writers, wires the
/// mic tap + ScreenCaptureKit stream, runs the flush/level timers, and
/// reports state transitions via `emitEvent`.
final class Session: NSObject, SCStreamOutput, SCStreamDelegate {
    let conversationId: String
    let micWriter: ChunkedWavWriter
    let systemWriter: ChunkedWavWriter
    let micDeviceId: String?

    private let engine = AVAudioEngine()
    private let micResampler = Resampler()
    private let systemResampler = Resampler()
    private var scStream: SCStream?
    /// Non-nil when system audio is coming from a Core Audio process tap
    /// rather than ScreenCaptureKit. Mutually exclusive with `scStream`.
    private var systemSource: SystemAudioSource?

    private var paused = false
    private let stateQueue = DispatchQueue(label: "mnemos.audio.session.state")

    private var micLastLevel: Float = -96.0
    private var systemLastLevel: Float = -96.0
    private var micSilenceStart: Date?
    private var systemSilenceStart: Date?
    private var micWarned = false
    private var systemWarned = false

    private var levelTimer: DispatchSourceTimer?
    private var flushTimer: DispatchSourceTimer?
    private var permissionTimer: DispatchSourceTimer?
    private var lastMicAuthorized = true

    init(conversationId: String, micPath: String, systemPath: String, micDeviceId: String?) {
        self.conversationId = conversationId
        self.micWriter = ChunkedWavWriter(path: micPath, label: "mic")
        self.systemWriter = ChunkedWavWriter(path: systemPath, label: "system")
        self.micDeviceId = micDeviceId
    }

    func start() {
        do {
            try micWriter.open()
            try systemWriter.open()
        } catch {
            // Classify by the real cause instead of a blanket "disk_full" —
            // `ChunkedWavWriter.open()` only throws `.diskFull` for a real
            // `ENOSPC`; everything else (permissions, missing directory, fd
            // limit, ...) is `.disk` and reported as the generic
            // `capture_failed` kind, carrying the real `errno`/`strerror` in
            // its message instead of a misleading label.
            let kind: String
            if case CaptureError.diskFull = error {
                kind = "disk_full"
            } else {
                kind = "capture_failed"
            }
            emitErrorEvent(kind: kind, message: "\(error)")
            exit(2)
        }

        setupMicTap()
        setupSystemCapture()
        startTimers()

        emitEvent("started", ["started_at_ms": nowMs()])
    }

    private func setupMicTap() {
        let input = engine.inputNode

        // W17c — speaker bleed. Without this, the mic tap receives the RAW
        // hardware input, which on speakers contains a delayed (~200-300ms,
        // measured) copy of whatever ScreenCaptureKit is simultaneously
        // recording into system.wav. Parakeet then transcribes the same
        // utterance twice and `merge_transcripts` labels the bleed copy
        // "You", since v1 attributes speakers purely by source file.
        //
        // Voice Processing I/O is the same AEC unit FaceTime/Zoom use, and
        // Apple documents the cancellation as applying to an installed input
        // tap — so this filters the bleed at the source rather than trying to
        // undo it in the transcript. Deliberately NOT a hand-rolled adaptive
        // filter: double-talk divergence and mic/system clock drift are
        // exactly what a tuned OS implementation already handles.
        //
        // Best-effort by design. It is documented to fail when input and
        // output belong to different devices (AirPods mic + built-in
        // speakers is the common case), and a thrown error there must not
        // take the recording down — bleed is a quality problem, a failed
        // start is a lost meeting. Falls back to the raw tap, which is
        // exactly the pre-W17c behavior.
        // DISABLED — see the W17c rationale above for what this cost us.
        //
        // `setVoiceProcessingEnabled(true)` does not throw here; it reports
        // success and then delivers pure digital silence. Every mic sample
        // comes back as exactly 0 (level meter pinned at -96 dB, the 16-bit
        // floor — a live mic never reads that; even a quiet room floors
        // around -55 dB), so recordings looked healthy end to end: WAV files
        // of the right duration, bytes streaming to the transcriber, no
        // error anywhere — and zero transcript segments, because there was
        // no audio in them.
        //
        // Measured on a real packaged build, same binary, same path, only
        // this line differing:
        //     VPIO enabled  -> mic_db = -96, -96, -96 …
        //     VPIO disabled -> mic_db = -56, -52, -61 … (room noise)
        // Also tried enabling it on `engine.outputNode` as well, on the
        // theory that VPIO is one I/O unit and the render half was
        // unconfigured: still silent.
        //
        // The cause is context, not configuration. VPIO expects a real app
        // audio session; this sidecar is a headless stdio process with no
        // NSApplication and no render chain, so the unit has nowhere to run
        // its output half and yields nothing on input. That it works under
        // `tauri dev` is incidental — the same silence reproduces from the
        // dev-tree binary too.
        //
        // Consequence, deliberately accepted: speaker bleed is back — on
        // speakers the mic re-records system audio and Parakeet transcribes
        // some utterances twice, the second attributed to "You". That is a
        // transcript-quality bug. Silent recordings are a product that does
        // not work. If AEC returns, it must be verified to actually pass
        // audio (start the engine, confirm a non-zero sample arrives, and
        // fall back to the raw tap otherwise) rather than trusted because
        // the call didn't throw.
        _ = input

        // Read AFTER enabling voice processing: the unit can renegotiate the
        // node's format, and a format captured beforehand would no longer
        // describe what the tap actually delivers.
        let format = input.outputFormat(forBus: 0)

        // PROVISIONAL — LLD-03 §4.1: the exact device-selection API is the
        // one the compiler accepts on macOS 14+. Falling back to the
        // default input and reporting `input_switched` is the documented
        // degrade path when a specific `mic_device_id` can't be honored.
        if micDeviceId != nil {
            emitEvent("warning", ["kind": "input_switched", "message": "explicit input device selection not wired in v1; using default input"])
        }

        installMicTap(format: format)

        do {
            try engine.start()
        } catch {
            emitErrorEvent(kind: "mic_disconnected", message: "engine start failed: \(error)")
            finish(mic: micWriter.bytesWritten, system: systemWriter.bytesWritten)
            exit(3)
        }

        NotificationCenter.default.addObserver(
            forName: .AVAudioEngineConfigurationChange,
            object: engine,
            queue: nil
        ) { [weak self] _ in
            self?.handleEngineConfigurationChange()
        }
    }

    private func installMicTap(format: AVAudioFormat) {
        engine.inputNode.installTap(onBus: 0, bufferSize: 1600, format: format) { [weak self] buffer, _ in
            guard let self else { return }
            guard !self.isPaused() else { return }
            guard let pcm = self.micResampler.convert(buffer) else { return }
            self.micWriter.append(pcm)
            self.trackLevel(pcm, isMic: true)
        }
    }

    /// `AVAudioEngineConfigurationChange` fires on *any* input/output route
    /// change — plugging in headphones, unplugging them, a Bluetooth device
    /// connecting — not just a device disappearing outright. Apple's own
    /// docs for this notification describe it as recoverable: stop, adapt to
    /// the new hardware, restart — not a reason to give up.
    ///
    /// This used to treat every firing as fatal, ending the whole recording
    /// (mic *and* system audio) the moment a user plugged in or removed
    /// headphones mid-meeting — a very ordinary action turning into a lost
    /// recording. The engine has already auto-stopped itself by the time
    /// this notification arrives; the installed tap's format now describes
    /// hardware that may no longer be routed, so it has to be replaced with
    /// one built from the *current* input format, not just restarted as-is.
    ///
    /// Only a genuine recovery failure (e.g. no input device exists at all
    /// right now) falls through to `emitDeviceLost` — the fatal path this
    /// entire method used to be.
    private func handleEngineConfigurationChange() {
        stateQueue.async { [weak self] in
            guard let self else { return }
            self.engine.inputNode.removeTap(onBus: 0)
            let newFormat = self.engine.inputNode.outputFormat(forBus: 0)
            self.installMicTap(format: newFormat)
            do {
                try self.engine.start()
                emitEvent("warning", [
                    "kind": "input_route_changed",
                    "message": "microphone route changed; capture resumed automatically",
                ])
            } catch {
                self.emitDeviceLost(
                    "audio engine configuration changed, and recovery failed: \(error)"
                )
            }
        }
    }

    private func emitDeviceLost(_ message: String) {
        emitErrorEvent(kind: "device_lost", message: message)
    }

    /// Chooses a system-audio backend. The Core Audio process tap is
    /// strongly preferred where it exists (macOS 14.2+): ScreenCaptureKit
    /// routes through `replayd`, a daemon shared with every other capturing
    /// app, which tore down real recordings mid-meeting with no error we
    /// could see or prevent.
    ///
    /// Any failure to bring the tap up — old OS, permission not granted,
    /// anything unexpected — falls through to ScreenCaptureKit rather than
    /// failing the recording. The fallback is the code that shipped before
    /// this change, unmodified.
    private func setupSystemCapture() {
        if #available(macOS 14.2, *) {
            let tap = ProcessTapSource(
                onPCM: { [weak self] data in
                    guard let self, !self.isPaused() else { return }
                    self.systemWriter.append(data)
                    self.trackLevel(data, isMic: false)
                },
                onFatal: { [weak self] message in
                    self?.emitDeviceLost("system audio tap failed: \(message)")
                },
                onSilentStartup: { [weak self] message in
                    // The tap came up but produced pure silence — almost
                    // always a missing "System Audio Recording" grant, which
                    // Core Audio reports by handing back zeros rather than an
                    // error. Abandon it and use ScreenCaptureKit, which asks
                    // for a permission this user has already granted.
                    guard let self else { return }
                    logErr("system audio tap produced no audio (\(message)); falling back to ScreenCaptureKit")
                    self.systemSource?.stop()
                    self.systemSource = nil
                    emitEvent("system_audio_backend", [
                        "backend": "screen_capture_kit",
                        "fell_back_from": "core_audio_tap",
                        "reason": message,
                    ])
                    self.setupScreenCaptureKitCapture()
                }
            )
            do {
                try tap.start()
                systemSource = tap
                emitEvent("system_audio_backend", ["backend": tap.backendName])
                return
            } catch {
                logErr("system audio tap unavailable, falling back to ScreenCaptureKit: \(error)")
            }
        }
        emitEvent("system_audio_backend", ["backend": "screen_capture_kit"])
        setupScreenCaptureKitCapture()
    }

    private func setupScreenCaptureKitCapture() {
        Task {
            do {
                let content = try await SCShareableContent.excludingDesktopWindows(
                    false, onScreenWindowsOnly: false
                )
                guard let display = content.displays.first else {
                    emitErrorEvent(kind: "device_lost", message: "no shareable display for system audio")
                    return
                }
                let filter = SCContentFilter(display: display, excludingWindows: [])
                let config = SCStreamConfiguration()
                config.capturesAudio = true
                config.sampleRate = Int(sampleRate)
                config.channelCount = 1
                // We want audio only, but SCStream has no way to turn video
                // off — a default-constructed SCStreamConfiguration captures
                // the full display at ~60fps. Left at the defaults, macOS
                // encodes every one of those frames and ships it to us over
                // XPC, and because only an `.audio` output was registered
                // each one lands nowhere:
                //
                //   _SCStream_RemoteVideoQueueOperationHandlerWithError:
                //   stream output NOT found. Dropping frame
                //
                // ~60 of those per second for the length of the recording,
                // after which `replayd` tears the whole stream down via
                // `-[RPClient stopAllStreamsWithError:]` — delivered to us as
                // `didStopWithError(nil)`, which read as an unexplained
                // "recording ended unexpectedly" mid-meeting. How long it
                // survives varies (17s in one run, 22 minutes in another),
                // which is what made it look intermittent and unrelated.
                //
                // So: shrink the video to the smallest frame the API accepts
                // and slow it to 1fps, and give it a real destination below.
                // The frames still exist — that is unavoidable here — but
                // they cost almost nothing and no longer error.
                config.width = 2
                config.height = 2
                config.minimumFrameInterval = CMTime(value: 1, timescale: 1)
                config.queueDepth = 3

                let stream = SCStream(filter: filter, configuration: config, delegate: self)
                try stream.addStreamOutput(self, type: .audio, sampleHandlerQueue: DispatchQueue(label: "mnemos.audio.sck"))
                // The destination for those unwanted frames. `didOutputSampleBuffer`
                // discards anything that isn't `.audio`; the point is purely that
                // an output *exists*, so ScreenCaptureKit stops erroring per frame.
                try stream.addStreamOutput(self, type: .screen, sampleHandlerQueue: DispatchQueue(label: "mnemos.audio.sck.video"))
                try await stream.startCapture()
                self.scStream = stream
            } catch {
                emitErrorEvent(kind: "permission_denied", message: "ScreenCaptureKit: \(error)")
                finish(mic: micWriter.bytesWritten, system: systemWriter.bytesWritten)
                exit(2)
            }
        }
    }

    // SCStreamOutput
    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .audio, !isPaused() else { return }
        guard let pcm = pcmBuffer(from: sampleBuffer) else { return }
        guard let data = systemResampler.convert(pcm) else { return }
        systemWriter.append(data)
        trackLevel(data, isMic: false)
    }

    // SCStreamDelegate
    //
    // `error` is declared non-optional because that's how the Objective-C
    // header imports, but ScreenCaptureKit delivers this callback over XPC
    // from ReplayKit and does pass a nil NSError on some stop paths. A nil
    // arriving in a non-optional slot is a null existential, so *any* use of
    // it — `\(error)` included — dereferences null. That crashed the whole
    // sidecar with SIGSEGV mid-recording (crash report: EXC_BAD_ACCESS,
    // KERN_INVALID_ADDRESS at 0x0, faulting frame
    // `Session.stream(_:didStopWithError:)` inside
    // `DefaultStringInterpolation.appendInterpolation`), which took the mic
    // capture down with it and lost the rest of the meeting — for what is
    // only ever an advisory about the *system* audio half.
    //
    // Accepting it as optional is what makes the nil representable instead
    // of undefined behaviour; the ObjC selector is unchanged, so this still
    // satisfies the delegate.
    // Selector pinned explicitly: making the parameter optional means this
    // no longer matches `SCStreamDelegate`'s Swift-level requirement, so
    // Swift would otherwise be free to not expose it to the ObjC runtime —
    // and ScreenCaptureKit dispatches this purely by selector. Without the
    // annotation a compiler change could silently stop the callback firing.
    @objc(stream:didStopWithError:)
    func stream(_ stream: SCStream, didStopWithError error: (any Error)?) {
        let detail = error.map { "\($0)" } ?? "no error reported"
        emitErrorEvent(kind: "device_lost", message: "system audio stream stopped: \(detail)")
    }

    private func pcmBuffer(from sampleBuffer: CMSampleBuffer) -> AVAudioPCMBuffer? {
        guard let formatDesc = CMSampleBufferGetFormatDescription(sampleBuffer),
              let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(formatDesc)
        else { return nil }
        guard let format = AVAudioFormat(streamDescription: asbd) else { return nil }
        let frameCount = AVAudioFrameCount(CMSampleBufferGetNumSamples(sampleBuffer))
        guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frameCount) else { return nil }
        buffer.frameLength = frameCount

        guard let blockBuffer = CMSampleBufferGetDataBuffer(sampleBuffer) else { return nil }
        var lengthAtOffset = 0
        var totalLength = 0
        var dataPointer: UnsafeMutablePointer<Int8>?
        CMBlockBufferGetDataPointer(
            blockBuffer, atOffset: 0, lengthAtOffsetOut: &lengthAtOffset,
            totalLengthOut: &totalLength, dataPointerOut: &dataPointer
        )
        guard let dataPointer, let channelData = buffer.floatChannelData else { return nil }
        dataPointer.withMemoryRebound(to: Float.self, capacity: totalLength / 4) { floatPtr in
            channelData[0].update(from: floatPtr, count: Int(frameCount))
        }
        return buffer
    }

    private func isPaused() -> Bool {
        stateQueue.sync { paused }
    }

    private func trackLevel(_ data: Data, isMic: Bool) {
        let db = rmsDb(data)
        stateQueue.async { [weak self] in
            guard let self else { return }
            if isMic {
                self.micLastLevel = db
                self.trackSilence(db: db, start: &self.micSilenceStart, warned: &self.micWarned, kind: "no_mic_signal")
            } else {
                self.systemLastLevel = db
                self.trackSilence(db: db, start: &self.systemSilenceStart, warned: &self.systemWarned, kind: "system_source_empty")
            }
        }
    }

    private func trackSilence(db: Float, start: inout Date?, warned: inout Bool, kind: String) {
        if db < noSignalDbThreshold {
            if start == nil { start = Date() }
            if !warned, let s = start, Date().timeIntervalSince(s) >= noSignalWarningSeconds {
                warned = true
                emitEvent("warning", ["kind": kind, "message": "no signal for \(Int(noSignalWarningSeconds))s"])
            }
        } else {
            start = nil
            warned = false
        }
    }

    private func startTimers() {
        let levels = DispatchSource.makeTimerSource(queue: .global())
        levels.schedule(deadline: .now() + .milliseconds(Int(levelIntervalMs)), repeating: .milliseconds(Int(levelIntervalMs)))
        levels.setEventHandler { [weak self] in
            guard let self else { return }
            let (mic, sys) = self.stateQueue.sync { (self.micLastLevel, self.systemLastLevel) }
            emitEvent("level", ["mic_db": mic, "system_db": sys])
        }
        levels.resume()
        levelTimer = levels

        let flush = DispatchSource.makeTimerSource(queue: .global())
        flush.schedule(deadline: .now() + .milliseconds(Int(flushIntervalMs)), repeating: .milliseconds(Int(flushIntervalMs)))
        flush.setEventHandler { [weak self] in
            guard let self else { return }
            let micBytes = self.micWriter.flush()
            if micBytes > 0 {
                emitEvent("chunk", ["source": "mic", "bytes_written": self.micWriter.bytesWritten])
            }
            let sysBytes = self.systemWriter.flush()
            if sysBytes > 0 {
                emitEvent("chunk", ["source": "system", "bytes_written": self.systemWriter.bytesWritten])
            }
        }
        flush.resume()
        flushTimer = flush

        // W17b (12_CORNER_CASES.md "Permissions" §If user revokes AFTER
        // onboarding — Microphone row): macOS has no push notification for a
        // TCC grant being revoked mid-process the way iOS's
        // `AVAudioSession.interruptionNotification` does for an audio
        // session interruption — `AVAudioEngineConfigurationChange` (already
        // handled above, `emitDeviceLost`) covers hardware/route changes,
        // not a permission being pulled in System Settings while the tap
        // stays technically installed. Polling `authorizationStatus` is the
        // documented way to detect this on macOS; every 2s is frequent
        // enough to feel immediate without meaningfully adding to idle CPU.
        let permission = DispatchSource.makeTimerSource(queue: .global())
        permission.schedule(deadline: .now() + .seconds(2), repeating: .seconds(2))
        permission.setEventHandler { [weak self] in
            guard let self else { return }
            let authorized = AVCaptureDevice.authorizationStatus(for: .audio) == .authorized
            if authorized != self.lastMicAuthorized {
                self.lastMicAuthorized = authorized
                if !authorized {
                    emitErrorEvent(kind: "mic_permission_revoked", message: "Microphone access was revoked mid-recording")
                }
            }
        }
        permission.resume()
        permissionTimer = permission
    }

    func pause() {
        stateQueue.sync { paused = true }
        emitEvent("paused")
    }

    func resume() {
        stateQueue.sync { paused = false }
        emitEvent("resumed")
    }

    func stop() {
        levelTimer?.cancel()
        flushTimer?.cancel()
        permissionTimer?.cancel()
        engine.stop()
        engine.inputNode.removeTap(onBus: 0)
        systemSource?.stop()
        systemSource = nil
        if let scStream {
            let sema = DispatchSemaphore(value: 0)
            scStream.stopCapture { _ in sema.signal() }
            _ = sema.wait(timeout: .now() + 2)
        }
        micWriter.close()
        systemWriter.close()
        finish(mic: micWriter.bytesWritten, system: systemWriter.bytesWritten)
    }

    private func finish(mic: UInt64, system: UInt64) {
        emitEvent("stopped", ["mic_bytes": mic, "system_bytes": system])
    }
}

// MARK: - Command loop

var session: Session?

func handleCommand(_ line: String) {
    guard let data = line.data(using: .utf8),
          let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let method = obj["method"] as? String
    else {
        logErr("malformed command: \(line)")
        return
    }

    switch method {
    case "handshake":
        emit(["event": "ready", "sidecar_version": sidecarVersion, "protocol_version": protocolVersion])
    case "start":
        guard let conversationId = obj["conversation_id"] as? String,
              let micPath = obj["mic_path"] as? String,
              let systemPath = obj["system_path"] as? String
        else {
            emitErrorEvent(kind: "encoder_error", message: "start missing required params")
            return
        }
        let deviceId = obj["mic_device_id"] as? String
        let s = Session(conversationId: conversationId, micPath: micPath, systemPath: systemPath, micDeviceId: deviceId)
        session = s
        s.start()
    case "pause":
        session?.pause()
    case "resume":
        session?.resume()
    case "stop":
        session?.stop()
        exit(0)
    default:
        logErr("unknown method: \(method)")
    }
}

// MARK: - Onboarding permission preflight (W15)
//
// The persistent per-recording protocol above still preflights lazily at
// `start` time (unchanged — see the comment that used to sit here, now
// below this block). This section adds three *one-shot* argv subcommands —
// `check-permissions` / `request-mic-permission` / `request-screen-permission`
// — so onboarding can ask "are we already allowed" and "please prompt for
// this" *before* a recording ever starts, without spinning up a full
// capture session to find out. Rust spawns this binary with one of these
// as `argv[1]`, reads exactly one JSON line from stdout, and the process
// exits — no stdin protocol, no persistent session.

func permissionStatusString(_ status: AVAuthorizationStatus) -> String {
    switch status {
    case .authorized: return "granted"
    case .denied, .restricted: return "denied"
    case .notDetermined: return "undetermined"
    @unknown default: return "undetermined"
    }
}

func micStatus() -> String {
    permissionStatusString(AVCaptureDevice.authorizationStatus(for: .audio))
}

/// `CGPreflightScreenCaptureAccess()` reads the TCC grant without prompting
/// — true/false only, no "undetermined" (macOS collapses "never asked" and
/// "denied" into the same `false` here; the request path below is what
/// actually distinguishes them, by whether calling it changes the result).
func screenStatus() -> String {
    CGPreflightScreenCaptureAccess() ? "granted" : "undetermined_or_denied"
}

func runCheckPermissions() -> Never {
    emit(["mic": micStatus(), "screen": screenStatus()])
    exit(0)
}

func runRequestMicPermission() -> Never {
    let sema = DispatchSemaphore(value: 0)
    var granted = false
    AVCaptureDevice.requestAccess(for: .audio) { ok in
        granted = ok
        sema.signal()
    }
    sema.wait()
    emit(["mic": granted ? "granted" : "denied"])
    exit(0)
}

/// Unlike mic, Screen Recording has no synchronous request API. Despite the
/// name, `CGRequestScreenCaptureAccess()` does **not** block waiting for the
/// user and does not itself prompt — `tccd` refuses to prompt for this
/// service at all ("Service kTCCServiceScreenCapture does not allow
/// prompting; returning denied", observed directly in the unified log). On a
/// never-asked grant it registers the app with TCC, returns `false`
/// immediately, and macOS posts its consent alert asynchronously; the grant
/// then only takes effect after the app restarts, because the authorization
/// is cached per-process at launch.
///
/// So `false` here is genuinely ambiguous — never-asked and actually-denied
/// are indistinguishable at this point (`CGPreflightScreenCaptureAccess`
/// collapses them too). Reporting it as `denied` would mislabel the entirely
/// normal first-run path as a hard refusal, so emit `pending` instead and let
/// the caller route to "enable in System Settings, then relaunch" — which is
/// the only real path forward for both cases anyway.
func runRequestScreenPermission() -> Never {
    let granted = CGRequestScreenCaptureAccess()
    emit(["screen": granted ? "granted" : "pending"])
    exit(0)
}

let argv = CommandLine.arguments
if argv.count > 1 {
    switch argv[1] {
    case "check-permissions": runCheckPermissions()
    case "request-mic-permission": runRequestMicPermission()
    case "request-screen-permission": runRequestScreenPermission()
    default:
        logErr("unknown subcommand: \(argv[1])")
        exit(1)
    }
}

// Permission preflight happens lazily at `start` time (ScreenCaptureKit's
// own async check inside `setupSystemCapture`, and AVAudioEngine surfacing
// `mic_disconnected` if the OS blocks the tap) rather than at boot — the OS
// permission prompt is asynchronous and blocking boot on it would make
// every recording pay a first-launch tax (LLD-03 §4.1).

let stdinHandle = FileHandle.standardInput
var buffer = Data()

DispatchQueue.global(qos: .userInteractive).async {
    while true {
        let chunk = stdinHandle.availableData
        if chunk.isEmpty {
            // EOF — Rust closed the pipe (or died). Nothing to clean up
            // beyond what `stop()`/process exit already does.
            exit(0)
        }
        buffer.append(chunk)
        while let newlineRange = buffer.range(of: Data([0x0A])) {
            let lineData = buffer.subdata(in: buffer.startIndex..<newlineRange.lowerBound)
            buffer.removeSubrange(buffer.startIndex..<newlineRange.upperBound)
            if let line = String(data: lineData, encoding: .utf8), !line.isEmpty {
                handleCommand(line)
            }
        }
    }
}

RunLoop.main.run()
