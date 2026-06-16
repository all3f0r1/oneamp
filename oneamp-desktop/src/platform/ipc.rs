//! Single-instance plumbing via a cross-platform local socket.
//!
//! On startup `try_forward` attempts to connect to an existing primary
//! and hand off the paths it received via argv. On success the
//! secondary exits silently — the file manager's "Open with OneAmp"
//! call lands in the already-running window instead of spawning a
//! second binary.
//!
//! When `try_forward` fails (no primary listening), `bind_primary`
//! claims the socket/pipe and returns a `Receiver<Vec<PathBuf>>` that
//! future secondaries push into. The listener thread is detached for
//! the lifetime of the process.
//!
//! Backend per OS (handled by the `interprocess` crate):
//!   - Linux / macOS — Unix domain socket on a filesystem path under
//!     `$XDG_RUNTIME_DIR` (Linux) or `$TMPDIR` (macOS).
//!   - Windows — Named pipe at `\\.\pipe\oneamp-<user>`.

use crossbeam_channel::{Receiver, Sender, unbounded};
use interprocess::local_socket::{ListenerOptions, Stream, prelude::*};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::thread;

/// Filesystem path of the Unix socket. Used both to construct the
/// `Name` and to remove the file on graceful shutdown.
#[cfg(unix)]
fn socket_path() -> PathBuf {
    let user = std::env::var("USER").unwrap_or_else(|_| "anon".into());
    // `$XDG_RUNTIME_DIR` on Linux (per-user tmpfs, cleaned at logout).
    // `$TMPDIR` on macOS (per-user `/var/folders/.../T`, set by the
    // OS). `/tmp` as a last-resort fallback — suffix with user so a
    // multi-user host doesn't see two players collide.
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("TMPDIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    dir.join(format!("oneamp-{user}.sock"))
}

#[cfg(unix)]
fn socket_name() -> io::Result<interprocess::local_socket::Name<'static>> {
    use interprocess::local_socket::GenericFilePath;
    socket_path()
        .into_os_string()
        .to_fs_name::<GenericFilePath>()
}

#[cfg(windows)]
fn socket_name() -> io::Result<interprocess::local_socket::Name<'static>> {
    use interprocess::local_socket::GenericNamespaced;
    // The OS maps a namespaced name to `\\.\pipe\<name>`. Suffix with
    // `USERNAME` so a multi-session terminal server doesn't pile two
    // OneAmps onto the same pipe.
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "anon".into());
    format!("oneamp-{user}").to_ns_name::<GenericNamespaced>()
}

/// Try to forward `paths` to a primary listening on the socket. Returns
/// `true` when the handoff succeeded (caller should exit 0), `false`
/// when no primary is reachable (caller becomes the primary).
///
/// "Not reachable" covers both *no listener bound* and *socket file
/// exists but no listener* (a previous crash left it behind) — both
/// surface as a connect error, and we treat both as "we are the primary".
pub fn try_forward(paths: &[PathBuf]) -> bool {
    if paths.is_empty() {
        return false;
    }
    let Ok(name) = socket_name() else {
        return false;
    };
    let mut stream = match Stream::connect(name) {
        Ok(s) => s,
        Err(_) => return false,
    };
    write_paths(&mut stream, paths).is_ok()
}

/// Become the primary: bind the socket/pipe, spawn a detached listener
/// thread that pushes incoming path batches into the returned receiver,
/// and return a `PrimaryGuard` whose Drop removes the Unix socket file
/// (no-op on Windows where there is no filesystem entry).
pub fn bind_primary() -> io::Result<(Receiver<Vec<PathBuf>>, PrimaryGuard)> {
    let name = socket_name()?;
    // ListenerOptions::reclaim_name(true) is the default on Unix — it
    // unlinks a stale socket file from a crashed previous primary
    // before binding. On Windows there is no analogous cleanup needed
    // (named pipes vanish with the process that bound them).
    let listener = ListenerOptions::new().name(name).create_sync()?;

    let (tx, rx) = unbounded::<Vec<PathBuf>>();
    let tx_clone = tx.clone();
    thread::Builder::new()
        .name("oneamp-ipc".into())
        .spawn(move || listener_loop(listener, tx_clone))?;

    Ok((
        rx,
        PrimaryGuard {
            #[cfg(unix)]
            path: socket_path(),
        },
    ))
}

fn listener_loop(listener: interprocess::local_socket::Listener, tx: Sender<Vec<PathBuf>>) {
    for incoming in listener.incoming() {
        let Ok(mut stream) = incoming else { continue };
        if let Ok(paths) = read_paths(&mut stream)
            && !paths.is_empty()
        {
            let _ = tx.send(paths);
        }
    }
}

/// `[u32 LE count] (repeated: [u32 LE len] [len bytes UTF-8 path])`
///
/// `OsStr::as_bytes()` would be Unix-only, so we cross-platform-encode
/// each path as UTF-8 via `Path::to_string_lossy()`. A non-UTF-8 Unix
/// path takes a lossy round-trip — acceptable since file managers
/// always pass well-formed UTF-8 in practice and the engine's audio
/// decoders won't open the result anyway.
fn write_paths<W: Write>(stream: &mut W, paths: &[PathBuf]) -> io::Result<()> {
    let count = u32::try_from(paths.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "too many paths in single batch",
        )
    })?;
    stream.write_all(&count.to_le_bytes())?;
    for path in paths {
        let lossy = path.to_string_lossy();
        let bytes = lossy.as_bytes();
        let len = u32::try_from(bytes.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path exceeds 4 GiB"))?;
        stream.write_all(&len.to_le_bytes())?;
        stream.write_all(bytes)?;
    }
    Ok(())
}

fn read_paths<R: Read>(stream: &mut R) -> io::Result<Vec<PathBuf>> {
    let mut count_buf = [0u8; 4];
    stream.read_exact(&mut count_buf)?;
    let count = u32::from_le_bytes(count_buf) as usize;
    // Sanity cap — a single secondary handing us 100k+ paths is almost
    // certainly a malformed frame, not a real "open with" batch.
    if count > 4096 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "path count too high",
        ));
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;
        if len > 64 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "path length too high",
            ));
        }
        let mut bytes = vec![0u8; len];
        stream.read_exact(&mut bytes)?;
        let s = String::from_utf8(bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "path is not valid UTF-8"))?;
        out.push(PathBuf::from(s));
    }
    Ok(out)
}

/// RAII guard that removes the Unix socket file when the primary
/// process shuts down cleanly. Crash cleanup on Unix is handled by the
/// next primary's `ListenerOptions::reclaim_name(true)` default. On
/// Windows the guard carries no state — named pipes evaporate with
/// the bound process.
pub struct PrimaryGuard {
    #[cfg(unix)]
    path: PathBuf,
}

impl Drop for PrimaryGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Collect file path args from `std::env::args_os()`, skipping argv[0].
/// Absolutises paths so the primary's working directory doesn't matter
/// — file managers pass absolute paths via `%F` already, but
/// `oneamp foo.mp3` on the command line works too.
pub fn collect_arg_paths() -> Vec<PathBuf> {
    std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .map(absolutise)
        .collect()
}

fn absolutise(p: PathBuf) -> PathBuf {
    if p.is_absolute() {
        return p;
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(&p),
        Err(_) => p,
    }
}
