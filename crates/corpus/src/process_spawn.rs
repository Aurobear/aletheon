//! Shared process-spawn behavior for Corpus-owned external executables.

use std::{io, time::Duration};

use tokio::process::{Child, Command};

/// Linux can transiently reject an executable with `ETXTBSY` immediately after
/// another process finishes writing it. Since the child has not started, a
/// short bounded retry is safe and does not repeat any script side effects.
const TRANSIENT_EXEC_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(10),
    Duration::from_millis(25),
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(200),
];

pub(crate) async fn spawn_with_transient_retry(command: &mut Command) -> io::Result<Child> {
    for delay in TRANSIENT_EXEC_RETRY_DELAYS {
        match command.spawn() {
            Err(error) if is_transient_exec_error(&error) => tokio::time::sleep(delay).await,
            result => return result,
        }
    }

    command.spawn()
}

fn is_transient_exec_error(error: &io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(libc::ETXTBSY)
    }

    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn only_text_file_busy_is_a_transient_exec_error() {
        assert!(is_transient_exec_error(&io::Error::from_raw_os_error(
            libc::ETXTBSY
        )));
        assert!(!is_transient_exec_error(&io::Error::from_raw_os_error(
            libc::ENOENT
        )));
    }
}
