use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::mpsc,
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
    handle: windows_sys::Win32::Foundation::HANDLE,
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
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
            // The unnamed job owns only this spawned command and descendants.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                bail!("child ownership unavailable");
            }
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = unsafe {
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    std::mem::size_of_val(&limits) as u32,
                ) != 0
                    && AssignProcessToJobObject(handle, child.as_raw_handle() as _) != 0
            };
            if !configured {
                unsafe {
                    CloseHandle(handle);
                }
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
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.handle, 1);
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
            use windows_sys::Win32::System::JobObjects::*;
            let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            if unsafe {
                SetInformationJobObject(
                    self.handle,
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

#[cfg(windows)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

enum StreamEvent {
    Stdout(std::io::Result<Vec<u8>>),
    Stderr(std::io::Result<Vec<u8>>),
    Input(std::io::Result<()>),
}

/// All pipes and child descendants share one deadline, including after parent exit.
/// Native subprocesses never receive GitHub API authentication environment values.
pub fn process_output(
    command: &mut Command,
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<Output> {
    let started = Instant::now();
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
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("child spawn failed")?;
    let group = match ProcessGroup::attach(&child) {
        Ok(group) => group,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    #[cfg(windows)]
    if let Err(error) = ProcessGroup::resume_owned_child(&child) {
        group.terminate();
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
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
                    status = child.try_wait()?;
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
                    bail!(
                    "child execution exceeded deadline (parent={}, stdout={}, stderr={}, stdin={})",
                    if status.is_some() { "exited" } else { "running" },
                    if stdout.is_some() { "closed" } else { "pending" },
                    if stderr.is_some() { "closed" } else { "pending" },
                    if input_complete { "complete" } else { "pending" },
                );
                }
                match receiver.recv_timeout(
                    Duration::from_millis(10).min(timeout.saturating_sub(started.elapsed())),
                ) {
                    Ok(StreamEvent::Stdout(bytes)) => stdout = Some(bytes?),
                    Ok(StreamEvent::Stderr(bytes)) => stderr = Some(bytes?),
                    Ok(StreamEvent::Input(result)) => {
                        result?;
                        input_complete = true;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected)
                        if stdout.is_some() && stderr.is_some() && input_complete =>
                    {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(_) => bail!("child pipe failed"),
                }
            }
        })();
    if !matches!(&result, Ok(output) if output.status.success()) {
        group.terminate();
        let _ = child.kill();
        let _ = child.wait();
    } else {
        group.completed()?;
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
}
