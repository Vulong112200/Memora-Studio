//! AI worker process (workers/ai-worker/memora_worker.py, protocol v1, ADR 0002).
//! Started on demand by the job runner, restarted on the next job after a crash.

use crate::jobs::{Job, Outcome, Runner};
use crate::process::{JobObject, command};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const PROTOCOL: u64 = 1;
const HELLO_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a cancelled job may take to stop on its own (the worker checks between batches, normally
/// well under a second). Assumed value, not measured on the slowest model compiles.
const CANCEL_GRACE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct WorkerPaths {
    pub python: PathBuf,
    pub script: PathBuf,
}

/// Packaged runtime under the data root (ADR 0002), else the development venv in the repo.
pub fn resolve(data_root: Option<&Path>) -> Result<WorkerPaths, String> {
    let candidates = data_root
        .map(|r| {
            let base = r.join("runtime").join("ai-worker");
            (base.join("python").join("python.exe"), base.join("app").join("memora_worker.py"))
        })
        .into_iter()
        // ponytail: dev fallback to the repo venv; remove once the installer provisions runtime/ai-worker.
        .chain(std::iter::once({
            let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("..").join("workers").join("ai-worker");
            (repo.join(".venv").join("Scripts").join("python.exe"), repo.join("memora_worker.py"))
        }));
    for (python, script) in candidates {
        if python.exists() && script.exists() {
            return Ok(WorkerPaths { python, script });
        }
    }
    Err("AI worker runtime not found. Expected <data folder>\\runtime\\ai-worker\\ (python\\python.exe and app\\memora_worker.py).".into())
}

pub enum Recv {
    Msg(Value),
    Timeout,
    Exited,
}

pub struct Worker {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Option<String>>,
    pub hello: Value,
}

impl Worker {
    pub fn spawn(paths: &WorkerPaths, job_object: &JobObject) -> Result<Worker, String> {
        let mut cmd = command(&paths.python);
        cmd.arg("-u").arg(&paths.script).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = crate::process::spawn(&mut cmd, job_object).map_err(|e| format!("Cannot start AI worker ({}): {e}", paths.python.display()))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                tracing::warn!(target: "ai_worker", "{line}");
            }
        });
        let (tx, rx) = channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(Some(line)).is_err() {
                    return;
                }
            }
            let _ = tx.send(None);
        });
        let mut w = Worker { child, stdin, rx, hello: Value::Null };
        match w.recv(HELLO_TIMEOUT) {
            Recv::Msg(h) if h["type"] == "hello" && h["protocol"] == PROTOCOL => {
                tracing::info!(hello = %h, "AI worker started");
                w.hello = h;
                Ok(w)
            }
            Recv::Msg(h) => {
                w.kill();
                Err(format!("AI worker protocol mismatch (expected {PROTOCOL}): {h}"))
            }
            Recv::Timeout => {
                w.kill();
                Err("AI worker did not start within 60 s.".into())
            }
            Recv::Exited => Err("AI worker exited during start-up (see log).".into()),
        }
    }

    pub fn send(&mut self, msg: &Value) -> Result<(), String> {
        writeln!(self.stdin, "{msg}").and_then(|_| self.stdin.flush()).map_err(|e| format!("AI worker pipe closed: {e}"))
    }

    pub fn recv(&mut self, timeout: Duration) -> Recv {
        match self.rx.recv_timeout(timeout) {
            Ok(Some(line)) => match serde_json::from_str(&line) {
                Ok(v) => Recv::Msg(v),
                Err(_) => {
                    tracing::warn!(target: "ai_worker", line, "non-JSON output ignored");
                    Recv::Timeout
                }
            },
            Ok(None) | Err(RecvTimeoutError::Disconnected) => Recv::Exited,
            Err(RecvTimeoutError::Timeout) => Recv::Timeout,
        }
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Job runner backed by the AI worker. `info` receives the latest `hello` (None when no worker).
pub fn runner(
    data_root: impl Fn() -> Option<PathBuf> + Send + 'static,
    job_object: Arc<JobObject>,
    info: Arc<Mutex<Option<Value>>>,
) -> Runner {
    let mut worker: Option<Worker> = None;
    Box::new(move |job: &Job, progress: &dyn Fn(u64, u64), cancelled: &dyn Fn() -> bool| {
        if worker.is_none() {
            match resolve(data_root().as_deref()).and_then(|p| Worker::spawn(&p, &job_object)) {
                Ok(w) => {
                    *info.lock().unwrap() = Some(w.hello.clone());
                    worker = Some(w);
                }
                Err(e) => return Outcome::Failed(e),
            }
        }
        let (outcome, alive) = run_on(worker.as_mut().unwrap(), job, progress, cancelled);
        if !alive {
            tracing::error!(job_id = job.id, "AI worker exited; it will be restarted for the next job");
            worker = None;
            *info.lock().unwrap() = None;
        }
        outcome
    })
}

/// Returns the outcome and whether the worker is still usable.
pub(crate) fn run_on(w: &mut Worker, job: &Job, progress: &dyn Fn(u64, u64), cancelled: &dyn Fn() -> bool) -> (Outcome, bool) {
    let id = job.id.to_string();
    let start = json!({"type": "job.start", "id": id, "kind": job.kind, "params": job.params});
    if let Err(e) = w.send(&start) {
        return (Outcome::Failed(e), false);
    }
    let mut cancel_sent: Option<Instant> = None;
    loop {
        match cancel_sent {
            None if cancelled() => {
                cancel_sent = Some(Instant::now());
                if let Err(e) = w.send(&json!({"type": "job.cancel", "id": id})) {
                    return (Outcome::Failed(e), false);
                }
            }
            // Stuck in one long native call (e.g. a model compile): stop the process, restart on the next job.
            Some(t) if t.elapsed() > CANCEL_GRACE => {
                tracing::warn!(job_id = job.id, "AI worker ignored cancel; stopping it");
                w.kill();
                return (Outcome::Cancelled, false);
            }
            _ => {}
        }
        let m = match w.recv(Duration::from_millis(100)) {
            Recv::Timeout => continue,
            Recv::Exited => {
                let code = w.child.wait().ok().and_then(|s| s.code());
                return (Outcome::Failed(format!("AI worker stopped unexpectedly (exit code {code:?}).")), false);
            }
            Recv::Msg(m) => m,
        };
        if m["id"] != id.as_str() {
            continue;
        }
        match m["type"].as_str() {
            Some("job.progress") => progress(m["done"].as_u64().unwrap_or(0), m["total"].as_u64().unwrap_or(0)),
            Some("job.result") => return (Outcome::Completed(m["result"].clone()), true),
            Some("job.cancelled") => return (Outcome::Cancelled, true),
            Some("job.error") => return (Outcome::Failed(m["error"].as_str().unwrap_or("job failed").into()), true),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    //! Integration tests against the real Python worker. Skipped when the dev venv is missing.
    use super::*;
    use crate::jobs::{JobManager, Status};
    use std::sync::mpsc::channel;

    fn manager() -> Option<(JobManager, std::sync::mpsc::Receiver<Job>, Arc<Mutex<Option<Value>>>)> {
        if resolve(None).is_err() {
            eprintln!("skipped: workers/ai-worker/.venv not found");
            return None;
        }
        let info = Arc::new(Mutex::new(None));
        let r = runner(|| None, Arc::new(JobObject::new().unwrap()), info.clone());
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        Some((JobManager::start(r, Box::new(move |j| tx.lock().unwrap().send(j.clone()).unwrap())), rx, info))
    }

    fn wait_final(rx: &std::sync::mpsc::Receiver<Job>, id: u64) -> Job {
        loop {
            let j = rx.recv_timeout(Duration::from_secs(120)).expect("job update");
            if j.id == id && !matches!(j.status, Status::Queued | Status::Running) {
                return j;
            }
        }
    }

    #[test]
    fn worker_lifecycle_end_to_end() {
        let Some((m, rx, info)) = manager() else { return };

        let id = m.submit("diagnostic.steps", json!({"steps": 5})).id;
        let j = wait_final(&rx, id);
        assert_eq!((j.status, j.done, j.total), (Status::Completed, 5, 5), "{j:?}");
        assert_eq!(info.lock().unwrap().as_ref().unwrap()["protocol"], PROTOCOL);

        let id = m.submit("diagnostic.steps", json!({"steps": 10000})).id;
        while m.list().iter().find(|j| j.id == id).unwrap().done == 0 {
            thread::sleep(Duration::from_millis(10));
        }
        m.cancel(id).unwrap();
        assert_eq!(wait_final(&rx, id).status, Status::Cancelled);

        // A job that never checks for cancellation is stopped after the grace period; the next job restarts the worker.
        let id = m.submit("diagnostic.block", json!({"seconds": 60})).id;
        while m.list().iter().find(|j| j.id == id).unwrap().status != Status::Running {
            thread::sleep(Duration::from_millis(10));
        }
        thread::sleep(Duration::from_millis(500));
        let started = Instant::now();
        m.cancel(id).unwrap();
        assert_eq!(wait_final(&rx, id).status, Status::Cancelled);
        assert!(started.elapsed() < CANCEL_GRACE + Duration::from_secs(5), "{:?}", started.elapsed());
        assert!(info.lock().unwrap().is_none(), "stuck worker was stopped");

        let id = m.submit("no.such.kind", Value::Null).id;
        assert!(wait_final(&rx, id).error.unwrap().contains("unknown job kind"));

        let id = m.submit("diagnostic.crash", Value::Null).id;
        let j = wait_final(&rx, id);
        assert_eq!(j.status, Status::Failed);
        assert!(j.error.unwrap().contains("stopped unexpectedly"));
        assert!(info.lock().unwrap().is_none());

        let id = m.submit("validate_devices", Value::Null).id;
        let j = wait_final(&rx, id);
        assert_eq!(j.status, Status::Completed, "worker restarted after crash: {j:?}");
        let devices = j.result.unwrap()["devices"].as_array().unwrap().clone();
        assert!(devices.iter().any(|d| d["device"] == "CPU" && d["ok"] == true), "CPU must validate: {devices:?}");
        println!("{}", serde_json::to_string_pretty(&devices).unwrap());
    }
}
