//! Scoped POSIX shutdown handlers. Handlers perform only a lock-free atomic store.
use std::{
    io,
    sync::atomic::{AtomicBool, AtomicI32, Ordering},
};
static ACTIVE: AtomicBool = AtomicBool::new(false);
static PENDING: AtomicI32 = AtomicI32::new(0);
extern "C" fn shutdown(signal: libc::c_int) {
    PENDING.store(signal, Ordering::Relaxed);
}
pub(crate) fn pending() -> Option<i32> {
    match PENDING.load(Ordering::Relaxed) {
        0 => None,
        s => Some(s),
    }
}
pub(crate) struct SignalSession {
    previous: Vec<(i32, libc::sigaction)>,
}
impl SignalSession {
    pub(crate) fn enter() -> io::Result<Self> {
        ACTIVE
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| io::Error::other("another interactive signal session is active"))?;
        PENDING.store(0, Ordering::Relaxed);
        let mut session = Self {
            previous: Vec::new(),
        };
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = shutdown as *const () as usize;
                libc::sigemptyset(&mut action.sa_mask);
                let mut previous = std::mem::zeroed();
                if libc::sigaction(signal, &action, &mut previous) != 0 {
                    return Err(io::Error::last_os_error());
                }
                session.previous.push((signal, previous));
            }
        }
        Ok(session)
    }
    pub(crate) fn pending_signal(&self) -> Option<i32> {
        pending()
    }
}
impl Drop for SignalSession {
    fn drop(&mut self) {
        for (signal, action) in self.previous.iter().rev() {
            unsafe {
                libc::sigaction(*signal, action, std::ptr::null_mut());
            }
        }
        PENDING.store(0, Ordering::Relaxed);
        ACTIVE.store(false, Ordering::SeqCst);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restores_handlers_and_rejects_concurrent_sessions() {
        unsafe {
            let mut before: libc::sigaction = std::mem::zeroed();
            libc::sigaction(libc::SIGTERM, std::ptr::null(), &mut before);
            {
                let _guard = SignalSession::enter().unwrap();
                assert!(SignalSession::enter().is_err());
                libc::raise(libc::SIGTERM);
                assert_eq!(pending(), Some(libc::SIGTERM));
            }
            let mut after: libc::sigaction = std::mem::zeroed();
            libc::sigaction(libc::SIGTERM, std::ptr::null(), &mut after);
            assert_eq!(before.sa_sigaction, after.sa_sigaction);
            assert_eq!(pending(), None);
        }
    }
}
