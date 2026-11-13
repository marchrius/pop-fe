//! Process-group helpers so cancellation also terminates subprocess descendants.
use std::process::{Child, Command};

pub(crate) fn isolate(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
}

pub(crate) fn terminate(child: &mut Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut taskkill = Command::new("taskkill");
        taskkill.creation_flags(0x08000000);
        let _ = taskkill
            .args(["/F", "/T", "/PID", &child.id().to_string()])
            .status();
    }
    #[cfg(unix)]
    {
        let process_group = -(child.id() as libc::pid_t);
        // The child is made its own process-group leader by `isolate`.
        unsafe {
            libc::kill(process_group, libc::SIGKILL);
        }
    }
    let _ = child.kill();
}
