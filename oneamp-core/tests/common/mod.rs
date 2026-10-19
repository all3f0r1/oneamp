//! Shared test rig: generated WAV media, an engine on a memory (pull)
//! output, and a scriptable local HTTP server.
#![allow(dead_code)]

use oneamp_core::output::pull::{PullConfig, PullHandle, pull_output};
use oneamp_core::{AudioCommand, AudioEngine, AudioEvent, EngineConfig, PlayerState};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 16-bit PCM WAV whose sample values encode (track, frame, channel),
/// so any loss, duplication or reordering is visible in the output.
pub fn wav(tag: u16, rate: u32, channels: u16, frames: usize) -> (Vec<u8>, Vec<f32>) {
    let mut pcm = Vec::with_capacity(frames * channels as usize);
    for f in 0..frames {
        for c in 0..channels {
            // Distinct per (tag, frame, channel), aperiodic, well inside
            // full scale: any loss, repeat or shift is detectable.
            let mut h = (tag as u64) << 48 ^ (f as u64) << 8 ^ c as u64;
            h ^= h >> 33;
            h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
            h ^= h >> 33;
            // Never 0: silence in an output is always padding/underrun.
            let v = (h % 20000) as i16 - 10000;
            pcm.push(if v >= 0 { v + 1 } else { v });
        }
    }
    let data_len = (pcm.len() * 2) as u32;
    let mut b = Vec::new();
    b.extend(b"RIFF");
    b.extend((36 + data_len).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(channels.to_le_bytes());
    b.extend(rate.to_le_bytes());
    b.extend((rate * channels as u32 * 2).to_le_bytes());
    b.extend((channels * 2).to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(data_len.to_le_bytes());
    for s in &pcm {
        b.extend(s.to_le_bytes());
    }
    let f32s = pcm.iter().map(|&s| s as f32 / 32768.0).collect();
    (b, f32s)
}

/// Write a WAV to a temp file; returns its path and samples.
pub fn wav_file(
    name: &str,
    tag: u16,
    rate: u32,
    channels: u16,
    frames: usize,
) -> (std::path::PathBuf, Vec<f32>) {
    let dir = std::env::temp_dir().join(format!("oneamp-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let (bytes, samples) = wav(tag, rate, channels, frames);
    std::fs::write(&path, bytes).unwrap();
    (path, samples)
}

/// An engine on a pull output, plus a collector thread acting as the
/// device.
pub struct Rig {
    pub engine: AudioEngine,
    pub handle: PullHandle,
    pub collected: Arc<Mutex<Vec<f32>>>,
    pub events: Vec<AudioEvent>,
    stop: Arc<AtomicBool>,
    collector: Option<std::thread::JoinHandle<()>>,
}

/// How fast the fake device consumes.
#[derive(Clone, Copy)]
pub enum Pace {
    /// Everything as soon as it is queued (offline render).
    Free,
    /// Real time times this factor.
    RealTime(f64),
}

impl Rig {
    pub fn new(config: EngineConfig, pace: Pace) -> Self {
        Self::with_pull(config, PullConfig::default(), pace)
    }

    pub fn with_pull(config: EngineConfig, pull: PullConfig, pace: Pace) -> Self {
        let (backend, handle) = pull_output(pull);
        let engine = AudioEngine::with_backend(Box::new(backend), config).unwrap();
        let collected = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (h, c, s) = (handle.clone(), collected.clone(), stop.clone());
        let collector = std::thread::spawn(move || {
            let mut buf = vec![0.0f32; 16384];
            let start = Instant::now();
            let mut consumed_secs = 0.0f64;
            while !s.load(Ordering::Relaxed) {
                match pace {
                    Pace::Free => {
                        let n = h.read_available(&mut buf);
                        if n > 0 {
                            let ch = h.format().map_or(2, |f| f.1 as usize);
                            c.lock().unwrap().extend_from_slice(&buf[..n * ch]);
                        } else {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    }
                    Pace::RealTime(factor) => {
                        std::thread::sleep(Duration::from_millis(5));
                        let Some((rate, ch)) = h.format() else {
                            continue;
                        };
                        let due = start.elapsed().as_secs_f64() * factor;
                        let frames = ((due - consumed_secs) * rate as f64) as usize;
                        let frames = frames.min(buf.len() / ch as usize);
                        if frames == 0 {
                            continue;
                        }
                        let n = h.render(&mut buf[..frames * ch as usize]);
                        consumed_secs += frames as f64 / rate as f64;
                        let _ = n;
                        c.lock()
                            .unwrap()
                            .extend_from_slice(&buf[..frames * ch as usize]);
                    }
                }
            }
        });
        Self {
            engine,
            handle,
            collected,
            events: Vec::new(),
            stop,
            collector: Some(collector),
        }
    }

    pub fn send(&self, cmd: AudioCommand) -> oneamp_core::CommandId {
        self.engine.send(cmd).expect("queued")
    }

    /// Drain events until `pred` matches one (returned) or `timeout`.
    pub fn wait_for(
        &mut self,
        timeout: Duration,
        mut pred: impl FnMut(&AudioEvent) -> bool,
    ) -> AudioEvent {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "timed out; events so far: {:#?}",
                self.events
            );
            if let Some(e) = self
                .engine
                .recv_event_timeout(left.min(Duration::from_millis(50)))
            {
                self.events.push(e.clone());
                if pred(&e) {
                    return e;
                }
            }
        }
    }

    pub fn wait_state(&mut self, timeout: Duration, state: PlayerState) {
        let deadline = Instant::now() + timeout;
        while self.engine.snapshot().state != state {
            assert!(
                Instant::now() < deadline,
                "state {:?} never reached (now {:?})",
                state,
                self.engine.snapshot().state
            );
            while let Some(e) = self.engine.try_recv_event() {
                self.events.push(e);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    pub fn finished(&mut self, timeout: Duration) {
        self.wait_for(timeout, |e| matches!(e, AudioEvent::Finished));
    }

    pub fn take(&self) -> Vec<f32> {
        std::mem::take(&mut *self.collected.lock().unwrap())
    }

    pub fn stop_collector(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(c) = self.collector.take() {
            c.join().unwrap();
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.stop_collector();
    }
}

/// Drop the limiter's leading silence and the drain padding: the audio
/// proper, for exact comparisons. `latency` in frames.
pub fn trim(out: &[f32], channels: usize, latency_frames: usize, len: usize) -> Vec<f32> {
    let start = latency_frames * channels;
    out[start.min(out.len())..(start + len).min(out.len())].to_vec()
}

/// The limiter lookahead at `rate` (2 ms rounded, minus one).
pub fn latency(rate: u32) -> usize {
    ((rate as f32 * 0.002).round() as usize).max(2) - 1
}

/// The output without silent samples: what was actually played,
/// whatever underruns or padding surrounded it. Test signals never
/// contain an exact zero.
pub fn audible(out: &[f32]) -> Vec<f32> {
    out.iter().copied().filter(|&s| s != 0.0).collect()
}

/// Index of `needle` in `hay` (exact), for locating a known run.
pub fn find(hay: &[f32], needle: &[f32]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Threads of this process (Linux), for leak checks.
pub fn thread_count() -> usize {
    std::fs::read_dir("/proc/self/task")
        .map(|d| d.count())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------
// HTTP

/// What the server does with one request.
#[derive(Clone, Default)]
pub struct Behavior {
    /// Ignore `Range`, always answer 200 with the whole body.
    pub ignore_range: bool,
    /// Answer 206 with a broken `Content-Range`.
    pub bad_content_range: bool,
    /// Close the connection after this many body bytes, on the n-th
    /// request (1-based) — `(request, bytes)`.
    pub cut: Option<(usize, usize)>,
    /// Accept, read the request, never answer.
    pub hang: bool,
    /// Send headers then stall the body after this many bytes.
    pub stall_after: Option<usize>,
    /// Require this `Authorization` value or `?cap=` query; 401 else.
    pub require_token: Option<String>,
    /// 403 for every request.
    pub forbid: bool,
    /// Content type to announce.
    pub content_type: Option<String>,
    /// Redirect every request to this absolute URL.
    pub redirect_to: Option<String>,
    /// Serve as a live ICY stream with this metaint and title.
    pub icy: Option<(usize, String)>,
    /// After this many requests, the resource changes (new ETag, new
    /// bytes).
    pub change_after: Option<usize>,
}

#[derive(Default)]
pub struct ServerState {
    pub body: Vec<u8>,
    pub etag: String,
    pub behavior: Behavior,
    pub requests: Vec<String>,
}

pub struct TestServer {
    pub addr: String,
    pub state: Arc<Mutex<ServerState>>,
    pub count: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl TestServer {
    pub fn start(body: Vec<u8>, behavior: Behavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(ServerState {
            body,
            etag: "\"v1\"".into(),
            behavior,
            requests: Vec::new(),
        }));
        let count = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (st, cnt, stp) = (state.clone(), count.clone(), stop.clone());
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            while !stp.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((sock, _)) => {
                        sock.set_nonblocking(false).unwrap();
                        let (st, cnt, stp) = (st.clone(), cnt.clone(), stp.clone());
                        std::thread::spawn(move || serve(sock, st, cnt, stp));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(2)),
                }
            }
        });
        Self {
            addr,
            state,
            count,
            stop,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.addr, path)
    }

    pub fn requests(&self) -> Vec<String> {
        self.state.lock().unwrap().requests.clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn serve(
    mut sock: TcpStream,
    st: Arc<Mutex<ServerState>>,
    cnt: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
) {
    let mut reader = BufReader::new(sock.try_clone().unwrap());
    let mut head = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        if line == "\r\n" {
            break;
        }
        head.push_str(&line);
    }
    let n = cnt.fetch_add(1, Ordering::SeqCst) + 1;
    let (mut body, mut etag, b) = {
        let mut g = st.lock().unwrap();
        g.requests.push(head.clone());
        (g.body.clone(), g.etag.clone(), g.behavior.clone())
    };
    if b.change_after.is_some_and(|k| n > k) {
        etag = "\"v2\"".into();
        for x in body.iter_mut().skip(44) {
            *x = x.wrapping_add(1);
        }
    }
    let header = |name: &str| {
        head.lines()
            .find(|l| {
                l.to_ascii_lowercase()
                    .starts_with(&format!("{}:", name.to_ascii_lowercase()))
            })
            .map(|l| l.split_once(':').unwrap().1.trim().to_string())
    };
    if b.hang {
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
        }
        return;
    }
    if let Some(to) = &b.redirect_to {
        let _ = write!(
            sock,
            "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        return;
    }
    if b.forbid {
        let _ = write!(
            sock,
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        return;
    }
    if let Some(token) = &b.require_token {
        let path = head.lines().next().unwrap_or("");
        let ok = header("authorization").as_deref() == Some(token.as_str())
            || path.contains(&format!("cap={token}"));
        if !ok {
            let _ = write!(
                sock,
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            return;
        }
    }
    let ct = b.content_type.clone().unwrap_or_else(|| "audio/wav".into());
    if let Some((metaint, title)) = &b.icy {
        let _ = write!(
            sock,
            "HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nicy-metaint: {metaint}\r\nicy-name: test\r\nConnection: close\r\n\r\n"
        );
        let mut meta = format!("StreamTitle='{title}';").into_bytes();
        let blocks = meta.len().div_ceil(16);
        meta.resize(blocks * 16, 0);
        for chunk in body.chunks(*metaint) {
            if sock.write_all(chunk).is_err() {
                return;
            }
            if chunk.len() == *metaint {
                let _ = sock.write_all(&[blocks as u8]);
                let _ = sock.write_all(&meta);
            }
        }
        // A live stream does not end: hold the line until the client goes.
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
            if sock.write_all(&[]).is_err() {
                return;
            }
        }
        return;
    }
    let total = body.len();
    let range = header("range").and_then(|r| {
        let r = r.strip_prefix("bytes=")?;
        let (a, z) = r.split_once('-')?;
        let a: usize = a.parse().ok()?;
        let z: usize = if z.is_empty() {
            total - 1
        } else {
            z.parse().ok()?
        };
        Some((a, z.min(total - 1)))
    });
    let if_range_ok = header("if-range").is_none_or(|v| v == etag);
    let (status, slice, extra) = match range {
        Some((a, _)) if !b.ignore_range && a >= total => {
            let _ = write!(
                sock,
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            return;
        }
        Some((a, z)) if !b.ignore_range && if_range_ok => {
            let cr = if b.bad_content_range {
                "bytes x-y/z".to_string()
            } else {
                format!("bytes {a}-{z}/{total}")
            };
            (
                "206 Partial Content",
                &body[a..=z],
                format!("Content-Range: {cr}\r\n"),
            )
        }
        _ => ("200 OK", &body[..], String::new()),
    };
    let _ = write!(
        sock,
        "HTTP/1.1 {status}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nETag: {etag}\r\n{extra}Connection: close\r\n\r\n",
        slice.len()
    );
    let mut sent = 0;
    for chunk in slice.chunks(4096) {
        if let Some((req, at)) = b.cut
            && req == n
            && sent + chunk.len() > at
        {
            let _ = sock.write_all(&chunk[..at - sent]);
            let _ = sock.flush();
            return;
        }
        if let Some(at) = b.stall_after
            && sent + chunk.len() > at
        {
            let _ = sock.write_all(&chunk[..at - sent]);
            let _ = sock.flush();
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
            }
            return;
        }
        if sock.write_all(chunk).is_err() {
            return;
        }
        sent += chunk.len();
    }
    let _ = sock.flush();
    let mut sink = [0u8; 64];
    let _ = sock.set_read_timeout(Some(Duration::from_millis(50)));
    let _ = sock.read(&mut sink);
}
