import Foundation

// MARK: - stdio JSON protocol
//
// Shared line-delimited JSON-over-stdio wire format, used by both
// `mnemos-audio` (per-recording capture) and `mnemos-meeting-watcher`
// (long-lived background detection) to talk to the Rust host. Same framing,
// same lock discipline — no reason for a second copy.

private let stdoutLock = NSLock()

public func emit(_ payload: [String: Any]) {
    guard let data = try? JSONSerialization.data(withJSONObject: payload) else { return }
    stdoutLock.lock()
    defer { stdoutLock.unlock() }
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write("\n".data(using: .utf8)!)
}

public func logErr(_ message: String) {
    FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
}

public func emitEvent(_ kind: String, _ extra: [String: Any] = [:]) {
    var payload: [String: Any] = ["event": kind]
    for (k, v) in extra { payload[k] = v }
    emit(payload)
}

public func emitErrorEvent(kind: String, message: String) {
    emitEvent("error", ["kind": kind, "message": message])
}

public func nowMs() -> Int64 {
    Int64(Date().timeIntervalSince1970 * 1000)
}
