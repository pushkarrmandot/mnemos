//! Small cross-platform `std::process::Command` helpers shared by every
//! child-process spawn site (Python worker, `claude` CLI, settings deep
//! link). Kept here instead of duplicated per call site — see Windows
//! parity audit finding #19.

/// `CREATE_NO_WINDOW` (`0x08000000`), documented in the Win32 Process
/// Creation Flags reference
/// (<https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags>).
/// The app binary sets `windows_subsystem = "windows"` on release builds so
/// its own console doesn't flash; without this flag, every console-subsystem
/// child (python.exe, claude.exe/.cmd, cmd.exe) it spawns opens its own
/// visible console window anyway.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Suppresses the console window a console-subsystem child would otherwise
/// flash open when spawned from this GUI-subsystem app. No-op on non-Windows
/// platforms. Call before `.spawn()`.
///
/// `tokio::process::Command` does NOT implement
/// `std::os::windows::process::CommandExt` — it's a distinct wrapper type
/// with its own inherent `creation_flags` method (this was originally
/// written as one function generic over that std trait, which compiles fine
/// on macOS since none of this is `#[cfg(windows)]`-active there, but fails
/// to build on real Windows; caught only once this actually got compiled on
/// a Windows target). Two entry points instead: one per `Command` type.
#[cfg(windows)]
pub fn suppress_console_window_std(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub fn suppress_console_window_std(command: &mut std::process::Command) {
    let _ = command;
}

#[cfg(windows)]
pub fn suppress_console_window_tokio(command: &mut tokio::process::Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub fn suppress_console_window_tokio(command: &mut tokio::process::Command) {
    let _ = command;
}
