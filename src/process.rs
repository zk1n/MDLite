//! Shell-free, bounded execution. Call `run` on a worker thread; cancellation is shared.
use std::{
    ffi::OsString,
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub struct ProcessRequest {
    pub executable: PathBuf,
    pub arguments: Vec<OsString>,
    pub cwd: PathBuf,
    pub timeout: Duration,
    /// Retained bytes per stream. Both streams continue draining after this limit.
    pub output_limit: usize,
    pub generation: u64,
    pub cancellation: Cancellation,
    /// Explicit environment overrides, for example noninteractive Git operation.
    pub environment: Vec<(OsString, OsString)>,
    pub remove_environment: Vec<OsString>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessOutcome {
    Exited(i32),
    Cancelled,
    TimedOut,
    SpawnFailed(String),
    IoFailed(String),
}
#[derive(Debug)]
pub struct ProcessResult {
    pub outcome: ProcessOutcome,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
    pub generation: u64,
}
impl ProcessResult {
    pub fn succeeded(&self) -> bool {
        self.outcome == ProcessOutcome::Exited(0)
    }
}

fn drain(mut reader: impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut output = Vec::new();
    let mut truncated = false;
    let mut buffer = [0; 8192];
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        let retain = count.min(limit.saturating_sub(output.len()));
        output.extend_from_slice(&buffer[..retain]);
        truncated |= retain < count;
    }
    Ok((output, truncated))
}

pub fn run(request: &ProcessRequest) -> ProcessResult {
    let mut result = ProcessResult {
        outcome: ProcessOutcome::Cancelled,
        stdout: Vec::new(),
        stderr: Vec::new(),
        truncated: false,
        generation: request.generation,
    };
    if request.cancellation.is_cancelled() {
        return result;
    }
    if request.timeout.is_zero()
        || request.timeout > Duration::from_secs(3600)
        || !request.cwd.is_dir()
    {
        result.outcome = ProcessOutcome::SpawnFailed(
            "A directory and finite timeout (at most one hour) are required".into(),
        );
        return result;
    }
    if request
        .executable
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("bat") || ext.eq_ignore_ascii_case("cmd"))
    {
        result.outcome =
            ProcessOutcome::SpawnFailed("Batch files require a shell and are not supported".into());
        return result;
    }
    #[cfg(not(windows))]
    {
        result.outcome = ProcessOutcome::SpawnFailed("Owned process trees require Windows".into());
        result
    }
    #[cfg(windows)]
    {
        run_windows(request, result)
    }
}

#[cfg(windows)]
fn run_windows(request: &ProcessRequest, mut result: ProcessResult) -> ProcessResult {
    use std::os::windows::{io::AsRawHandle, process::CommandExt};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        System::{Diagnostics::ToolHelp::*, JobObjects::*, Threading::*},
    };
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        result.outcome = ProcessOutcome::SpawnFailed(std::io::Error::last_os_error().to_string());
        return result;
    }
    let job = Handle(job);
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            std::mem::size_of_val(&limits) as u32,
        )
    } == 0
    {
        result.outcome = ProcessOutcome::SpawnFailed(std::io::Error::last_os_error().to_string());
        return result;
    }
    let mut command = Command::new(&request.executable);
    command
        .args(&request.arguments)
        .envs(request.environment.iter().cloned())
        .current_dir(&request.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
    for name in &request.remove_environment {
        command.env_remove(name);
    }
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            result.outcome = ProcessOutcome::SpawnFailed(e.to_string());
            return result;
        }
    };
    // The child cannot execute a single instruction before joining our kill-on-close job.
    let assign_ok = unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle()) } != 0;
    let resume = || -> std::io::Result<()> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        let snapshot = Handle(snapshot);
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of_val(&entry) as u32;
        let mut found = unsafe { Thread32First(snapshot.0, &mut entry) };
        while found != 0 {
            if entry.th32OwnerProcessID == child.id() {
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                let thread = Handle(thread);
                if unsafe { ResumeThread(thread.0) } == u32::MAX {
                    return Err(std::io::Error::last_os_error());
                }
                return Ok(());
            }
            found = unsafe { Thread32Next(snapshot.0, &mut entry) };
        }
        Err(std::io::Error::other(
            "Owned suspended primary thread not found",
        ))
    };
    if !assign_ok {
        let error = std::io::Error::last_os_error();
        let _ = child.kill();
        let _ = child.wait();
        result.outcome = ProcessOutcome::SpawnFailed(error.to_string());
        return result;
    }
    if let Err(e) = resume() {
        drop(job);
        let _ = child.wait();
        result.outcome = ProcessOutcome::SpawnFailed(e.to_string());
        return result;
    }
    // The request timeout bounds child execution, not Job setup or the
    // suspended launch that must finish before the process can run at all.
    let started = Instant::now();
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let limit = request.output_limit;
    let out = thread::spawn(move || drain(stdout, limit));
    let err = thread::spawn(move || drain(stderr, limit));
    result.outcome = loop {
        if request.cancellation.is_cancelled() {
            break ProcessOutcome::Cancelled;
        }
        if started.elapsed() >= request.timeout {
            break ProcessOutcome::TimedOut;
        }
        match child.try_wait() {
            Ok(Some(status)) => break ProcessOutcome::Exited(status.code().unwrap_or(-1)),
            Ok(None) => {}
            Err(e) => break ProcessOutcome::IoFailed(e.to_string()),
        }
        thread::sleep(Duration::from_millis(10));
    };
    // Also kill descendants that retain a pipe after their direct parent exits.
    drop(job);
    let _ = child.wait();
    for (reader, output) in [(out, &mut result.stdout), (err, &mut result.stderr)] {
        match reader.join() {
            Ok(Ok((bytes, truncated))) => {
                *output = bytes;
                result.truncated |= truncated;
            }
            Ok(Err(e)) => result.outcome = ProcessOutcome::IoFailed(e.to_string()),
            Err(_) => result.outcome = ProcessOutcome::IoFailed("Output reader panicked".into()),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[cfg(windows)]
    fn readiness_event_name() -> String {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("Local\\MDLiteProcessReady-{}-{nonce}", std::process::id())
    }

    #[cfg(windows)]
    fn create_readiness_event(name: &str) -> windows_sys::Win32::Foundation::HANDLE {
        use windows_sys::Win32::System::Threading::CreateEventW;
        let name: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        unsafe { CreateEventW(std::ptr::null(), 1, 0, name.as_ptr()) }
    }

    #[cfg(windows)]
    fn signal_readiness_event() {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{EVENT_MODIFY_STATE, OpenEventW, SetEvent},
        };
        let Ok(name) = std::env::var("MDLITE_PROCESS_READY_EVENT") else {
            return;
        };
        let name: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
        assert!(
            !event.is_null(),
            "fixture readiness event could not be opened"
        );
        assert_ne!(
            unsafe { SetEvent(event) },
            0,
            "fixture readiness signal failed"
        );
        unsafe {
            CloseHandle(event);
        }
    }

    #[cfg(windows)]
    fn run_in_thread(
        request: ProcessRequest,
    ) -> (mpsc::Receiver<ProcessResult>, thread::JoinHandle<()>) {
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let _ = sender.send(run(&request));
        });
        (receiver, worker)
    }
    #[test]
    fn drain_discards_tail_but_reads_to_eof() {
        let mut input = std::io::Cursor::new(vec![42; 32768]);
        let (bytes, truncated) = drain(&mut input, 17).unwrap();
        assert_eq!(bytes, vec![42; 17]);
        assert!(truncated);
        assert_eq!(input.position(), 32768);
    }
    #[test]
    fn cancelled_request_does_not_spawn() {
        let cancellation = Cancellation::default();
        cancellation.cancel();
        let r = run(&ProcessRequest {
            executable: "nonexistent".into(),
            arguments: vec![],
            cwd: ".".into(),
            timeout: Duration::from_secs(1),
            output_limit: 8,
            generation: 4,
            cancellation,
            environment: vec![],
            remove_environment: vec![],
        });
        assert_eq!(r.outcome, ProcessOutcome::Cancelled);
        assert_eq!(r.generation, 4);
    }
    #[cfg(windows)]
    fn fixture_request(mode: &str) -> ProcessRequest {
        ProcessRequest {
            executable: std::env::current_exe().unwrap(),
            arguments: vec![
                "--exact".into(),
                "process::tests::fixture_child".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::temp_dir(),
            timeout: Duration::from_secs(5),
            output_limit: 4096,
            generation: 1,
            cancellation: Cancellation::default(),
            environment: vec![("MDLITE_PROCESS_FIXTURE".into(), mode.into())],
            remove_environment: vec![],
        }
    }
    #[test]
    #[cfg(windows)]
    fn fixture_child() {
        use std::io::Write;
        match std::env::var("MDLITE_PROCESS_FIXTURE").as_deref() {
            Ok("output") => {
                let err = thread::spawn(|| {
                    std::io::stderr()
                        .write_all(&vec![b'e'; 256 * 1024])
                        .unwrap();
                });
                std::io::stdout()
                    .write_all(&vec![b'o'; 256 * 1024])
                    .unwrap();
                err.join().unwrap();
            }
            Ok("sleep") => {
                signal_readiness_event();
                thread::sleep(Duration::from_secs(60));
            }
            Ok("tree") => {
                let request = fixture_request("sleep");
                let mut child = Command::new(request.executable)
                    .args(request.arguments)
                    .envs(request.environment)
                    .env_remove("MDLITE_PROCESS_READY_EVENT")
                    .spawn()
                    .unwrap();
                println!("OWNED_DESCENDANT={}", child.id());
                std::io::stdout().flush().unwrap();
                signal_readiness_event();
                // The owning JobObject terminates us and this descendant on timeout.
                let _ = child.wait();
            }
            Ok("exit") => std::process::exit(23),
            _ => {}
        }
    }
    #[test]
    #[cfg(windows)]
    fn owned_process_outcomes_and_simultaneous_bounded_output() {
        let result = run(&fixture_request("output"));
        assert_eq!(result.outcome, ProcessOutcome::Exited(0), "{result:?}");
        assert!(result.truncated);
        assert_eq!(result.stdout.len(), 4096);
        assert_eq!(result.stderr.len(), 4096);
        assert_eq!(
            run(&fixture_request("exit")).outcome,
            ProcessOutcome::Exited(23)
        );
        let mut request = fixture_request("sleep");
        let cancellation = request.cancellation.clone();
        let trigger = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            cancellation.cancel();
        });
        assert_eq!(run(&request).outcome, ProcessOutcome::Cancelled);
        trigger.join().unwrap();

        let started = Instant::now();
        let ready_name = readiness_event_name();
        let ready_event = create_readiness_event(&ready_name);
        assert!(!ready_event.is_null());
        request = fixture_request("tree");
        request.timeout = Duration::from_secs(8);
        request
            .environment
            .push(("MDLITE_PROCESS_READY_EVENT".into(), ready_name.into()));
        let cancellation = request.cancellation.clone();
        let (receiver, worker) = run_in_thread(request);
        let descendant_ready = unsafe { WaitForSingleObject(ready_event, 4000) } == 0;
        cancellation.cancel();
        let result = receiver
            .recv_timeout(Duration::from_secs(9))
            .expect("owned tree cancellation completed before its finite deadline");
        worker.join().unwrap();
        unsafe {
            CloseHandle(ready_event);
        }
        assert!(
            descendant_ready,
            "descendant did not become ready: {result:?}"
        );
        assert_eq!(result.outcome, ProcessOutcome::Cancelled);
        let output = String::from_utf8_lossy(&result.stdout);
        let id: u32 = output
            .lines()
            .find_map(|line| line.strip_prefix("OWNED_DESCENDANT="))
            .expect("child spawned before timeout")
            .parse()
            .unwrap();
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, id) };
        if !handle.is_null() {
            assert_eq!(unsafe { WaitForSingleObject(handle, 1000) }, 0);
            unsafe {
                CloseHandle(handle);
            }
        }

        let ready_name = readiness_event_name();
        let ready_event = create_readiness_event(&ready_name);
        assert!(!ready_event.is_null());
        let mut request = fixture_request("sleep");
        request.timeout = Duration::from_secs(1);
        request
            .environment
            .push(("MDLITE_PROCESS_READY_EVENT".into(), ready_name.into()));
        let cancellation = request.cancellation.clone();
        let (receiver, worker) = run_in_thread(request);
        let sleeper_ready = unsafe { WaitForSingleObject(ready_event, 4000) } == 0;
        if !sleeper_ready {
            cancellation.cancel();
        }
        let result = receiver
            .recv_timeout(Duration::from_secs(4))
            .expect("ready sleep timeout completed before its finite deadline");
        worker.join().unwrap();
        unsafe {
            CloseHandle(ready_event);
        }
        assert!(
            sleeper_ready,
            "sleep fixture did not become ready: {result:?}"
        );
        assert_eq!(result.outcome, ProcessOutcome::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(25));

        let mut request = fixture_request("exit");
        request.executable = request.cwd.join("mdlite-nonexistent-fixture.exe");
        assert!(matches!(
            run(&request).outcome,
            ProcessOutcome::SpawnFailed(_)
        ));
    }
}
