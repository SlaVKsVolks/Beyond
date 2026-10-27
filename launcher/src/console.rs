//! A desktop launch has no console; CLI commands can use the invoking terminal or pipes.
#[cfg(windows)]
pub fn attach_for_cli() -> std::io::Result<()> {
    use win32console::console::WinConsole;

    attach_with(
        std::env::args_os().nth(1).is_some(),
        |kind| {
            // GetStdHandle's copies are Shared/non-owning. INVALID becomes Err;
            // null is returned successfully by this API and must be filtered separately.
            WinConsole::get_std_handle(kind)
                .ok()
                .filter(|handle| !handle.get_raw().is_null())
        },
        WinConsole::attach_console,
        WinConsole::set_std_handle,
    )
}

#[cfg(windows)]
fn attach_with<H>(
    has_arguments: bool,
    mut snapshot: impl FnMut(win32console::console::HandleType) -> Option<H>,
    attach: impl FnOnce(u32) -> std::io::Result<()>,
    mut restore: impl FnMut(win32console::console::HandleType, H) -> std::io::Result<()>,
) -> std::io::Result<()> {
    use win32console::console::HandleType::{Error, Input, Output};

    if !has_arguments {
        return Ok(());
    }
    // Attaching can replace the standard handles. Keep inherited pipes/files intact for
    // JSON output, MCP and redirected logs, while filling missing handles from the terminal.
    let handles = [Input, Output, Error].map(|kind| (kind, snapshot(kind)));
    if attach(u32::MAX).is_err() {
        // No parent console (for example, an MCP client), or already attached.
        return Ok(());
    }
    for (kind, handle) in handles {
        if let Some(handle) = handle {
            restore(kind, handle)?;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn attach_for_cli() -> std::io::Result<()> {
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::attach_with;
    use std::{cell::RefCell, io};
    use win32console::console::HandleType::{Error, Input, Output};

    #[test]
    fn no_arguments_never_query_or_attach() {
        attach_with::<u8>(
            false,
            |_| panic!("snapshot"),
            |_| panic!("attach"),
            |_, _| panic!("restore"),
        )
        .unwrap();
    }

    #[test]
    fn failed_attach_preserves_the_snapshot_without_restoration() {
        let calls = RefCell::new(Vec::new());
        attach_with(
            true,
            |kind| {
                calls.borrow_mut().push(format!("get:{kind:?}"));
                Some(9)
            },
            |parent| {
                assert_eq!(parent, u32::MAX);
                calls.borrow_mut().push("attach".into());
                Err(io::Error::from_raw_os_error(5))
            },
            |_, _| panic!("restore after failed attachment"),
        )
        .unwrap();
        assert_eq!(
            *calls.borrow(),
            ["get:Input", "get:Output", "get:Error", "attach"]
        );
    }

    #[test]
    fn successful_attach_restores_only_saved_handles_in_input_output_error_order() {
        let calls = RefCell::new(Vec::new());
        attach_with(
            true,
            |kind| {
                calls.borrow_mut().push(format!("get:{kind:?}"));
                match kind {
                    Input => Some(11),
                    Output => None,
                    Error => Some(33),
                }
            },
            |_| {
                calls.borrow_mut().push("attach".into());
                Ok(())
            },
            |kind, handle| {
                calls.borrow_mut().push(format!("set:{kind:?}:{handle}"));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            *calls.borrow(),
            [
                "get:Input",
                "get:Output",
                "get:Error",
                "attach",
                "set:Input:11",
                "set:Error:33"
            ]
        );
    }

    #[test]
    fn first_restoration_error_retains_its_os_code_and_stops() {
        let calls = RefCell::new(Vec::new());
        let error = attach_with(
            true,
            |_| Some(1),
            |_| Ok(()),
            |kind, _| {
                calls.borrow_mut().push(kind);
                match kind {
                    Input => Ok(()),
                    Output => Err(io::Error::from_raw_os_error(123)),
                    Error => panic!("continued after failure"),
                }
            },
        )
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(123));
        assert_eq!(*calls.borrow(), [Input, Output]);
    }
}
