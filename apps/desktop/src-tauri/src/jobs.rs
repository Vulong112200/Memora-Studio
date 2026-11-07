//! Background job manager: FIFO queue, one job at a time (resource policy: one heavy workload),
//! progress, cancellation, change notifications. Jobs live in memory for the session.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

/// Finished jobs kept for display; older ones are dropped.
const KEEP_FINISHED: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: u64,
    pub kind: String,
    pub params: Value,
    pub status: Status,
    pub done: u64,
    pub total: u64,
    pub result: Option<Value>,
    pub error: Option<String>,
}

pub enum Outcome {
    Completed(Value),
    Cancelled,
    Failed(String),
}

/// Executes one job. Gets a progress reporter `(done, total)` and a cancellation check.
pub type Runner = Box<dyn FnMut(&Job, &dyn Fn(u64, u64), &dyn Fn() -> bool) -> Outcome + Send>;
pub type Notify = Box<dyn Fn(&Job) + Send + Sync>;

#[derive(Default)]
struct Inner {
    jobs: Vec<Job>,
    next_id: u64,
    cancel: HashSet<u64>,
}

struct Shared {
    inner: Mutex<Inner>,
    wake: Condvar,
    notify: Notify,
}

#[derive(Clone)]
pub struct JobManager(Arc<Shared>);

impl JobManager {
    pub fn start(mut runner: Runner, notify: Notify) -> Self {
        let shared = Arc::new(Shared { inner: Mutex::default(), wake: Condvar::new(), notify });
        let s = shared.clone();
        thread::Builder::new()
            .name("job-executor".into())
            .spawn(move || loop {
                let job = {
                    let mut inner = s.inner.lock().unwrap();
                    loop {
                        if let Some(j) = inner.jobs.iter_mut().find(|j| j.status == Status::Queued) {
                            j.status = Status::Running;
                            break j.clone();
                        }
                        inner = s.wake.wait(inner).unwrap();
                    }
                };
                (s.notify)(&job);
                let id = job.id;
                let progress = |done, total| update(&s, id, |j| (j.done, j.total) = (done, total));
                let cancelled = || s.inner.lock().unwrap().cancel.contains(&id);
                let outcome = runner(&job, &progress, &cancelled);
                match &outcome {
                    Outcome::Failed(e) => tracing::warn!(job_id = id, kind = job.kind, error = %e, "job failed"),
                    Outcome::Cancelled => tracing::info!(job_id = id, kind = job.kind, "job cancelled"),
                    Outcome::Completed(_) => tracing::info!(job_id = id, kind = job.kind, "job completed"),
                }
                update(&s, id, |j| match outcome {
                    Outcome::Completed(v) => (j.status, j.result) = (Status::Completed, Some(v)),
                    Outcome::Cancelled => j.status = Status::Cancelled,
                    Outcome::Failed(e) => (j.status, j.error) = (Status::Failed, Some(e)),
                });
                let mut inner = s.inner.lock().unwrap();
                inner.cancel.remove(&id);
                prune(&mut inner.jobs);
            })
            .expect("spawn job executor");
        Self(shared)
    }

    pub fn submit(&self, kind: &str, params: Value) -> Job {
        let job = {
            let mut inner = self.0.inner.lock().unwrap();
            inner.next_id += 1;
            let job = Job {
                id: inner.next_id,
                kind: kind.into(),
                params,
                status: Status::Queued,
                done: 0,
                total: 0,
                result: None,
                error: None,
            };
            inner.jobs.push(job.clone());
            job
        };
        (self.0.notify)(&job);
        self.0.wake.notify_all();
        job
    }

    /// Queued jobs are cancelled immediately; running jobs are asked to stop at the next batch.
    pub fn cancel(&self, id: u64) -> Result<(), String> {
        let mut inner = self.0.inner.lock().unwrap();
        let job = inner.jobs.iter_mut().find(|j| j.id == id).ok_or(format!("Job {id} not found."))?;
        match job.status {
            Status::Queued => {
                job.status = Status::Cancelled;
                let snapshot = job.clone();
                drop(inner);
                (self.0.notify)(&snapshot);
                Ok(())
            }
            Status::Running => {
                inner.cancel.insert(id);
                Ok(())
            }
            _ => Err(format!("Job {id} has already finished.")),
        }
    }

    pub fn list(&self) -> Vec<Job> {
        self.0.inner.lock().unwrap().jobs.clone()
    }
}

fn update(s: &Shared, id: u64, f: impl FnOnce(&mut Job)) {
    let snapshot = {
        let mut inner = s.inner.lock().unwrap();
        let Some(job) = inner.jobs.iter_mut().find(|j| j.id == id) else { return };
        f(job);
        job.clone()
    };
    (s.notify)(&snapshot);
}

fn prune(jobs: &mut Vec<Job>) {
    let finished = |j: &Job| !matches!(j.status, Status::Queued | Status::Running);
    let excess = jobs.iter().filter(|j| finished(j)).count().saturating_sub(KEEP_FINISHED);
    let mut dropped = 0;
    jobs.retain(|j| {
        let drop_it = dropped < excess && finished(j);
        dropped += drop_it as usize;
        !drop_it
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    /// Fake runner: params {steps, fail} → steps of 20 ms, cancellable.
    fn fake() -> Runner {
        Box::new(|job, progress, cancelled| {
            let steps = job.params["steps"].as_u64().unwrap_or(3);
            for i in 1..=steps {
                if cancelled() {
                    return Outcome::Cancelled;
                }
                thread::sleep(Duration::from_millis(20));
                progress(i, steps);
            }
            match job.params["fail"].as_str() {
                Some(e) => Outcome::Failed(e.into()),
                None => Outcome::Completed(json!({"steps": steps})),
            }
        })
    }

    fn manager() -> (JobManager, Receiver<Job>) {
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        (JobManager::start(fake(), Box::new(move |j| tx.lock().unwrap().send(j.clone()).unwrap())), rx)
    }

    fn wait_final(rx: &Receiver<Job>, id: u64) -> Job {
        loop {
            let j = rx.recv_timeout(Duration::from_secs(5)).expect("job update");
            if j.id == id && !matches!(j.status, Status::Queued | Status::Running) {
                return j;
            }
        }
    }

    #[test]
    fn completes_with_progress() {
        let (m, rx) = manager();
        let id = m.submit("test", json!({"steps": 3})).id;
        let j = wait_final(&rx, id);
        assert_eq!((j.status, j.done, j.total), (Status::Completed, 3, 3));
        assert_eq!(j.result, Some(json!({"steps": 3})));
    }

    #[test]
    fn failure_is_reported() {
        let (m, rx) = manager();
        let id = m.submit("test", json!({"steps": 1, "fail": "boom"})).id;
        let j = wait_final(&rx, id);
        assert_eq!((j.status, j.error.as_deref()), (Status::Failed, Some("boom")));
    }

    #[test]
    fn runs_in_fifo_order_one_at_a_time() {
        let (m, rx) = manager();
        let a = m.submit("a", json!({"steps": 5})).id;
        let b = m.submit("b", json!({"steps": 1})).id;
        assert_eq!(m.list().iter().find(|j| j.id == b).unwrap().status, Status::Queued);
        let mut order = vec![];
        while order.len() < 2 {
            let j = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            if j.status == Status::Completed {
                order.push(j.id);
            }
        }
        assert_eq!(order, vec![a, b]);
    }

    #[test]
    fn cancel_queued_and_running() {
        let (m, rx) = manager();
        let running = m.submit("long", json!({"steps": 200})).id;
        let queued = m.submit("next", json!({"steps": 1})).id;
        m.cancel(queued).unwrap();
        assert_eq!(wait_final(&rx, queued).status, Status::Cancelled);
        while m.list().iter().find(|j| j.id == running).unwrap().done == 0 {
            thread::sleep(Duration::from_millis(5));
        }
        m.cancel(running).unwrap();
        let j = wait_final(&rx, running);
        assert_eq!(j.status, Status::Cancelled);
        assert!(j.done < 200);
        assert!(m.cancel(running).is_err(), "finished jobs cannot be cancelled");
        assert!(m.cancel(999).is_err());
    }

    #[test]
    fn prune_keeps_recent_finished_and_all_active() {
        let mk = |id, status| Job {
            id, kind: "k".into(), params: Value::Null, status, done: 0, total: 0, result: None, error: None,
        };
        let mut jobs: Vec<Job> = (1..=KEEP_FINISHED as u64 + 5).map(|i| mk(i, Status::Completed)).collect();
        jobs.insert(0, mk(0, Status::Queued));
        prune(&mut jobs);
        assert_eq!(jobs.len(), KEEP_FINISHED + 1);
        assert_eq!(jobs[0].status, Status::Queued);
        assert_eq!(jobs[1].id, 6, "oldest finished jobs dropped first");
    }
}
