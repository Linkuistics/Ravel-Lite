//! Interactive PTY-capture path for diagnostic byte-level logging.
//!
//! When `--debug` is on, ravel-lite spawns the interactive child
//! (claude) under a freshly-allocated pseudo-terminal instead of
//! inheriting the parent's stdio. The child still sees a real tty so
//! its TUI renders normally; ravel-lite copies bytes between the
//! parent's terminal and the PTY master end, teeing every byte to the
//! debug log.
//!
//! Why this exists: the inherit-stdio path leaves ravel-lite blind to
//! what the child prints. On a fresh machine the work-phase hangs with
//! claude apparently invisible after the banner. Without a transcript
//! we cannot tell whether claude printed nothing, printed an unseen
//! permission modal, emitted escape sequences the terminal didn't
//! honour, or exited silently. The PTY path turns every future hang
//! of this shape into a log-readable event.
//!
//! Scope: interactive-only. The headless paths already pipe stdout
//! and stderr separately; PTYs merge the two streams so they do not
//! suit headless capture.

use std::io::{self, Read, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{
    Child, CommandBuilder, ExitStatus, MasterPty, PtyPair, PtySize, native_pty_system,
};

use crate::debug_log;

/// Buffer size for the master→stdout copy. Sized to comfortably hold
/// a single ANSI repaint of a typical 80x24 frame without splitting it
/// across log entries, while staying small enough that an idle
/// connection doesn't sit on a large allocation.
const MASTER_TO_PARENT_BUF: usize = 4096;

/// Buffer size for the stdin→master copy. Keystrokes arrive in tiny
/// chunks so this can stay small.
const PARENT_TO_CHILD_BUF: usize = 1024;

/// Poll interval for the stdin pump. The thread checks the shutdown
/// flag every iteration, so a small value bounds how long the function
/// waits for the stdin thread to notice that the child has exited.
/// Memory note `spawn-blocking-does-not-cancel-cleanly-in-tokio-select`
/// rules out spawn_blocking + tokio::select for this; an OS thread
/// with a poll-and-flag loop is the cancellable alternative.
const STDIN_POLL_TIMEOUT_MS: i32 = 100;

/// Default PTY size when the parent terminal size cannot be queried
/// (e.g. integration tests). Matches portable-pty's docs example.
const DEFAULT_PTY_ROWS: u16 = 24;
const DEFAULT_PTY_COLS: u16 = 80;

/// Spawn `program` with `args` (cwd `cwd`) under a freshly-allocated
/// pseudo-terminal, tee every byte to the debug log under
/// `debug_label`, and wait for the child to exit. The parent's tty is
/// switched to raw mode for the duration so keystrokes pass through
/// to the child unbuffered, then restored to its prior mode.
///
/// Caller must guarantee the parent terminal is currently OUT of raw
/// mode (the TUI's Suspend handler does this before invoking the
/// interactive phase). Returns the child's `ExitStatus`.
pub fn run_pty_session(
    program: &str,
    args: &[String],
    cwd: &str,
    debug_label: &str,
) -> Result<ExitStatus> {
    let size = current_terminal_size().unwrap_or(PtySize {
        rows: DEFAULT_PTY_ROWS,
        cols: DEFAULT_PTY_COLS,
        pixel_width: 0,
        pixel_height: 0,
    });

    let pty_system = native_pty_system();
    let pair = pty_system.openpty(size).context("openpty failed")?;

    let mut cmd = CommandBuilder::new(program);
    for arg in args {
        cmd.arg(arg);
    }
    cmd.cwd(cwd);

    let child = pair
        .slave
        .spawn_command(cmd)
        .with_context(|| format!("Failed to spawn {program} under PTY"))?;

    debug_log::log(
        &format!("{debug_label} pty open"),
        &format!(
            "rows: {} cols: {} pid: {:?}",
            size.rows,
            size.cols,
            child.process_id()
        ),
    );

    let _raw_guard = RawModeGuard::enable()?;

    // SAFETY: stdin/stdout are owned by the process for the whole life
    // of the program; the helpers below borrow them via locks.
    let stdin = io::stdin();
    let stdout = io::stdout();

    let status = drive_pty_io(pair, child, stdin, stdout, debug_label)?;

    debug_log::log(
        &format!("{debug_label} pty close"),
        &format!("status: {status:?}"),
    );

    Ok(status)
}

/// Inner driver: takes a PTY pair, the spawned child, and parent
/// stdin/stdout streams. Tees bytes between them and the PTY master,
/// returning the child's exit status when it terminates.
///
/// Factored out of `run_pty_session` so tests can pass in synthetic
/// stdin/stdout (a real test harness has no controlling terminal).
pub(crate) fn drive_pty_io<R, W>(
    pair: PtyPair,
    mut child: Box<dyn Child + Send + Sync>,
    parent_in: R,
    parent_out: W,
    debug_label: &str,
) -> Result<ExitStatus>
where
    R: AsRawFd + Send + 'static,
    W: Write + Send + 'static,
{
    // Drop the slave fd in the parent. Without this, the master's
    // read won't see EOF when the child exits (the slave fd held by
    // the parent keeps the pty open).
    let PtyPair { master, slave } = pair;
    drop(slave);

    let master_reader = master
        .try_clone_reader()
        .context("PTY try_clone_reader failed")?;
    let master_writer = master.take_writer().context("PTY take_writer failed")?;

    let shutdown = Arc::new(AtomicBool::new(false));

    // Master → parent stdout (+ debug log)
    let label_out = debug_label.to_string();
    let stdout_handle = {
        let mut parent_out = parent_out;
        thread::spawn(move || -> io::Result<()> {
            tee_to(master_reader, &mut parent_out, |chunk| {
                debug_log::log_pty_chunk(&label_out, "child→parent", chunk);
            })
        })
    };

    // Parent stdin → master (cancellable via shutdown flag)
    let label_in = debug_label.to_string();
    let shutdown_for_stdin = shutdown.clone();
    let stdin_fd = parent_in.as_raw_fd();
    let stdin_handle = thread::spawn(move || -> io::Result<()> {
        // Hold parent_in until the loop exits to keep the fd open.
        let _hold = parent_in;
        pump_stdin_with_shutdown(stdin_fd, master_writer, shutdown_for_stdin, &label_in)
    });

    // SIGWINCH → resize PTY. Best-effort: if signal-hooking fails we
    // still capture I/O, just without dynamic resize. The master is
    // moved into the resize thread, which holds it alive until
    // shutdown — `MasterPty` is `Send` but not `Sync`, so single-owner
    // is the only sound pattern here.
    #[cfg(unix)]
    let resize_handle = spawn_resize_forwarder(master, shutdown.clone());
    #[cfg(not(unix))]
    let _master_keepalive = master;

    let status = child.wait().context("child wait failed")?;

    // Signal stdin pump (and resize thread) to exit. Both check the
    // flag every STDIN_POLL_TIMEOUT_MS, so worst-case shutdown
    // latency is bounded.
    shutdown.store(true, Ordering::Release);

    // Join the stdout pump first — when the child exits the master
    // closes and read returns 0, so this terminates promptly.
    if let Err(panic) = stdout_handle.join() {
        anyhow::bail!("PTY stdout pump panicked: {panic:?}");
    }

    // Join the stdin pump. Its poll-loop wakes within
    // STDIN_POLL_TIMEOUT_MS of the shutdown flag flip.
    if let Err(panic) = stdin_handle.join() {
        anyhow::bail!("PTY stdin pump panicked: {panic:?}");
    }

    #[cfg(unix)]
    {
        if let Some(handle) = resize_handle {
            let _ = handle.join();
        }
    }

    Ok(status)
}

/// Read from `reader`, write everything to `writer`, and call `tap`
/// with each chunk before forwarding. Loops until reader hits EOF or
/// returns an error other than `Interrupted`.
pub(crate) fn tee_to<R, W, F>(mut reader: R, writer: &mut W, mut tap: F) -> io::Result<()>
where
    R: Read,
    W: Write,
    F: FnMut(&[u8]),
{
    let mut buf = [0u8; MASTER_TO_PARENT_BUF];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                let chunk = &buf[..n];
                tap(chunk);
                writer.write_all(chunk)?;
                writer.flush()?;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
}

/// Read keystrokes from `stdin_fd` (using `libc::poll` so we can wake
/// on the shutdown flag instead of blocking forever in `read`) and
/// forward them to `writer`. Logs each chunk to the debug log under
/// `debug_label`. Returns when the shutdown flag is set or stdin hits
/// EOF.
fn pump_stdin_with_shutdown(
    stdin_fd: RawFd,
    mut writer: Box<dyn Write + Send>,
    shutdown: Arc<AtomicBool>,
    debug_label: &str,
) -> io::Result<()> {
    let mut buf = [0u8; PARENT_TO_CHILD_BUF];
    loop {
        if shutdown.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut pfd = libc::pollfd {
            fd: stdin_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: pollfd is a single valid struct; libc::poll is a
        // thread-safe syscall.
        let rc = unsafe { libc::poll(&mut pfd, 1, STDIN_POLL_TIMEOUT_MS) };
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if rc == 0 {
            // Timeout: loop back and re-check shutdown flag.
            continue;
        }
        if pfd.revents & libc::POLLIN == 0 {
            // POLLHUP/POLLERR — treat as EOF.
            return Ok(());
        }
        // SAFETY: buf is a valid writable slice of length buf.len().
        let n = unsafe {
            libc::read(
                stdin_fd,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
            )
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if n == 0 {
            return Ok(());
        }
        let chunk = &buf[..n as usize];
        debug_log::log_pty_chunk(debug_label, "parent→child", chunk);
        writer.write_all(chunk)?;
        writer.flush()?;
    }
}

#[cfg(unix)]
fn spawn_resize_forwarder(
    master: Box<dyn MasterPty + Send>,
    shutdown: Arc<AtomicBool>,
) -> Option<thread::JoinHandle<()>> {
    use std::sync::Mutex;

    static SIGWINCH_FLAG: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
    static REGISTERED: Mutex<bool> = Mutex::new(false);

    let flag = SIGWINCH_FLAG
        .get_or_init(|| Arc::new(AtomicBool::new(false)))
        .clone();

    {
        let mut registered = match REGISTERED.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !*registered {
            // SAFETY: signal() is a one-time global registration.
            // SIG_DFL → custom handler is well-defined; the handler
            // only does an atomic store, which is async-signal-safe.
            extern "C" fn handler(_: libc::c_int) {
                if let Some(flag) = SIGWINCH_FLAG.get() {
                    flag.store(true, Ordering::Release);
                }
            }
            unsafe {
                libc::signal(libc::SIGWINCH, handler as *const () as libc::sighandler_t);
            }
            *registered = true;
        }
    }

    Some(thread::spawn(move || {
        loop {
            if shutdown.load(Ordering::Acquire) {
                return;
            }
            if flag.swap(false, Ordering::AcqRel) {
                if let Some(size) = current_terminal_size() {
                    let _ = master.resize(size);
                }
            }
            thread::sleep(std::time::Duration::from_millis(STDIN_POLL_TIMEOUT_MS as u64));
        }
    }))
}

/// Query the parent terminal's current size via TIOCGWINSZ. Returns
/// `None` if stdout is not a tty (e.g. test harness, redirected
/// output) — caller falls back to a sensible default.
fn current_terminal_size() -> Option<PtySize> {
    #[cfg(unix)]
    {
        let mut ws = libc::winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: ws is a valid writable struct; ioctl returns an
        // error on failure rather than corrupting memory.
        let rc =
            unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws as *mut _) };
        if rc != 0 || ws.ws_row == 0 || ws.ws_col == 0 {
            return None;
        }
        Some(PtySize {
            rows: ws.ws_row,
            cols: ws.ws_col,
            pixel_width: ws.ws_xpixel,
            pixel_height: ws.ws_ypixel,
        })
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// RAII guard that puts the parent terminal into raw mode on `enable`
/// and restores it on drop. Uses crossterm so behaviour matches the
/// rest of the TUI's terminal handling. Drop is best-effort: if the
/// restore fails (e.g. the terminal disappeared), the user's shell
/// will reset its own state on the next prompt.
struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> Result<Self> {
        crossterm::terminal::enable_raw_mode().context("enable_raw_mode failed")?;
        Ok(RawModeGuard)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::Mutex;

    #[test]
    fn tee_to_copies_bytes_and_invokes_tap() {
        // Pure logic: a Cursor as the source, a Vec<u8> as the sink,
        // and a Mutex<Vec<Vec<u8>>> capturing each chunk handed to the
        // tap. Verifies (a) every byte reaches the writer, (b) the tap
        // sees the same bytes in the same order.
        let source = Cursor::new(b"hello\nworld".to_vec());
        let mut sink = Vec::new();
        let captured: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

        tee_to(source, &mut sink, |chunk| {
            captured.lock().unwrap().push(chunk.to_vec());
        })
        .expect("tee_to should succeed on Cursor");

        assert_eq!(sink, b"hello\nworld");
        let chunks = captured.lock().unwrap().clone();
        let joined: Vec<u8> = chunks.into_iter().flatten().collect();
        assert_eq!(joined, b"hello\nworld");
    }

    #[test]
    fn tee_to_handles_empty_source() {
        // EOF on the very first read: tap is never called, sink stays
        // empty, no error.
        let source: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        let mut sink = Vec::new();
        let mut tap_calls = 0;

        tee_to(source, &mut sink, |_| tap_calls += 1).expect("empty tee_to should succeed");

        assert_eq!(sink.len(), 0);
        assert_eq!(tap_calls, 0);
    }

    #[test]
    fn tee_to_propagates_writer_errors() {
        // A writer that always errors: tee_to surfaces the error
        // rather than swallowing it. This guards against silent loss
        // of bytes when the parent's stdout closes.
        struct FailingWriter;
        impl Write for FailingWriter {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "stdout gone"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let source = Cursor::new(b"abc".to_vec());
        let mut sink = FailingWriter;
        let err = tee_to(source, &mut sink, |_| {}).expect_err("must surface writer error");
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    /// Writes a tiny shell shim into a tempdir and returns its path.
    /// The shim prints `body` to stdout and exits with `exit_code`.
    /// Used by the PTY integration tests below.
    fn write_shim(dir: &std::path::Path, name: &str, body: &str, exit_code: i32) -> std::path::PathBuf {
        let path = dir.join(name);
        let script = format!(
            "#!/bin/sh\nprintf '%s' '{}'\nexit {}\n",
            body.replace('\'', "'\\''"),
            exit_code
        );
        std::fs::write(&path, script).expect("write shim");
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    /// A no-op "stdin" that never has bytes to read. Used by the
    /// integration tests where no parent input is being typed.
    /// `as_raw_fd` returns the fd of `/dev/null`, so the poll loop
    /// blocks until the shutdown flag flips.
    struct NullStdin {
        fd: std::fs::File,
    }
    impl NullStdin {
        fn open() -> Self {
            Self {
                fd: std::fs::File::open("/dev/null").expect("/dev/null"),
            }
        }
    }
    impl AsRawFd for NullStdin {
        fn as_raw_fd(&self) -> RawFd {
            self.fd.as_raw_fd()
        }
    }

    #[test]
    fn drive_pty_io_captures_child_stdout_to_writer() {
        // Spawn a shim under a real PTY, route master output to a
        // shared Vec<u8>, and verify the shim's text comes through.
        // No debug log is enabled here, so this test exercises the
        // copy path independently of the log-tap.
        let tmp = tempfile::tempdir().expect("tempdir");
        let shim = write_shim(tmp.path(), "fake_claude.sh", "HELLO PTY WORLD", 0);

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: DEFAULT_PTY_ROWS,
                cols: DEFAULT_PTY_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");

        let mut cmd = CommandBuilder::new(shim.to_str().unwrap());
        cmd.cwd(tmp.path());
        let child = pair.slave.spawn_command(cmd).expect("spawn shim");

        let captured = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = SharedWriter(captured.clone());

        let status = drive_pty_io(pair, child, NullStdin::open(), writer, "test")
            .expect("drive_pty_io ok");
        assert!(status.success(), "shim should exit 0");

        let bytes = captured.lock().unwrap().clone();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("HELLO PTY WORLD"), "captured: {text:?}");
    }

    #[test]
    fn drive_pty_io_propagates_nonzero_exit_status() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let shim = write_shim(tmp.path(), "fail.sh", "boom", 17);

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: DEFAULT_PTY_ROWS,
                cols: DEFAULT_PTY_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");

        let mut cmd = CommandBuilder::new(shim.to_str().unwrap());
        cmd.cwd(tmp.path());
        let child = pair.slave.spawn_command(cmd).expect("spawn shim");

        let captured = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = SharedWriter(captured);

        let status = drive_pty_io(pair, child, NullStdin::open(), writer, "test")
            .expect("drive_pty_io ok");
        assert!(!status.success(), "shim exited 17");
        assert_eq!(status.exit_code(), 17);
    }

    /// A `Write` that pushes into a shared `Vec<u8>` so a thread can
    /// own the writer while the test reads what was captured.
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);
    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
