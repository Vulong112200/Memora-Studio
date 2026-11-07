//! Child-process helpers: every child joins a Job Object so it dies with the app (even on a crash
//! or forced kill), and is started without a console window.

use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command};
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
