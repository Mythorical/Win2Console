//! "Start with Windows": a per-user Task Scheduler task with a logon trigger. Unlike the `Run` key (which
//! explorer processes late, after its own startup), the task starts as soon as the user signs in.
//! Creating a task limited to the current user's own logon needs no admin rights.

use std::io;
use std::path::Path;

const TASK: &str = "Win2Console";

/// Whether Win2Console starts at sign-in (task, or the older `Run` key entry).
pub fn enabled() -> bool {
    imp::task_exists() || crate::registry::run_at_login_enabled()
}

/// Registers (`Some(exe)`) or removes (`None`) the startup task. Also drops the old `Run` key entry.
pub fn set(exe: Option<&Path>) -> io::Result<()> {
    match exe {
        Some(exe) => imp::register(exe)?,
        None => imp::delete()?,
    }
    let _ = crate::registry::set_run_at_login(None);
    Ok(())
}

#[cfg(windows)]
mod imp {
    use super::TASK;
    use std::io;
    use std::path::Path;
    use windows::Win32::Foundation::VARIANT_FALSE;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
    use windows::Win32::System::TaskScheduler::{
        IExecAction, ILogonTrigger, ITaskFolder, ITaskService, TASK_ACTION_EXEC, TASK_CREATE_OR_UPDATE,
        TASK_INSTANCES_IGNORE_NEW, TASK_LOGON_INTERACTIVE_TOKEN, TASK_TRIGGER_LOGON, TaskScheduler,
    };
    use windows::Win32::System::Variant::VARIANT;
    use windows::core::{BSTR, Interface};

    fn err(e: windows::core::Error) -> io::Error {
        io::Error::other(e.message())
    }

    fn service() -> windows::core::Result<ITaskService> {
        // SAFETY: COM is initialised for this thread (repeated init is harmless), then plain COM calls.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let service: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)?;
            let none = VARIANT::default();
            service.Connect(&none, &none, &none, &none)?;
            Ok(service)
        }
    }

    fn root() -> windows::core::Result<ITaskFolder> {
        // SAFETY: COM call on a connected service.
        unsafe { service()?.GetFolder(&BSTR::from("\\")) }
    }

    fn user() -> String {
        let var = |k| std::env::var(k).unwrap_or_default();
        format!("{}\\{}", var("USERDOMAIN"), var("USERNAME"))
    }

    pub fn task_exists() -> bool {
        // SAFETY: COM call on a valid folder.
        root().is_ok_and(|f| unsafe { f.GetTask(&BSTR::from(TASK)) }.is_ok())
    }

    pub fn register(exe: &Path) -> io::Result<()> {
        let user = BSTR::from(user());
        // SAFETY: COM calls on interfaces obtained from the Task Scheduler service.
        unsafe {
            let service = service().map_err(err)?;
            let folder = service.GetFolder(&BSTR::from("\\")).map_err(err)?;
            let none = VARIANT::default();
            let task = service.NewTask(0).map_err(err)?;

            let trigger: ILogonTrigger = task.Triggers().map_err(err)?.Create(TASK_TRIGGER_LOGON).map_err(err)?.cast().map_err(err)?;
            trigger.SetUserId(&user).map_err(err)?;

            let principal = task.Principal().map_err(err)?;
            principal.SetUserId(&user).map_err(err)?;
            principal.SetLogonType(TASK_LOGON_INTERACTIVE_TOKEN).map_err(err)?;

            let settings = task.Settings().map_err(err)?;
            settings.SetDisallowStartIfOnBatteries(VARIANT_FALSE).map_err(err)?;
            settings.SetStopIfGoingOnBatteries(VARIANT_FALSE).map_err(err)?;
            settings.SetExecutionTimeLimit(&BSTR::from("PT0S")).map_err(err)?;
            settings.SetMultipleInstances(TASK_INSTANCES_IGNORE_NEW).map_err(err)?;
            // Tasks default to below-normal priority, which games launched from the shell would inherit.
            settings.SetPriority(4).map_err(err)?;

            let action: IExecAction = task.Actions().map_err(err)?.Create(TASK_ACTION_EXEC).map_err(err)?.cast().map_err(err)?;
            action.SetPath(&BSTR::from(exe.display().to_string())).map_err(err)?;
            if let Some(dir) = exe.parent() {
                action.SetWorkingDirectory(&BSTR::from(dir.display().to_string())).map_err(err)?;
            }

            folder
                .RegisterTaskDefinition(
                    &BSTR::from(TASK),
                    &task,
                    TASK_CREATE_OR_UPDATE.0,
                    &none,
                    &none,
                    TASK_LOGON_INTERACTIVE_TOKEN,
                    &none,
                )
                .map_err(err)?;
        }
        Ok(())
    }

    pub fn delete() -> io::Result<()> {
        if !task_exists() {
            return Ok(());
        }
        // SAFETY: COM call on a valid folder.
        unsafe { root().map_err(err)?.DeleteTask(&BSTR::from(TASK), 0).map_err(err) }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;
    use std::path::Path;

    pub fn task_exists() -> bool {
        false
    }

    pub fn register(_exe: &Path) -> io::Result<()> {
        Err(crate::unsupported())
    }

    pub fn delete() -> io::Result<()> {
        Ok(())
    }
}
