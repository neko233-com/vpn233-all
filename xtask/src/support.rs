use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Output, Stdio},
    sync::{
        atomic::{AtomicU8, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

/// Runtime discovery avoids embedding the developer's machine paths in tools.
pub fn root() -> Result<PathBuf> {
    let output = process_output(
        Command::new("git").args(["rev-parse", "--show-toplevel"]),
        None,
        Duration::from_secs(20),
    )?;
    if !output.status.success() {
        bail!("workspace discovery failed");
    }
    let value = String::from_utf8(output.stdout)?;
    let value = value.trim_end_matches(['\r', '\n']);
    if value.is_empty() || value.contains(['\r', '\n', '\0']) {
        bail!("invalid workspace root");
    }
    canonical_path(Path::new(value))
}

pub fn normalize_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let value = path.as_os_str().to_string_lossy();
        if let Some(value) = value.strip_prefix("\\\\?\\UNC\\") {
            return PathBuf::from(format!("\\\\{value}"));
        }
        if let Some(value) = value.strip_prefix("\\\\?\\") {
            return PathBuf::from(value);
        }
    }
    path.to_path_buf()
}

pub fn canonical_path(path: &Path) -> Result<PathBuf> {
    Ok(normalize_path(&path.canonicalize()?))
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut input = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn read_output(mut input: impl Read) -> std::io::Result<Vec<u8>> {
    const LIMIT: u64 = 256 * 1024 * 1024;
    let mut bytes = Vec::new();
    input.by_ref().take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(std::io::Error::other("child output limit exceeded"));
    }
    Ok(bytes)
}

struct ProcessGroup {
    #[cfg(windows)]
    handle: std::os::windows::io::OwnedHandle,
    #[cfg(unix)]
    pid: libc::pid_t,
}

impl ProcessGroup {
    #[cfg(windows)]
    fn resume_owned_child(child: &Child) -> Result<()> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::{Diagnostics::ProcessSnapshotting::*, Threading::*},
        };
        // CREATE_SUSPENDED ensures no user code can spawn descendants before
        // job assignment. Capture only that child's threads, never the system.
        let process = child.as_raw_handle() as _;
        let mut snapshot = std::ptr::null_mut();
        if unsafe { PssCaptureSnapshot(process, PSS_CAPTURE_THREADS, 0, &mut snapshot) } != 0 {
            bail!("child thread ownership unavailable");
        }
        let mut marker = std::ptr::null_mut();
        let captured = (|| {
            let mut info = PSS_THREAD_INFORMATION::default();
            if unsafe {
                PssQuerySnapshot(
                    snapshot,
                    PSS_QUERY_THREAD_INFORMATION,
                    &mut info as *mut _ as _,
                    std::mem::size_of_val(&info) as u32,
                )
            } != 0
                || info.ThreadsCaptured != 1
            {
                bail!("child primary thread ownership unavailable");
            }
            if unsafe { PssWalkMarkerCreate(std::ptr::null(), &mut marker) } != 0 {
                bail!("child thread marker unavailable");
            }
            let mut entry = PSS_THREAD_ENTRY::default();
            if unsafe {
                PssWalkSnapshot(
                    snapshot,
                    PSS_WALK_THREADS,
                    marker,
                    &mut entry as *mut _ as _,
                    std::mem::size_of_val(&entry) as u32,
                )
            } != 0
                || entry.ProcessId != child.id()
            {
                bail!("child thread identity mismatch");
            }
            Ok(entry.ThreadId)
        })();
        let marker_closed = marker.is_null() || unsafe { PssWalkMarkerFree(marker) } == 0;
        // The snapshot belongs to this caller, although it describes the child.
        let snapshot_closed = unsafe { PssFreeSnapshot(GetCurrentProcess(), snapshot) } == 0;
        if !marker_closed || !snapshot_closed {
            bail!("owned child snapshot release failed");
        }
        let handle = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, captured?) };
        if handle.is_null() {
            bail!("owned child thread unavailable");
        }
        let previous = unsafe { ResumeThread(handle) };
        unsafe { CloseHandle(handle) };
        if previous != 1 {
            bail!("owned child thread resume failed");
        }
        Ok(())
    }

    fn attach(child: &Child) -> Result<Self> {
        #[cfg(windows)]
        {
            use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
            use windows_sys::Win32::System::JobObjects::*;
            // The unnamed job owns only this spawned command and descendants.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                bail!("child ownership unavailable");
            }
            // OwnedHandle is thread safe and closes the job exactly once.
            let handle = unsafe { OwnedHandle::from_raw_handle(handle as _) };
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = unsafe {
                SetInformationJobObject(
                    handle.as_raw_handle() as _,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    std::mem::size_of_val(&limits) as u32,
                ) != 0
                    && AssignProcessToJobObject(
                        handle.as_raw_handle() as _,
                        child.as_raw_handle() as _,
                    ) != 0
            };
            if !configured {
                bail!("child ownership unavailable");
            }
            Ok(Self { handle })
        }
        #[cfg(unix)]
        {
            Ok(Self {
                pid: child.id().try_into()?,
            })
        }
    }

    fn terminate(&self) {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(
                    self.handle.as_raw_handle() as _,
                    1,
                );
            }
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-self.pid, libc::SIGKILL);
        }
    }

    fn completed(&self) -> Result<()> {
        // A successful start script deliberately leaves its owned services alive.
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::JobObjects::*;
            let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            if unsafe {
                SetInformationJobObject(
                    self.handle.as_raw_handle() as _,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    std::mem::size_of_val(&limits) as u32,
                )
            } == 0
            {
                bail!("child completion ownership failed");
            }
        }
        Ok(())
    }
}

enum StreamEvent {
    Stdout(std::io::Result<Vec<u8>>),
    Stderr(std::io::Result<Vec<u8>>),
    Input(std::io::Result<()>),
}

/// Error labels are deliberately fixed; callers need not disclose child output,
/// paths, arguments, or operating-system error messages to diagnose a failure.
#[derive(Clone, Copy, Debug)]
pub enum ProcessFailure {
    Spawn,
    Ownership,
    Deadline,
    Pipe,
    Capture,
    Wait,
    ParentExit(Option<i32>),
}

impl std::fmt::Display for ProcessFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Spawn => "child spawn failed",
            Self::Ownership => "child ownership failed",
            Self::Deadline => "child execution exceeded deadline",
            Self::Pipe => "child pipe failed",
            Self::Capture => "private child capture failed",
            Self::Wait => "child wait failed",
            Self::ParentExit(_) => "child parent exited unsuccessfully",
        })
    }
}

impl std::error::Error for ProcessFailure {}

pub fn process_failure_kind(error: &anyhow::Error) -> &'static str {
    match error.downcast_ref::<ProcessFailure>() {
        Some(ProcessFailure::Spawn) => "spawn",
        Some(ProcessFailure::Ownership) => "ownership",
        Some(ProcessFailure::Deadline) => "deadline",
        Some(ProcessFailure::Pipe) => "pipe",
        Some(ProcessFailure::Capture) => "capture",
        Some(ProcessFailure::Wait | ProcessFailure::ParentExit(_)) => "wait",
        None => "unknown",
    }
}

pub fn process_failure_exit_code(error: &anyhow::Error) -> Option<i32> {
    match error.downcast_ref::<ProcessFailure>() {
        Some(ProcessFailure::ParentExit(code)) => *code,
        _ => None,
    }
}

fn prepare_owned_command(command: &mut Command) {
    for key in [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
    ] {
        command.env_remove(key);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
    }
}

fn attach_owned_child(mut child: Child) -> Result<(Child, ProcessGroup)> {
    let group = match ProcessGroup::attach(&child) {
        Ok(group) => group,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.context(ProcessFailure::Ownership));
        }
    };
    #[cfg(windows)]
    if let Err(error) = ProcessGroup::resume_owned_child(&child) {
        group.terminate();
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.context(ProcessFailure::Ownership));
    }
    Ok((child, group))
}

const CAPTURE_LIMIT: u64 = 256 * 1024 * 1024;

struct CaptureFile {
    file: File,
    length: u64,
}

impl CaptureFile {
    fn create(path: &Path) -> Result<Self> {
        let mut options = OpenOptions::new();
        options.create_new(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::{fs::OpenOptionsExt, io::AsRawFd};
            options.mode(0o600);
            let file = options.open(path).context(ProcessFailure::Capture)?;
            // Keep this newly created private file locked for its whole lifetime.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(ProcessFailure::Capture.into());
            }
            Ok(Self { file, length: 0 })
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Permit read-only inspection, but deny replacement/deletion and
            // additional writers. The parent directory supplies the private ACL.
            options.share_mode(1); // FILE_SHARE_READ
            let file = options.open(path).context(ProcessFailure::Capture)?;
            Ok(Self { file, length: 0 })
        }
    }

    fn append(&mut self, bytes: &[u8], limit: u64) -> Result<()> {
        let next = self.length.checked_add(bytes.len() as u64);
        if next.is_none_or(|length| length > limit) {
            return Err(ProcessFailure::Capture.into());
        }
        self.file
            .seek(SeekFrom::End(0))
            .context(ProcessFailure::Capture)?;
        self.file
            .write_all(bytes)
            .context(ProcessFailure::Capture)?;
        self.length = next.context(ProcessFailure::Capture)?;
        Ok(())
    }

    fn snapshot(&mut self) -> Result<Vec<u8>> {
        self.file
            .seek(SeekFrom::Start(0))
            .context(ProcessFailure::Capture)?;
        let mut bytes = vec![0; self.length.try_into().context(ProcessFailure::Capture)?];
        self.file
            .read_exact(&mut bytes)
            .context(ProcessFailure::Capture)?;
        Ok(bytes)
    }
}

enum CapturePipe {
    Stdout(ChildStdout),
    Stderr(ChildStderr),
}

impl CapturePipe {
    #[cfg(unix)]
    fn raw(&self) -> std::os::fd::RawFd {
        use std::os::fd::AsRawFd;
        match self {
            Self::Stdout(pipe) => pipe.as_raw_fd(),
            Self::Stderr(pipe) => pipe.as_raw_fd(),
        }
    }

    #[cfg(windows)]
    fn raw(&self) -> std::os::windows::io::RawHandle {
        use std::os::windows::io::AsRawHandle;
        match self {
            Self::Stdout(pipe) => pipe.as_raw_handle(),
            Self::Stderr(pipe) => pipe.as_raw_handle(),
        }
    }

    fn prepare(&self) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            let flags = unsafe { libc::fcntl(self.raw(), libc::F_GETFL) };
            if flags < 0
                || unsafe { libc::fcntl(self.raw(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(())
    }

    fn read_available(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::{Foundation::ERROR_BROKEN_PIPE, System::Pipes::PeekNamedPipe};
            let mut available = 0;
            if unsafe {
                PeekNamedPipe(
                    self.raw() as _,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                let error = std::io::Error::last_os_error();
                return if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
                    Ok(0)
                } else {
                    Err(error)
                };
            }
            if available == 0 {
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            let length = bytes.len().min(available as usize);
            match self {
                Self::Stdout(pipe) => pipe.read(&mut bytes[..length]),
                Self::Stderr(pipe) => pipe.read(&mut bytes[..length]),
            }
        }
        #[cfg(unix)]
        match self {
            Self::Stdout(pipe) => pipe.read(bytes),
            Self::Stderr(pipe) => pipe.read(bytes),
        }
    }
}

struct CaptureState {
    file: Mutex<CaptureFile>,
    failure: AtomicU8,
    // 0: running; 1: drain currently readable bytes; 2: stop immediately.
    stop: AtomicU8,
}

impl CaptureState {
    fn check(&self) -> Result<()> {
        match self.failure.load(Ordering::Acquire) {
            0 => Ok(()),
            1 => Err(ProcessFailure::Pipe.into()),
            _ => Err(ProcessFailure::Capture.into()),
        }
    }

    fn fail(&self, kind: u8, group: &ProcessGroup) {
        let _ = self
            .failure
            .compare_exchange(0, kind, Ordering::AcqRel, Ordering::Acquire);
        group.terminate();
    }

    fn snapshot(&self) -> Result<Vec<u8>> {
        self.check()?;
        self.file
            .lock()
            .map_err(|_| ProcessFailure::Capture)?
            .snapshot()
    }
}

fn spool_capture(
    mut pipe: CapturePipe,
    state: Arc<CaptureState>,
    group: Arc<ProcessGroup>,
    limit: u64,
) {
    if pipe.prepare().is_err() {
        state.fail(1, &group);
        return;
    }
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if state.stop.load(Ordering::Acquire) == 2 {
            break;
        }
        match pipe.read_available(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                let appended = state
                    .file
                    .lock()
                    .map_err(|_| ProcessFailure::Capture)
                    .and_then(|mut file| {
                        file.append(&buffer[..count], limit)
                            .map_err(|_| ProcessFailure::Capture)
                    });
                if appended.is_err() {
                    state.fail(2, &group);
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if state.stop.load(Ordering::Acquire) != 0 {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                state.fail(1, &group);
                break;
            }
        }
    }
}

/// Owns the launcher's process group and private bounded output captures.
/// Keep this guard alive until the installation has stopped its services.
/// Dropping it always terminates remaining owned descendants and joins readers.
pub struct OwnedProcessGroup {
    group: Arc<ProcessGroup>,
    child: Child,
    captures: [Arc<CaptureState>; 2],
    readers: Vec<thread::JoinHandle<()>>,
}

impl OwnedProcessGroup {
    pub fn check_health(&self) -> Result<()> {
        for capture in &self.captures {
            capture.check()?;
        }
        Ok(())
    }

    fn stop_readers(&mut self, drain: bool) -> Result<()> {
        for capture in &self.captures {
            capture
                .stop
                .store(if drain { 1 } else { 2 }, Ordering::Release);
        }
        self.group.terminate();
        let mut panicked = false;
        for reader in self.readers.drain(..) {
            if reader.join().is_err() {
                panicked = true;
            }
        }
        if panicked {
            return Err(ProcessFailure::Pipe.into());
        }
        self.check_health()
    }

    /// Finish after the caller has proved its stop script and process identities.
    /// Drain only bytes currently available, never wait for inherited pipe EOF,
    /// and report every reader failure before releasing the ownership guard.
    pub fn finish(mut self) -> Result<()> {
        self.stop_readers(true)
    }

    fn launcher_status(&mut self) -> Result<Option<ExitStatus>> {
        #[cfg(windows)]
        {
            self.child.try_wait().context(ProcessFailure::Wait)
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            // Do not reap this group leader until Drop. Its retained PID keeps
            // our PGID reserved even when the successful launcher has exited.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.child.id(),
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result != 0 {
                let error = std::io::Error::last_os_error();
                return if error.kind() == std::io::ErrorKind::Interrupted {
                    Ok(None)
                } else {
                    Err(error).context(ProcessFailure::Wait)
                };
            }
            if unsafe { info.si_pid() } == 0 {
                return Ok(None);
            }
            let code = unsafe { info.si_status() };
            let raw = match info.si_code {
                libc::CLD_EXITED => code << 8,
                libc::CLD_KILLED => code & 0x7f,
                libc::CLD_DUMPED => (code & 0x7f) | 0x80,
                _ => return Err(ProcessFailure::Wait.into()),
            };
            Ok(Some(ExitStatus::from_raw(raw)))
        }
    }
}

impl Drop for OwnedProcessGroup {
    fn drop(&mut self) {
        // Readers poll stop rather than waiting for inherited pipe EOF.
        let _ = self.stop_readers(false);
        // Readers can terminate the group on capture failure. Finish them before
        // reaping the Unix leader so none can use a subsequently recycled PGID.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Specialized capture for a launcher that intentionally leaves services alive.
/// The parent must exit successfully before the deadline. Unlike process_output,
/// this returns without waiting for descendants to close inherited output pipes.
/// Capture destinations must be new files in a private caller-owned directory.
/// Output contains a current snapshot; the private captures continue recording
/// while the returned guard owns the services.
pub fn process_launcher(
    command: &mut Command,
    stdout_path: &Path,
    stderr_path: &Path,
    timeout: Duration,
) -> Result<(Output, OwnedProcessGroup)> {
    process_launcher_with_limit(command, stdout_path, stderr_path, timeout, CAPTURE_LIMIT)
}

fn process_launcher_with_limit(
    command: &mut Command,
    stdout_path: &Path,
    stderr_path: &Path,
    timeout: Duration,
    limit: u64,
) -> Result<(Output, OwnedProcessGroup)> {
    let started = Instant::now();
    let captures = [stdout_path, stderr_path].map(|path| {
        CaptureFile::create(path).map(|file| {
            Arc::new(CaptureState {
                file: Mutex::new(file),
                failure: AtomicU8::new(0),
                stop: AtomicU8::new(0),
            })
        })
    });
    let [stdout, stderr] = captures;
    let captures = [stdout?, stderr?];
    prepare_owned_command(command);
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context(ProcessFailure::Spawn)?;
    let (child, group) = attach_owned_child(child)?;
    let mut owner = OwnedProcessGroup {
        group: Arc::new(group),
        child,
        captures,
        readers: Vec::new(),
    };
    let stdout = owner.child.stdout.take().context(ProcessFailure::Pipe)?;
    let stderr = owner.child.stderr.take().context(ProcessFailure::Pipe)?;
    for (pipe, capture) in [CapturePipe::Stdout(stdout), CapturePipe::Stderr(stderr)]
        .into_iter()
        .zip(&owner.captures)
    {
        let capture = Arc::clone(capture);
        let group = Arc::clone(&owner.group);
        owner.readers.push(
            thread::Builder::new()
                .name("private-child-capture".into())
                .spawn(move || spool_capture(pipe, capture, group, limit))
                .context(ProcessFailure::Capture)?,
        );
    }
    loop {
        owner.check_health()?;
        if started.elapsed() >= timeout {
            return Err(ProcessFailure::Deadline.into());
        }
        if let Some(status) = owner.launcher_status()? {
            if !status.success() {
                return Err(ProcessFailure::ParentExit(status.code()).into());
            }
            let output = Output {
                status,
                stdout: owner.captures[0].snapshot()?,
                stderr: owner.captures[1].snapshot()?,
            };
            owner.check_health()?;
            if started.elapsed() >= timeout {
                return Err(ProcessFailure::Deadline.into());
            }
            return Ok((output, owner));
        }
        thread::sleep(Duration::from_millis(5).min(timeout.saturating_sub(started.elapsed())));
    }
}

/// All pipes and child descendants share one deadline, including after parent exit.
/// Native subprocesses never receive GitHub API authentication environment values.
pub fn process_output(
    command: &mut Command,
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<Output> {
    let started = Instant::now();
    prepare_owned_command(command);
    let child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context(ProcessFailure::Spawn)?;
    let (mut child, group) = attach_owned_child(child)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("missing stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("missing stderr"))?;
    let (sender, receiver) = mpsc::channel();
    let stdout_sender = sender.clone();
    thread::spawn(move || {
        let _ = stdout_sender.send(StreamEvent::Stdout(read_output(stdout)));
    });
    let stderr_sender = sender.clone();
    thread::spawn(move || {
        let _ = stderr_sender.send(StreamEvent::Stderr(read_output(stderr)));
    });
    if let Some(bytes) = input {
        let bytes = bytes.to_vec();
        let pipe = child.stdin.take();
        thread::spawn(move || {
            let result = (|| {
                if let Some(mut pipe) = pipe {
                    pipe.write_all(&bytes)?;
                }
                Ok(())
            })();
            let _ = sender.send(StreamEvent::Input(result));
        });
    }
    let result =
        (|| {
            let mut status = None;
            let mut stdout = None;
            let mut stderr = None;
            let mut input_complete = input.is_none();
            loop {
                if status.is_none() {
                    status = child.try_wait().context(ProcessFailure::Wait)?;
                    if status.is_some_and(|status| !status.success()) {
                        // Stop descendants immediately so inherited pipes cannot
                        // delay a failed parent until the execution deadline.
                        group.terminate();
                    }
                }
                if status.is_some() && stdout.is_some() && stderr.is_some() && input_complete {
                    return Ok(Output {
                        status: status.context("missing child status")?,
                        stdout: stdout.context("missing child output")?,
                        stderr: stderr.context("missing child error output")?,
                    });
                }
                if started.elapsed() >= timeout {
                    return Err(anyhow!(
                    "child execution exceeded deadline (parent={}, stdout={}, stderr={}, stdin={})",
                    if status.is_some() { "exited" } else { "running" },
                    if stdout.is_some() { "closed" } else { "pending" },
                    if stderr.is_some() { "closed" } else { "pending" },
                    if input_complete { "complete" } else { "pending" },
                )
                    .context(ProcessFailure::Deadline));
                }
                match receiver.recv_timeout(
                    Duration::from_millis(10).min(timeout.saturating_sub(started.elapsed())),
                ) {
                    Ok(StreamEvent::Stdout(bytes)) => {
                        stdout = Some(bytes.context(ProcessFailure::Capture)?)
                    }
                    Ok(StreamEvent::Stderr(bytes)) => {
                        stderr = Some(bytes.context(ProcessFailure::Capture)?)
                    }
                    Ok(StreamEvent::Input(result)) => {
                        result.context(ProcessFailure::Pipe)?;
                        input_complete = true;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected)
                        if stdout.is_some() && stderr.is_some() && input_complete =>
                    {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(_) => return Err(ProcessFailure::Pipe.into()),
                }
            }
        })();
    if !matches!(&result, Ok(output) if output.status.success()) {
        group.terminate();
        let _ = child.kill();
        let _ = child.wait();
    } else {
        group.completed().context(ProcessFailure::Ownership)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_digest_matches_standard_vector() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"abc").unwrap();
        assert_eq!(
            sha256_file(file.path()).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn runtime_root_uses_current_repository() {
        let repository = root().unwrap();
        assert!(repository.join("Cargo.toml").is_file());
        assert!(repository.join("xtask/Cargo.toml").is_file());
    }

    #[test]
    #[allow(clippy::zombie_processes)] // Deliberately detached; the owning test kills its process group.
    fn inherited_pipe_fixture() {
        match std::env::var("VPN233_TEST_PIPE_STAGE").as_deref() {
            Ok("queued-capture") => {
                // Fit in an anonymous pipe before any reader is started. The
                // separate ready file proves these bytes are already queued.
                std::io::stdout().write_all(&[b'q'; 1024]).unwrap();
                std::io::stdout().flush().unwrap();
                std::fs::write(
                    std::env::var_os("VPN233_TEST_PIPE_READY").unwrap(),
                    b"queued",
                )
                .unwrap();
                thread::sleep(Duration::from_secs(15));
            }
            Ok(
                stage @ ("launcher-parent"
                | "launcher-failed-parent"
                | "launcher-hanging-parent"
                | "launcher-flood-parent"),
            ) => {
                let _child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "support::tests::inherited_pipe_fixture",
                        "--nocapture",
                    ])
                    .env(
                        "VPN233_TEST_PIPE_STAGE",
                        if stage == "launcher-flood-parent" {
                            "launcher-flood-listener"
                        } else {
                            "launcher-listener"
                        },
                    )
                    .spawn()
                    .unwrap();
                let ready = PathBuf::from(std::env::var_os("VPN233_TEST_PIPE_READY").unwrap());
                let started = Instant::now();
                while !ready.is_file() {
                    assert!(started.elapsed() < Duration::from_secs(4));
                    thread::sleep(Duration::from_millis(5));
                }
                match stage {
                    "launcher-failed-parent" => std::process::exit(7),
                    "launcher-hanging-parent" => thread::sleep(Duration::from_secs(15)),
                    _ => {}
                }
            }
            Ok(stage @ ("launcher-listener" | "launcher-flood-listener")) => {
                let listener =
                    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
                let ready = PathBuf::from(std::env::var_os("VPN233_TEST_PIPE_READY").unwrap());
                std::fs::write(ready, listener.local_addr().unwrap().port().to_string()).unwrap();
                if stage == "launcher-flood-listener" {
                    let trigger =
                        PathBuf::from(std::env::var_os("VPN233_TEST_PIPE_TRIGGER").unwrap());
                    let started = Instant::now();
                    while !trigger.is_file() {
                        assert!(started.elapsed() < Duration::from_secs(5));
                        thread::sleep(Duration::from_millis(5));
                    }
                    let _ = std::io::stdout().write_all(&vec![b'x'; 1024 * 1024]);
                }
                thread::sleep(Duration::from_secs(15));
                drop(listener);
            }
            Ok("parent") => {
                // A native child avoids shell-specific detachment and quoting.
                // Inherit both pipes, then let this parent exit without waiting.
                let _child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "support::tests::inherited_pipe_fixture",
                        "--nocapture",
                    ])
                    .env("VPN233_TEST_PIPE_STAGE", "descendant")
                    .spawn()
                    .unwrap();
            }
            Ok("descendant") => thread::sleep(Duration::from_secs(5)),
            Ok("failed-parent") => {
                let _child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "support::tests::inherited_pipe_fixture",
                        "--nocapture",
                    ])
                    .env("VPN233_TEST_PIPE_STAGE", "redirected")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                let ready = PathBuf::from(std::env::var_os("VPN233_TEST_PIPE_READY").unwrap());
                let started = Instant::now();
                while !ready.is_file() {
                    assert!(started.elapsed() < Duration::from_secs(2));
                    thread::sleep(Duration::from_millis(10));
                }
                std::process::exit(7);
            }
            Ok("redirected") => {
                let listener =
                    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
                let ready = PathBuf::from(std::env::var_os("VPN233_TEST_PIPE_READY").unwrap());
                std::fs::write(ready, listener.local_addr().unwrap().port().to_string()).unwrap();
                thread::sleep(Duration::from_secs(5));
                drop(listener);
            }
            _ => {}
        }
    }

    #[test]
    fn inherited_descendant_pipes_cannot_remove_the_deadline() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "support::tests::inherited_pipe_fixture",
                "--nocapture",
            ])
            .env("VPN233_TEST_PIPE_STAGE", "parent");
        let started = Instant::now();
        assert!(process_output(&mut command, None, Duration::from_millis(500)).is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn failed_commands_cannot_leave_redirected_descendants_running() {
        let directory = tempfile::tempdir().unwrap();
        let ready = directory.path().join("ready");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "support::tests::inherited_pipe_fixture",
                "--nocapture",
            ])
            .env("VPN233_TEST_PIPE_STAGE", "failed-parent")
            .env("VPN233_TEST_PIPE_READY", &ready);
        let output = process_output(&mut command, None, Duration::from_secs(4)).unwrap();
        assert_eq!(output.status.code(), Some(7));
        let port: u16 = std::fs::read_to_string(ready).unwrap().parse().unwrap();
        let started = Instant::now();
        while std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).is_ok() {
            assert!(started.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn child_pipes_are_drained_while_large_input_is_written() {
        #[cfg(unix)]
        let mut command = {
            let mut command = Command::new("sh");
            command.args(["-c", "cat; head -c 262144 /dev/zero >&2"]);
            command
        };
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("powershell");
            command.args(["-NoProfile", "-Command", "[Console]::Out.Write([Console]::In.ReadToEnd()); [Console]::Error.Write(('x' * 262144))"]);
            command
        };
        let input = vec![b'a'; 262144];
        let result = process_output(&mut command, Some(&input), Duration::from_secs(10)).unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout, input);
        assert_eq!(result.stderr.len(), 262144);
    }

    fn launcher_fixture(directory: &Path, stage: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "support::tests::inherited_pipe_fixture",
                "--nocapture",
            ])
            .env("VPN233_TEST_PIPE_STAGE", stage)
            .env("VPN233_TEST_PIPE_READY", directory.join("ready"))
            .env("VPN233_TEST_PIPE_TRIGGER", directory.join("trigger"));
        command
    }

    fn listener_port(directory: &Path) -> u16 {
        std::fs::read_to_string(directory.join("ready"))
            .unwrap()
            .parse()
            .unwrap()
    }

    fn assert_listener_stopped(port: u16) {
        let started = Instant::now();
        while std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).is_ok() {
            assert!(started.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn launcher_keeps_inherited_pipe_listener_owned_until_guard_drop() {
        let directory = tempfile::tempdir().unwrap();
        let mut command = launcher_fixture(directory.path(), "launcher-parent");
        let stdout = directory.path().join("stdout");
        let stderr = directory.path().join("stderr");
        let started = Instant::now();
        let (output, owner) =
            process_launcher(&mut command, &stdout, &stderr, Duration::from_secs(5)).unwrap();
        assert!(output.status.success());
        assert!(started.elapsed() < Duration::from_secs(5));
        let port = listener_port(directory.path());
        assert!(std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).is_ok());
        owner.check_health().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&stdout).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let stopped = Instant::now();
        drop(owner);
        assert!(stopped.elapsed() < Duration::from_secs(2));
        assert_listener_stopped(port);
    }

    #[test]
    fn launcher_finish_joins_readers_and_terminates_owned_listener() {
        let directory = tempfile::tempdir().unwrap();
        let mut command = launcher_fixture(directory.path(), "launcher-parent");
        let (_, owner) = process_launcher(
            &mut command,
            &directory.path().join("stdout"),
            &directory.path().join("stderr"),
            Duration::from_secs(5),
        )
        .unwrap();
        let port = listener_port(directory.path());
        owner.check_health().unwrap();
        let finished = Instant::now();
        owner.finish().unwrap();
        assert!(finished.elapsed() < Duration::from_secs(2));
        assert_listener_stopped(port);
    }

    #[test]
    fn launcher_finish_reports_reader_panics_instead_of_accepting_success() {
        let directory = tempfile::tempdir().unwrap();
        let mut command = launcher_fixture(directory.path(), "launcher-parent");
        let (_, mut owner) = process_launcher(
            &mut command,
            &directory.path().join("stdout"),
            &directory.path().join("stderr"),
            Duration::from_secs(5),
        )
        .unwrap();
        let port = listener_port(directory.path());
        owner.readers.push(thread::spawn(|| {
            panic!("controlled capture reader fixture")
        }));
        let error = owner.finish().unwrap_err();
        assert_eq!(process_failure_kind(&error), "pipe");
        assert_listener_stopped(port);
    }

    #[test]
    fn failed_launcher_parent_terminates_inherited_pipe_listener() {
        let directory = tempfile::tempdir().unwrap();
        let mut command = launcher_fixture(directory.path(), "launcher-failed-parent");
        let error = process_launcher(
            &mut command,
            &directory.path().join("stdout"),
            &directory.path().join("stderr"),
            Duration::from_secs(5),
        )
        .err()
        .unwrap();
        assert_eq!(process_failure_kind(&error), "wait");
        assert_eq!(process_failure_exit_code(&error), Some(7));
        assert_listener_stopped(listener_port(directory.path()));
    }

    #[test]
    fn launcher_deadline_terminates_parent_and_listener() {
        let directory = tempfile::tempdir().unwrap();
        let mut command = launcher_fixture(directory.path(), "launcher-hanging-parent");
        let started = Instant::now();
        let error = process_launcher(
            &mut command,
            &directory.path().join("stdout"),
            &directory.path().join("stderr"),
            Duration::from_secs(2),
        )
        .err()
        .unwrap();
        assert_eq!(process_failure_kind(&error), "deadline");
        assert!(started.elapsed() < Duration::from_secs(4));
        assert_listener_stopped(listener_port(directory.path()));
    }

    #[test]
    fn launcher_capture_limit_is_enforced_after_successful_parent_exit() {
        let directory = tempfile::tempdir().unwrap();
        let mut command = launcher_fixture(directory.path(), "launcher-flood-parent");
        let stdout = directory.path().join("stdout");
        let (output, owner) = process_launcher_with_limit(
            &mut command,
            &stdout,
            &directory.path().join("stderr"),
            Duration::from_secs(5),
            64 * 1024,
        )
        .unwrap();
        assert!(output.status.success());
        let port = listener_port(directory.path());
        assert!(std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).is_ok());
        owner.check_health().unwrap();
        std::fs::write(directory.path().join("trigger"), b"go").unwrap();
        // No further health check may consume or hide this late failure. The
        // final consuming finish must reject it after joining all readers.
        assert_listener_stopped(port);
        let error = owner.finish().unwrap_err();
        assert_eq!(process_failure_kind(&error), "capture");
        assert!(std::fs::metadata(&stdout).unwrap().len() <= 64 * 1024);
    }

    #[test]
    fn finishing_drains_already_queued_bytes_and_enforces_the_same_hard_limit() {
        for (limit, expected_success) in [(4096, true), (128, false)] {
            let directory = tempfile::tempdir().unwrap();
            let stdout = directory.path().join("stdout");
            let captures = [&stdout, &directory.path().join("stderr")].map(|path| {
                Arc::new(CaptureState {
                    file: Mutex::new(CaptureFile::create(path).unwrap()),
                    failure: AtomicU8::new(0),
                    stop: AtomicU8::new(0),
                })
            });
            let mut command = launcher_fixture(directory.path(), "queued-capture");
            prepare_owned_command(&mut command);
            let child = command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let (mut child, group) = attach_owned_child(child).unwrap();
            let pipe = CapturePipe::Stdout(child.stdout.take().unwrap());
            let mut owner = OwnedProcessGroup {
                group: Arc::new(group),
                child,
                captures,
                readers: Vec::new(),
            };
            let started = Instant::now();
            while !directory.path().join("ready").is_file() {
                assert!(started.elapsed() < Duration::from_secs(5));
                thread::sleep(Duration::from_millis(5));
            }
            let capture = Arc::clone(&owner.captures[0]);
            let group = Arc::clone(&owner.group);
            // Deliberately queue bytes before starting the real pipe reader.
            owner.readers.push(thread::spawn(move || {
                spool_capture(pipe, capture, group, limit)
            }));
            let result = owner.finish();
            assert_eq!(result.is_ok(), expected_success);
            let bytes = std::fs::read(stdout).unwrap();
            if expected_success {
                assert!(bytes.windows(1024).any(|window| window == [b'q'; 1024]));
            } else {
                assert_eq!(process_failure_kind(&result.unwrap_err()), "capture");
            }
            assert!(bytes.len() as u64 <= limit);
        }
    }

    #[test]
    fn launcher_capture_never_overwrites_existing_files() {
        let directory = tempfile::tempdir().unwrap();
        let stdout = directory.path().join("stdout");
        std::fs::write(&stdout, b"preserve").unwrap();
        let mut command = launcher_fixture(directory.path(), "launcher-parent");
        let error = process_launcher(
            &mut command,
            &stdout,
            &directory.path().join("stderr"),
            Duration::from_secs(5),
        )
        .err()
        .unwrap();
        assert_eq!(process_failure_kind(&error), "capture");
        assert_eq!(std::fs::read(stdout).unwrap(), b"preserve");
        assert!(!directory.path().join("ready").exists());
    }
}
