//! SIGTERM/SIGINT → graceful shutdown.
//!
//! A signal handler may only use async-signal-safe calls, so it just makes a
//! pipe readable; a watcher thread then pokes eframe and the UI thread runs the
//! normal shutdown (mpv children quit, IPC sockets and the single-instance
//! socket are removed). `PR_SET_PDEATHSIG` in `ml-audio` covers SIGKILL, where
//! no cleanup can run.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

static TERMINATE: AtomicBool = AtomicBool::new(false);
static WRITE_FD: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_signal(_signal: libc::c_int) {
    let fd = WRITE_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        let byte = 1u8;
        // SAFETY: `write` is async-signal-safe and `fd` stays open for the
        // process lifetime.
        unsafe { libc::write(fd, (&byte as *const u8).cast(), 1) };
    }
}

/// Install handlers for SIGTERM/SIGINT and the thread that wakes `ctx`.
pub fn install(ctx: eframe::egui::Context) {
    let mut fds = [0i32; 2];
    // SAFETY: plain pipe setup with a valid out-pointer.
    unsafe {
        if libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) != 0 {
            return;
        }
        // The handler must never block when the pipe is full.
        let flags = libc::fcntl(fds[1], libc::F_GETFL);
        libc::fcntl(fds[1], libc::F_SETFL, flags | libc::O_NONBLOCK);
        WRITE_FD.store(fds[1], Ordering::SeqCst);
    }
    let read_fd = fds[0];
    let watcher = std::thread::Builder::new()
        .name("ml-signals".to_string())
        .spawn(move || {
            let mut byte = 0u8;
            loop {
                // SAFETY: reading one byte from the pipe's read end.
                let read = unsafe { libc::read(read_fd, (&mut byte as *mut u8).cast(), 1) };
                if read == 1 {
                    break;
                }
                if read < 0
                    && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
                {
                    return;
                }
            }
            TERMINATE.store(true, Ordering::SeqCst);
            ctx.request_repaint();
        });
    // Only now install the handlers: with the reader in place a signal can
    // never be swallowed by a handler whose pipe nobody drains.
    if watcher.is_err() {
        return;
    }
    // SAFETY: valid action struct and out-pointers.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as usize;
        action.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
    }
}

/// True once SIGTERM/SIGINT was received.
pub fn terminate_requested() -> bool {
    TERMINATE.load(Ordering::SeqCst)
}
