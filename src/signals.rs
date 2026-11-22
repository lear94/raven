//! Ctrl-C handling. Outside critical sections an interrupt exits immediately.
//! Inside a critical section (moving files into the system and committing the
//! database) the interrupt is deferred until the transaction has finished, so
//! the system is never left half-installed.

use crate::core::RavenError;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static CRITICAL_DEPTH: AtomicUsize = AtomicUsize::new(0);
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

pub fn install_handler() {
    tokio::spawn(async {
        loop {
            if tokio::signal::ctrl_c().await.is_err() {
                return;
            }
            if CRITICAL_DEPTH.load(Ordering::SeqCst) > 0 {
                INTERRUPTED.store(true, Ordering::SeqCst);
                eprintln!("\nInterrupt received: finishing the current transaction safely before exiting...");
            } else {
                eprintln!("\nInterrupted.");
                std::process::exit(130);
            }
        }
    });
}

pub struct CriticalSection(());

impl CriticalSection {
    pub fn enter() -> Self {
        CRITICAL_DEPTH.fetch_add(1, Ordering::SeqCst);
        CriticalSection(())
    }
}

impl Drop for CriticalSection {
    fn drop(&mut self) {
        CRITICAL_DEPTH.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Returns an error if an interrupt was deferred during a critical section.
pub fn check_interrupted() -> Result<(), RavenError> {
    if INTERRUPTED.load(Ordering::SeqCst) {
        Err(RavenError::Interrupted)
    } else {
        Ok(())
    }
}
