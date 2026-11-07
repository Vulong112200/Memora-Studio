//! Spike 1b-3: Rust orchestration of the Python worker and FFmpeg (Tauri-agnostic).
//!
//! Usage:
//!   orchestrator-spike <python.exe> <worker.py>          run all scenarios, exit 1 on failure
//!   orchestrator-spike <python.exe> <worker.py> hold     spawn worker + ffmpeg, print PIDs, sleep
//!                                                        (used to test kill-on-close from outside)

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

/// Windows Job Object: every assigned process is killed when the last job handle closes,
/// i.e. when this process exits for any reason (including being force-killed).
mod job {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::*;

    pub struct Job(HANDLE);

    impl Job {
        pub fn new() -> Job {
            unsafe {
                let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                assert!(!h.is_null(), "CreateJobObjectW failed");
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of_val(&info) as u32,
                );
                assert!(ok != 0, "SetInformationJobObject failed");
                Job(h)
            }
        }

        pub fn assign(&self, child: &std::process::Child) {
            let ok = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) };
            assert!(ok != 0, "AssignProcessToJobObject failed");
        }
    }
}

/// Spawn a child with piped stdin/stdout, put it in the job, and forward stdout lines to a channel.
/// `None` on the channel means the child closed stdout (exited or crashed).
fn spawn(job: &job::Job, cmd: &mut Command) -> (Child, ChildStdin, Receiver<Option<String>>) {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn failed");
    job.assign(&child);
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(l) => {
                    if tx.send(Some(l)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(None);
    });
    (child, stdin, rx)
}

struct Worker {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Option<String>>,
}

impl Worker {
    fn start(job: &job::Job, py: &str, script: &str) -> (Worker, Value, Duration) {
        let t = Instant::now();
        let (child, stdin, rx) = spawn(job, Command::new(py).args(["-u", script]));
        let mut w = Worker { child, stdin, rx };
        let hello = w.recv(Duration::from_secs(60)).expect("no hello");
        (w, hello, t.elapsed())
    }

    fn send(&mut self, v: Value) {
        writeln!(self.stdin, "{v}").and_then(|_| self.stdin.flush()).expect("write to worker");
    }

    fn recv(&mut self, timeout: Duration) -> Result<Value, &'static str> {
        match self.rx.recv_timeout(timeout) {
            Ok(Some(line)) => serde_json::from_str(&line).map_err(|_| "invalid json from worker"),
            Ok(None) | Err(RecvTimeoutError::Disconnected) => Err("worker exited"),
            Err(RecvTimeoutError::Timeout) => Err("timeout"),
        }
    }
}

/// Run one job to completion. Optionally send `job.cancel` after N progress events and/or a
/// `health` probe after the first progress event. Returns (progress count, final msg, cancel ms, health reply).
fn run_job(w: &mut Worker, msg: Value, cancel_after: Option<usize>, probe_health: bool)
    -> (usize, Value, Option<u128>, Option<Value>) {
    let id = msg["id"].clone();
    w.send(msg);
    let (mut progress, mut cancel_at, mut health) = (0, None, None);
    loop {
        let m = match w.recv(Duration::from_secs(60)) {
            Ok(m) => m,
            Err(e) => return (progress, json!({"type": "recv_error", "error": e}), None, health),
        };
        match m["type"].as_str() {
            Some("job.progress") => {
                progress += 1;
                if probe_health && progress == 1 {
                    w.send(json!({"type": "health", "id": "busy-probe"}));
                }
                if Some(progress) == cancel_after {
                    cancel_at = Some(Instant::now());
                    w.send(json!({"type": "job.cancel", "id": id}));
                }
            }
            Some("health.ok") => health = Some(m),
            Some("job.result" | "job.cancelled" | "job.error") => {
                return (progress, m, cancel_at.map(|t| t.elapsed().as_millis()), health)
            }
            _ => {}
        }
    }
}

fn ffmpeg_path() -> PathBuf {
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|d| d.join("ffmpeg.exe"))
        .find(|p| p.exists());
    on_path.unwrap_or_else(|| {
        PathBuf::from(std::env::var("LOCALAPPDATA").unwrap()).join(r"Microsoft\WinGet\Links\ffmpeg.exe")
    })
}

/// Long synthetic encode with machine-readable progress on stdout.
fn ffmpeg_cmd(seconds: u32) -> Command {
    let mut c = Command::new(ffmpeg_path());
    c.args(["-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size=1920x1080:rate=30:duration={seconds}"))
        .args(["-c:v", "libx264", "-preset", "medium", "-f", "null", "-"]);
    c
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: orchestrator-spike <python.exe> <worker.py> [hold]");
        std::process::exit(2);
    }
    let (py, script) = (&args[1], &args[2]);
    let job = job::Job::new();

    if args.get(3).map(String::as_str) == Some("hold") {
        let (w, _, _) = Worker::start(&job, py, script);
        let (ff, _, _) = spawn(&job, &mut ffmpeg_cmd(3600));
        println!("PIDS orchestrator={} worker={} ffmpeg={}", std::process::id(), w.child.id(), ff.id());
        std::io::stdout().flush().unwrap();
        thread::sleep(Duration::from_secs(600));
        return;
    }

    let mut failures = 0;
    let mut check = |name: &str, ok: bool, detail: String| {
        println!("[{}] {name}: {detail}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            failures += 1;
        }
    };

    // 1. Spawn + handshake
    let (mut w, hello, startup) = Worker::start(&job, py, script);
    check(
        "handshake",
        hello["type"] == "hello" && hello["protocol"] == 1,
        format!("{} ms total (worker-reported {} ms); {hello}", startup.as_millis(), hello["startup_ms"]),
    );

    // 2. Job with progress + health probe while busy
    let (p, end, _, health) =
        run_job(&mut w, json!({"type": "job.start", "id": "j1", "kind": "steps", "steps": 10}), None, true);
    check("progress + result", p == 10 && end["type"] == "job.result", format!("{p} progress events, {end}"));
    check("health while busy", health.is_some(), format!("{health:?}"));

    // 3. Cancellation mid-job
    let (_, end, ms, _) =
        run_job(&mut w, json!({"type": "job.start", "id": "j2", "kind": "steps", "steps": 1000}), Some(3), false);
    check("cancel", end["type"] == "job.cancelled", format!("acknowledged after {ms:?} ms; {end}"));

    // 3b. Real OpenVINO inference inside the worker (lazy import after handshake), per device
    for (dev, cancel) in [("CPU", None), ("GPU", Some(5)), ("NPU", None)] {
        let msg = json!({"type": "job.start", "id": format!("ov-{dev}"), "kind": "ov_infer",
                         "device": dev, "iterations": if cancel.is_some() { 100000 } else { 50 }});
        let (p, end, ms, _) = run_job(&mut w, msg, cancel, false);
        let want = if cancel.is_some() { "job.cancelled" } else { "job.result" };
        check(&format!("openvino job {dev}"), end["type"] == want,
              format!("{p} progress, cancel ack {ms:?} ms; {end}"));
    }

    // 4. Worker crash -> detect -> restart
    w.send(json!({"type": "job.start", "id": "j3", "kind": "crash"}));
    let err = w.recv(Duration::from_secs(10)).err();
    let status = w.child.wait().ok().and_then(|s| s.code());
    check("crash detected", err == Some("worker exited"), format!("recv={err:?}, exit code={status:?}"));
    let (mut w, _, restart) = Worker::start(&job, py, script);
    w.send(json!({"type": "health", "id": "h1"}));
    let health = w.recv(Duration::from_secs(10));
    check(
        "restart + health",
        matches!(&health, Ok(m) if m["type"] == "health.ok"),
        format!("restart {} ms; {health:?}", restart.as_millis()),
    );

    // 5. FFmpeg child: progress parsing + graceful cancel ('q' on stdin)
    let (mut ff, mut ff_in, ff_rx) = spawn(&job, &mut ffmpeg_cmd(600));
    let (t0, mut updates, mut last_us) = (Instant::now(), 0, 0i64);
    while t0.elapsed() < Duration::from_secs(3) {
        if let Ok(Some(line)) = ff_rx.recv_timeout(Duration::from_millis(200)) {
            if let Some(v) = line.strip_prefix("out_time_us=") {
                updates += 1;
                last_us = v.trim().parse().unwrap_or(last_us);
            }
        }
    }
    let tq = Instant::now();
    let _ = ff_in.write_all(b"q").and_then(|_| ff_in.flush());
    let mut code = None;
    while tq.elapsed() < Duration::from_secs(10) {
        if let Ok(Some(s)) = ff.try_wait() {
            code = s.code();
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    if code.is_none() {
        let _ = ff.kill();
    }
    check(
        "ffmpeg progress + cancel",
        updates > 0 && code.is_some(),
        format!(
            "{updates} progress updates, encoded {:.1}s of video in 3s, exit {code:?} {} ms after 'q'",
            last_us as f64 / 1e6,
            tq.elapsed().as_millis()
        ),
    );

    // 6. Graceful shutdown
    w.send(json!({"type": "shutdown"}));
    let t = Instant::now();
    let code = w.child.wait().ok().and_then(|s| s.code());
    check("shutdown", code == Some(0), format!("exit {code:?} after {} ms", t.elapsed().as_millis()));

    std::process::exit(if failures == 0 { 0 } else { 1 });
}
