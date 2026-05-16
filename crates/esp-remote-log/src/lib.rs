//! Stream ESP-IDF log output over TCP, with on-connect replay from a bounded
//! ring buffer.
//!
//! Lifecycle:
//!   * `init(LogTarget::TcpServer { port, buffer_bytes })` installs an
//!     `esp_log_set_vprintf` hook and spawns a thread that accepts TCP
//!     connections on `port`.
//!   * Every `ESP_LOGI / log::info!` line goes through the hook. The hook
//!     renders the line once with `vsnprintf`, writes it to stdout (UART
//!     remains intact for serial monitors), appends it to a ring buffer of
//!     `buffer_bytes` bytes, and fans it out to every currently-connected
//!     TCP client.
//!   * When a client connects, the server first writes the current ring
//!     buffer contents (recent history → boot logs if still resident) and
//!     then includes the client in subsequent fan-outs.
//!
//! Backpressure policy:
//!   * Each client socket gets a short `write_timeout`. If a client can't
//!     keep up, write_all returns `TimedOut` and the client is dropped on
//!     the spot. The MCU's logging path never blocks indefinitely.
//!   * The ring buffer is FIFO-truncated; the producer never blocks on a
//!     full buffer either.
//!
//! Re-entrance / safety:
//!   * The vprintf hook runs from arbitrary tasks. It must not call
//!     `log::*` itself (would recurse) and only ever holds the state mutex
//!     via `try_lock` so it can't deadlock the accept thread.
//!   * `va_list` is consumed exactly once via `vsnprintf` into a stack
//!     buffer; the rendered bytes are then re-emitted.

use std::collections::VecDeque;
use std::ffi::{c_char, c_int};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use esp_idf_sys::{esp_log_set_vprintf, va_list};

// `vsnprintf` is in newlib (libc) and links cleanly, but esp-idf-sys doesn't
// emit a binding for it. Declare it ourselves so we can render a `va_list`
// into a fixed-size stack buffer.
unsafe extern "C" {
    fn vsnprintf(s: *mut c_char, n: usize, format: *const c_char, args: va_list) -> c_int;
}

/// Where logs should go.
#[derive(Debug, Clone, Copy)]
pub enum LogTarget {
    /// Run a TCP server on `port` that streams logs to any connected client.
    /// Each new client first receives the contents of the in-memory ring
    /// buffer (up to `buffer_bytes` bytes, FIFO-truncated), then receives
    /// every new line as it is emitted.
    TcpServer { port: u16, buffer_bytes: usize },
}

/// Install (or replace) the vprintf hook and start the TCP server if not
/// already running.
pub fn init(target: LogTarget) -> Result<(), esp_idf_sys::EspError> {
    let LogTarget::TcpServer { port, buffer_bytes } = target;

    // Initialise / replace the shared sink. Existing client connections
    // (if any) are dropped intentionally — caller asked to re-init.
    *state().lock().unwrap_or_else(|p| p.into_inner()) = Some(LogSink::new(buffer_bytes));

    // Install the vprintf hook exactly once. We deliberately discard the
    // previous hook pointer: chaining `va_list` is undefined behaviour on
    // most ABIs including Xtensa, and stdout already covers UART output.
    INSTALLED.get_or_init(|| {
        // SAFETY: matches `vprintf_like_t = int (*)(const char*, va_list)`.
        let _prev = unsafe { esp_log_set_vprintf(Some(remote_log_vprintf)) };
    });

    // Spawn the accept thread exactly once. The first init's port is the
    // one that sticks — re-init with a different port keeps the original.
    ACCEPT_THREAD.get_or_init(|| {
        thread::Builder::new()
            .name("rlog-accept".into())
            .stack_size(4 * 1024)
            .spawn(move || accept_loop(port))
            .expect("spawn remote-log accept thread")
    });

    Ok(())
}

static INSTALLED: OnceLock<()> = OnceLock::new();
static ACCEPT_THREAD: OnceLock<thread::JoinHandle<()>> = OnceLock::new();

struct LogSink {
    /// Bounded ring of recently emitted bytes.
    ring: VecDeque<u8>,
    ring_cap: usize,
    /// Currently connected clients. Dead ones are pruned on every write.
    clients: Vec<TcpStream>,
}

impl LogSink {
    fn new(ring_cap: usize) -> Self {
        Self {
            ring: VecDeque::with_capacity(ring_cap),
            ring_cap,
            clients: Vec::new(),
        }
    }
}

fn state() -> &'static Mutex<Option<LogSink>> {
    static STATE: OnceLock<Mutex<Option<LogSink>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

/// Per-line buffer cap; comfortably larger than ESP-IDF's longest tag.
const LINE_BUF: usize = 512;
/// How long a client write may block before we declare it stale and drop.
const CLIENT_WRITE_TIMEOUT: Duration = Duration::from_millis(200);

fn accept_loop(port: u16) {
    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(l) => l,
        Err(e) => {
            // We can use log::* here — vprintf hook is installed but recursion
            // ends at the format step. Just emit the error and bail out.
            log::error!("remote log: TCP bind on port {port} failed: {e}");
            return;
        }
    };
    log::info!("remote log: TCP listening on 0.0.0.0:{port}");

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let peer = stream.peer_addr().ok();
                if let Err(e) = register_client(stream) {
                    log::warn!("remote log: rejecting client {peer:?}: {e}");
                }
            }
            Err(e) => log::warn!("remote log: accept failed: {e}"),
        }
    }
}

/// Replay the ring buffer to the newcomer, then add them to the fan-out
/// list. We hold the state lock for the duration of the replay so that no
/// new log lines slip in between "replay" and "register" — this guarantees
/// the client sees a contiguous (old → new) stream.
fn register_client(mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_write_timeout(Some(CLIENT_WRITE_TIMEOUT))?;
    stream.set_nodelay(true).ok();

    let mut guard = state().lock().unwrap_or_else(|p| p.into_inner());
    let sink = match guard.as_mut() {
        Some(s) => s,
        None => return Ok(()),
    };

    // Write the ring buffer in its two physical halves.
    let (front, back) = sink.ring.as_slices();
    stream.write_all(front)?;
    stream.write_all(back)?;

    sink.clients.push(stream);
    Ok(())
}

/// vprintf hook installed via `esp_log_set_vprintf`.
unsafe extern "C" fn remote_log_vprintf(fmt: *const c_char, args: va_list) -> c_int {
    let mut buf = [0u8; LINE_BUF];
    let want = unsafe { vsnprintf(buf.as_mut_ptr() as *mut c_char, buf.len(), fmt, args) };
    if want <= 0 {
        return want;
    }
    let len = (want as usize).min(buf.len() - 1);
    let line = &buf[..len];

    // 1) UART — fd 1 under ESP-IDF's VFS goes to the serial console.
    let _ = std::io::stdout().write_all(line);

    // 2) Append to ring buffer + fan-out. `try_lock` so the hook never
    //    blocks the producer task; if a client is being registered right
    //    now, we drop this single line from the network path. It will
    //    still hit UART above.
    if let Ok(mut guard) = state().try_lock() {
        if let Some(sink) = guard.as_mut() {
            append_to_ring(sink, line);
            fan_out(sink, line);
        }
    }

    want
}

fn append_to_ring(sink: &mut LogSink, bytes: &[u8]) {
    let cap = sink.ring_cap;
    if cap == 0 {
        return;
    }
    if bytes.len() >= cap {
        // The new line is bigger than the entire ring — keep only its tail.
        sink.ring.clear();
        sink.ring.extend(bytes[bytes.len() - cap..].iter().copied());
        return;
    }
    while sink.ring.len() + bytes.len() > cap {
        sink.ring.pop_front();
    }
    sink.ring.extend(bytes.iter().copied());
}

fn fan_out(sink: &mut LogSink, bytes: &[u8]) {
    sink.clients.retain_mut(|client| client.write_all(bytes).is_ok());
}
