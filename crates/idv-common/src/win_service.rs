//! Minimal Windows service host for the Center/Edge server binaries.
//!
//! The SCM entry point must be registered from the process main thread.  The
//! actual server runs on a worker thread with its own Tokio runtime; STOP /
//! SHUTDOWN requests terminate the worker process.  Running `service` from a
//! normal console falls back to running the worker directly so the same binary
//! is easy to smoke-test.

use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::System::Services::{
    RegisterServiceCtrlHandlerExW, SERVICE_ACCEPT_SHUTDOWN, SERVICE_ACCEPT_STOP,
    SERVICE_CONTROL_SHUTDOWN, SERVICE_CONTROL_STOP, SERVICE_RUNNING, SERVICE_START_PENDING,
    SERVICE_STATUS, SERVICE_STATUS_HANDLE, SERVICE_STOP_PENDING, SERVICE_STOPPED,
    SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS, SetServiceStatus, StartServiceCtrlDispatcherW,
};

type RunFn = Box<dyn FnOnce() -> anyhow::Result<()> + Send>;

static RUN: Mutex<Option<RunFn>> = Mutex::new(None);
static SERVICE_NAME: OnceLock<Vec<u16>> = OnceLock::new();
static STATUS_HANDLE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn set_status(state: u32, exit_code: u32, wait_hint: u32) {
    let handle = STATUS_HANDLE.load(Ordering::SeqCst);
    if handle.is_null() {
        return;
    }
    let accepted = if state == SERVICE_RUNNING {
        SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
    } else {
        0
    };
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: accepted,
        dwWin32ExitCode: exit_code,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: wait_hint,
    };
    unsafe {
        SetServiceStatus(handle, &status);
    }
}

unsafe extern "system" fn handler(
    control: u32,
    _event_type: u32,
    _event_data: *mut c_void,
    _context: *mut c_void,
) -> u32 {
    if control == SERVICE_CONTROL_STOP || control == SERVICE_CONTROL_SHUTDOWN {
        set_status(SERVICE_STOP_PENDING, 0, 5_000);
        // The async server has no cancellation hook yet; stopped services must
        // release their ports immediately, so terminate after reporting status.
        set_status(SERVICE_STOPPED, 0, 0);
        std::process::exit(0);
    }
    0
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut *mut u16) {
    let Some(name) = SERVICE_NAME.get() else {
        return;
    };
    let handle: SERVICE_STATUS_HANDLE =
        unsafe { RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(handler), std::ptr::null_mut()) };
    if handle.is_null() {
        return;
    }
    STATUS_HANDLE.store(handle, Ordering::SeqCst);
    set_status(SERVICE_START_PENDING, 0, 5_000);
    set_status(SERVICE_RUNNING, 0, 0);

    let run = RUN.lock().ok().and_then(|mut g| g.take());
    std::thread::spawn(move || {
        let result = match run {
            Some(f) => f(),
            None => Err(anyhow::anyhow!("service worker was not configured")),
        };
        match result {
            Ok(()) => {
                set_status(SERVICE_STOPPED, 0, 0);
                std::process::exit(0);
            }
            Err(err) => {
                eprintln!("idv service error: {err:#}");
                set_status(SERVICE_STOPPED, 1, 0);
                std::process::exit(1);
            }
        }
    });

    // ServiceMain must not return while the worker is alive; the control handler
    // exits the process on STOP, and the worker exits on fatal errors.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

/// Run `worker` as the named Windows service.
///
/// `service_name` must match the name used by `New-Service` / `sc create`.
pub fn run_service<F>(service_name: &str, worker: F) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()> + Send + 'static,
{
    *RUN.lock().map_err(|_| anyhow::anyhow!("service run lock poisoned"))? = Some(Box::new(worker));
    let name = wide(service_name);
    let _ = SERVICE_NAME.set(name.clone());

    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: name.as_ptr() as *mut u16,
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: std::ptr::null_mut(),
            lpServiceProc: None,
        },
    ];

    let ok = unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) };
    if ok != 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    // 1063 = ERROR_FAILED_SERVICE_CONTROLLER_CONNECT (started from console).
    if err.raw_os_error() == Some(1063) {
        let run = RUN
            .lock()
            .map_err(|_| anyhow::anyhow!("service run lock poisoned"))?
            .take()
            .ok_or_else(|| anyhow::anyhow!("service worker was not configured"))?;
        // The caller may already be inside a Tokio runtime (the normal
        // `#[tokio::main]` path). Build the server runtime on a dedicated
        // thread instead of nesting block_on.
        return std::thread::spawn(run)
            .join()
            .map_err(|_| anyhow::anyhow!("service worker thread panicked"))?;
    }
    Err(anyhow::anyhow!(
        "StartServiceCtrlDispatcher failed: {err}"
    ))
}
