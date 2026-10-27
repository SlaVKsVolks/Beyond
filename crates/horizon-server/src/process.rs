//! The programs the server runs. Started by the launcher, the server has no console, and each
//! console program it started would open a terminal window on Windows: those open none. Run from
//! a terminal, they share it, so what they print still reaches it.
use std::{ffi::OsStr, process::Command};

pub fn command(program: impl AsRef<OsStr>) -> Command {
    let command = Command::new(program);
    #[cfg(windows)]
    let command = {
        use std::os::windows::process::CommandExt;
        use win32console::console::WinConsole;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        let mut command = command;
        // Presence, not visibility: a connected pseudoconsole can have an invisible window.
        if WinConsole::get_window().is_none() {
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
    };
    command
}
