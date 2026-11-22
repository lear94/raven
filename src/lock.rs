//! Exclusive process lock so that two raven instances never modify the system
//! at the same time. The lock is released automatically when the process exits.

use crate::core::RavenError;
use crate::ui::log_warn;
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::Path;

pub struct ProcessLock {
    _file: File,
}

impl ProcessLock {
    pub fn acquire(path: &Path) -> Result<Self, RavenError> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        let fd = file.as_raw_fd();
        // SAFETY: flock on a valid, owned file descriptor.
        if unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(err.into());
            }
            log_warn("Another raven process is running; waiting for it to finish...");
            loop {
                // SAFETY: as above.
                if unsafe { libc::flock(fd, libc::LOCK_EX) } == 0 {
                    break;
                }
                let err = std::io::Error::last_os_error();
                if err.kind() != std::io::ErrorKind::Interrupted {
                    return Err(err.into());
                }
            }
        }
        Ok(Self { _file: file })
    }
}
