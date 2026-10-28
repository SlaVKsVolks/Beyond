//! A desktop launch has no console; CLI commands can use the invoking terminal or pipes.
#[cfg(windows)]
pub fn attach_for_cli() -> std::io::Result<()> {
    use win32console::console::WinConsole;
    attach_with(
        std::env::args_os().nth(1).is_some(),
        |kind| {
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
    // Snapshot shared, non-owning handles before attachment can replace them.
    let handles = [Input, Output, Error].map(|kind| (kind, snapshot(kind)));
    if attach(u32::MAX).is_err() {
        // A parent without a console, or an already attached process, is benign.
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
    fn no_arguments_do_not_query_or_attach() {
        attach_with::<u8>(
            false,
            |_| panic!("query"),
            |_| panic!("attach"),
            |_, _| panic!("restore"),
        )
        .unwrap();
    }

    #[test]
    fn failed_attachment_does_not_restore_inherited_handles() {
        let queried = RefCell::new(Vec::new());
        attach_with(
            true,
            |kind| {
                queried.borrow_mut().push(kind);
                Some(9)
            },
            |parent| {
                assert_eq!(parent, u32::MAX);
                Err(io::Error::from_raw_os_error(5))
            },
            |_, _| panic!("restored after failed attach"),
        )
        .unwrap();
        assert_eq!(*queried.borrow(), [Input, Output, Error]);
    }

    #[test]
    fn snapshots_before_attachment_and_restores_valid_handles_in_order() {
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
            |parent| {
                assert_eq!(parent, u32::MAX);
                calls.borrow_mut().push("attach".into());
                Ok(())
            },
            |kind, value| {
                calls.borrow_mut().push(format!("set:{kind:?}:{value}"));
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
    fn first_restore_error_is_returned_and_stops_restoration() {
        let restored = RefCell::new(Vec::new());
        let error = attach_with(
            true,
            |_| Some(1),
            |_| Ok(()),
            |kind, _| {
                restored.borrow_mut().push(kind);
                match kind {
                    Output => Err(io::Error::from_raw_os_error(123)),
                    Error => panic!("continued after error"),
                    _ => Ok(()),
                }
            },
        )
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(123));
        assert_eq!(*restored.borrow(), [Input, Output]);
    }
}
