use std::io::{self, IsTerminal};
use std::sync::mpsc;
use std::time::Duration;

/// Wait up to `timeout` for the user to press Enter on stdin.
///
/// Returns `true` if Enter was pressed within the timeout, `false` if the
/// timeout elapsed or stdin is not a terminal (non-interactive environment).
///
/// In non-interactive environments (piped input, no TTY), returns `false`
/// immediately without waiting.
pub fn wait_for_enter_or_timeout(timeout: Duration) -> bool {
    if !io::stdin().is_terminal() {
        return false;
    }

    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = io::stdin().read_line(&mut buf);
        let _ = tx.send(());
    });

    rx.recv_timeout(timeout).is_ok()
}
