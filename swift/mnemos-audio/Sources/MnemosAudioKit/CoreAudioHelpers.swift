import CoreAudio
import Foundation

// MARK: - Core Audio helpers
//
// Shared between the recording sidecar (`mnemos-audio`, `ProcessTapSource`)
// and the background meeting watcher (`mnemos-meeting-watcher`) — both read
// Core Audio process/device properties, and duplicating this plumbing would
// only let the two drift. Nothing here is tap-specific; `ProcessTapSource`
// itself stays in the `mnemos-audio` target.

public func propertyAddress(
    _ selector: AudioObjectPropertySelector
) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress(
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain
    )
}

/// Reads a fixed-size property into a `T`. Returns nil on any failure rather
/// than throwing — every call site here treats "couldn't read it" the same
/// way, and an `OSStatus` is not information a caller can act on.
public func readProperty<T>(
    _ objectID: AudioObjectID,
    _ selector: AudioObjectPropertySelector,
    _ initial: T
) -> T? {
    var address = propertyAddress(selector)
    var value = initial
    var size = UInt32(MemoryLayout<T>.size)
    let status = withUnsafeMutablePointer(to: &value) { ptr -> OSStatus in
        AudioObjectGetPropertyData(objectID, &address, 0, nil, &size, ptr)
    }
    return status == noErr ? value : nil
}

public func readStringProperty(
    _ objectID: AudioObjectID,
    _ selector: AudioObjectPropertySelector
) -> String? {
    guard let cf: CFString = readProperty(objectID, selector, nil as CFString?) ?? nil else {
        return nil
    }
    return cf as String
}

/// `AudioObjectID` for our own process, so a global tap can exclude us and
/// never record audio this app itself plays back.
///
/// Best-effort: if the translation fails we tap everything, which is a
/// slightly worse recording (we might capture our own playback) but not a
/// broken one — so this must not be fatal.
public func currentProcessAudioObjectID() -> AudioObjectID? {
    var pid = getpid()
    var address = propertyAddress(kAudioHardwarePropertyTranslatePIDToProcessObject)
    var objectID = AudioObjectID(kAudioObjectUnknown)
    var size = UInt32(MemoryLayout<AudioObjectID>.size)
    let status = AudioObjectGetPropertyData(
        AudioObjectID(kAudioObjectSystemObject),
        &address,
        UInt32(MemoryLayout<pid_t>.size),
        &pid,
        &size,
        &objectID
    )
    guard status == noErr, objectID != AudioObjectID(kAudioObjectUnknown) else { return nil }
    return objectID
}

/// All process objects Core Audio currently knows about (one per process
/// that has ever touched audio, not just ones that are active right now).
public func allAudioProcessObjects() -> [AudioObjectID] {
    var address = propertyAddress(kAudioHardwarePropertyProcessObjectList)
    var dataSize: UInt32 = 0
    guard AudioObjectGetPropertyDataSize(
        AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &dataSize
    ) == noErr, dataSize > 0 else { return [] }

    let count = Int(dataSize) / MemoryLayout<AudioObjectID>.size
    var processes = [AudioObjectID](repeating: AudioObjectID(kAudioObjectUnknown), count: count)
    guard AudioObjectGetPropertyData(
        AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &dataSize, &processes
    ) == noErr else { return [] }
    return processes
}

public func asbdEqual(
    _ a: AudioStreamBasicDescription,
    _ b: AudioStreamBasicDescription
) -> Bool {
    a.mSampleRate == b.mSampleRate
        && a.mFormatID == b.mFormatID
        && a.mFormatFlags == b.mFormatFlags
        && a.mBytesPerPacket == b.mBytesPerPacket
        && a.mFramesPerPacket == b.mFramesPerPacket
        && a.mBytesPerFrame == b.mBytesPerFrame
        && a.mChannelsPerFrame == b.mChannelsPerFrame
        && a.mBitsPerChannel == b.mBitsPerChannel
}
