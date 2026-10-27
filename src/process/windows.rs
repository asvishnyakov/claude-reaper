use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_PARAMETER, FILETIME, GetLastError, HANDLE, STILL_ACTIVE,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::Identity;
use super::environment::owned_by_current_user;

/// A process opened for querying, closed when dropped. While it is open, the process's object
/// stays alive and its PID cannot be given to another process.
struct ProcessHandle(HANDLE);

impl ProcessHandle {
    /// `Err` carries `GetLastError`: `ERROR_INVALID_PARAMETER` when no process has that PID.
    fn open(pid: u32) -> Result<Self, u32> {
        // SAFETY: plain values in; a null handle is never wrapped.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            // SAFETY: no argument; read right after the call that failed.
            return Err(unsafe { GetLastError() });
        }
        Ok(Self(handle))
    }

    /// `false` once the process has exited, even if a handle keeps its object alive; `None` when
    /// the exit code cannot be read.
    fn is_running(&self) -> Option<bool> {
        let mut exit_code = 0;
        // SAFETY: the handle is open and `exit_code` outlives the call.
        let read = unsafe { GetExitCodeProcess(self.0, &mut exit_code) } != 0;
        read.then_some(exit_code == STILL_ACTIVE.cast_unsigned())
    }

    /// The creation FILETIME as one 64-bit number, which Claude Code records as `procStart` (P2).
    fn creation_time(&self) -> Option<u64> {
        let mut creation = FILETIME::default();
        let (mut exit, mut kernel, mut user) = (creation, creation, creation);
        // SAFETY: the handle is open and all four FILETIMEs outlive the call.
        let read =
            unsafe { GetProcessTimes(self.0, &mut creation, &mut exit, &mut kernel, &mut user) }
                != 0;
        read.then(|| (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        // SAFETY: the handle came from `OpenProcess` and is closed only here. A failure leaves
        // nothing to do.
        unsafe { CloseHandle(self.0) };
    }
}

pub fn identity(pid: u32, proc_start: Option<&str>) -> Identity {
    let process = match ProcessHandle::open(pid) {
        Ok(process) => process,
        Err(ERROR_INVALID_PARAMETER) => return Identity::Gone,
        // Typically access denied: a process of another user, or a protected one.
        Err(_) => return Identity::Different,
    };
    match process.is_running() {
        Some(true) => {}
        Some(false) => return Identity::Gone,
        None => return Identity::Unverified,
    }
    // A session record can name any PID; only this user's own processes are ever ours to stop.
    match owned_by_current_user(pid) {
        Some(true) => {}
        Some(false) => return Identity::Different,
        None => return Identity::Unverified,
    }
    let Some(recorded) = proc_start else {
        return Identity::Unverified;
    };
    match process.creation_time() {
        Some(created) if created.to_string() == recorded => Identity::Same,
        Some(_) => Identity::Different,
        None => Identity::Unverified,
    }
}

/// What Claude Code would record as `procStart` for `pid`.
#[cfg(test)]
pub(crate) fn proc_start_of(pid: u32) -> String {
    ProcessHandle::open(pid)
        .expect("the process can be opened")
        .creation_time()
        .expect("its times can be read")
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::thread::sleep;
    use std::time::Duration;

    use super::*;

    #[test]
    fn own_process_with_its_own_start_is_same() {
        let id = std::process::id();

        assert_eq!(identity(id, Some(&proc_start_of(id))), Identity::Same);
    }

    #[test]
    fn own_process_with_a_wrong_start_is_different() {
        assert_eq!(identity(std::process::id(), Some("1")), Identity::Different);
    }

    #[test]
    fn own_process_without_recorded_start_is_unverified() {
        assert_eq!(identity(std::process::id(), None), Identity::Unverified);
    }

    #[test]
    fn exited_child_whose_handle_is_held_is_gone() {
        let mut child = Command::new("cmd").args(["/c", "exit 0"]).spawn().unwrap();
        while child.try_wait().unwrap().is_none() {
            sleep(Duration::from_millis(10));
        }

        // `child` still holds its handle, which keeps the exited process's object alive.
        assert_eq!(identity(child.id(), None), Identity::Gone);
    }

    #[test]
    fn system_process_is_not_same() {
        assert_ne!(identity(4, None), Identity::Same);
    }
}
