//! Child-process helpers: every child joins a Job Object so it dies with the app (even on a crash
//! or forced kill), and is started without a console window.

use std::io::Read;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Output};
use std::thread;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows::core::PCWSTR;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The handle is intentionally never closed: Windows closes it when the app process ends,
/// which kills every assigned child.
pub struct JobObject(HANDLE);

// SAFETY: a job object handle may be used from any thread.
unsafe impl Send for JobObject {}
unsafe impl Sync for JobObject {}

impl JobObject {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let h = CreateJobObjectW(None, PCWSTR::null()).map_err(|e| format!("CreateJobObject failed: {e}"))?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                size_of_val(&info) as u32,
            )
            .map_err(|e| format!("SetInformationJobObject failed: {e}"))?;
            Ok(Self(h))
        }
    }

    pub fn assign(&self, child: &Child) -> Result<(), String> {
        unsafe { AssignProcessToJobObject(self.0, HANDLE(child.as_raw_handle())) }
            .map_err(|e| format!("AssignProcessToJobObject failed: {e}"))
    }
}

/// `Command` that does not open a console window when started from the GUI app.
pub fn command(program: &Path) -> Command {
    let mut c = Command::new(program);
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

/// Starts `cmd` inside the Job Object. A child that cannot join it is killed at once, so no process
/// ever runs outside the app's control (it would survive an app crash).
pub fn spawn(cmd: &mut Command, job: &JobObject) -> Result<Child, String> {
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    if let Err(e) = job.assign(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(e);
    }
    Ok(child)
}

fn drain<R: Read + Send + 'static>(r: Option<R>) -> Option<thread::JoinHandle<Vec<u8>>> {
    r.map(|mut r| {
        thread::spawn(move || {
            let mut b = vec![];
            let _ = r.read_to_end(&mut b);
            b
        })
    })
}

/// Like `wait_with_output`, but the child is killed when `cancelled()` turns true ("cancelled") or when
/// it runs longer than `timeout` (a hung tool must not block the one-at-a-time job queue).
pub fn wait_output(mut child: Child, timeout: Duration, cancelled: &dyn Fn() -> bool) -> Result<Output, String> {
    let (out, err) = (drain(child.stdout.take()), drain(child.stderr.take()));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        let stop = if cancelled() {
            Some("cancelled".to_string())
        } else if Instant::now() >= deadline {
            Some(format!("timed out after {} s", timeout.as_secs()))
        } else {
            None
        };
        if let Some(why) = stop {
            let _ = child.kill();
            let _ = child.wait();
            return Err(why);
        }
        thread::sleep(Duration::from_millis(50));
    };
    let join = |h: Option<thread::JoinHandle<Vec<u8>>>| h.map(|h| h.join().unwrap_or_default()).unwrap_or_default();
    Ok(Output { status, stdout: join(out), stderr: join(err) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    fn ping(seconds: u32) -> Command {
        // ping.exe ships with Windows: -n N waits about N-1 seconds.
        let mut c = command(Path::new("ping.exe"));
        c.args(["-n", &seconds.to_string(), "127.0.0.1"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        c
    }

    #[test]
    fn wait_output_returns_output_times_out_and_cancels() {
        let job = JobObject::new().unwrap();
        let out = wait_output(spawn(&mut ping(1), &job).unwrap(), Duration::from_secs(30), &|| false).unwrap();
        assert!(out.status.success() && !out.stdout.is_empty());

        let started = Instant::now();
        let e = wait_output(spawn(&mut ping(30), &job).unwrap(), Duration::from_millis(300), &|| false).unwrap_err();
        assert!(e.contains("timed out") && started.elapsed() < Duration::from_secs(5), "{e}");

        let e = wait_output(spawn(&mut ping(30), &job).unwrap(), Duration::from_secs(30), &|| true).unwrap_err();
        assert_eq!(e, "cancelled");
    }
}
