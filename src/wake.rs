//! **Waking the host from another thread.** A terminal's reader thread
//! (E5) and a remote connection's reader (async sends) learn of new output
//! off the UI thread; a pane's dirty flag is the UI thread's, so they ask
//! the host's event loop to come round instead (winit's proxy, in both
//! hosts). On waking, a host redraws and delivers waiting answers —
//! `(remote-poll)`.

use std::sync::OnceLock;

static WAKER: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Install the host's wake-up. The first one stays.
pub fn set_waker(wake: Box<dyn Fn() + Send + Sync>) {
    let _ = WAKER.set(wake);
}

/// Ask the host to come round. Nothing, before a host installed a waker.
pub fn wake() {
    if let Some(w) = WAKER.get() {
        w();
    }
}
