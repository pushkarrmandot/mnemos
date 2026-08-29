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
/// Generic over `C` so it works on both `std::process::Command` (used for
/// the `claude` CLI and `cmd /C start` spawns) and `tokio::process::Command`
/// (used for the Python worker spawn) — both implement
/// `std::os::windows::process::CommandExt` on Windows, so one function body
/// covers every call site instead of repeating the `#[cfg(windows)]` block
/// three times.
#[cfg(windows)]
pub fn suppress_console_window<C: std::os::windows::process::CommandExt>(command: &mut C) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub fn suppress_console_window<C>(command: &mut C) {
    let _ = command;
}
