//! Read-only ownership and exit proofs for the three installed Windows services.
//! Retained kernel handles identify the original instances even after PID reuse.
use super::{CONFIGS, PROGRAMS};
use anyhow::{ensure, Context, Result};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs::{self, File},
    io::Read,
    os::windows::{
        ffi::OsStringExt,
        fs::MetadataExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::{FILETIME, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{
        GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    },
};

// FILETIME counts 100 ns since 1601; DateTime.UtcNow.Ticks counts since year 1.
const DOTNET_FILETIME_EPOCH_TICKS: u64 = 504_911_232_000_000_000;
const RECORD_LIMIT: u64 = 64;

pub(super) struct ProcessIdentities {
    handles: Vec<OwnedHandle>,
}

impl ProcessIdentities {
    pub(super) fn ensure_exited(&self) -> Result<()> {
        for handle in &self.handles {
            // An exit code of STILL_ACTIVE (259) is a legitimate process exit
            // code. Only the original process handle's signaled state proves exit.
            ensure!(
                unsafe { WaitForSingleObject(handle.as_raw_handle() as _, 0) } == WAIT_OBJECT_0,
                "original installed Windows process has not exited"
            );
        }
        Ok(())
    }
}

fn decimal_record(path: &Path) -> Result<u64> {
    let meta = fs::symlink_metadata(path).context("process identity record unavailable")?;
    ensure!(
        meta.is_file()
            && meta.file_attributes() & 0x400 == 0 // FILE_ATTRIBUTE_REPARSE_POINT
            && meta.len() <= RECORD_LIMIT,
        "process identity record is not a bounded regular file"
    );
    let mut bytes = Vec::new();
    File::open(path)
        .context("process identity record cannot be opened")?
        .take(RECORD_LIMIT + 1)
        .read_to_end(&mut bytes)
        .context("process identity record cannot be read")?;
    ensure!(
        bytes.len() as u64 <= RECORD_LIMIT,
        "process identity record exceeds its bound"
    );
    let digits = bytes
        .strip_suffix(b"\r\n")
        .or_else(|| bytes.strip_suffix(b"\n"))
        .unwrap_or(&bytes);
    ensure!(
        !digits.is_empty() && digits.iter().all(u8::is_ascii_digit),
        "process identity record is not strict decimal"
    );
    std::str::from_utf8(digits)
        .context("process identity record encoding invalid")?
        .parse()
        .context("process identity record exceeds its numeric bound")
}

fn running(handle: &OwnedHandle) -> Result<()> {
    ensure!(
        unsafe { WaitForSingleObject(handle.as_raw_handle() as _, 0) } == WAIT_TIMEOUT,
        "installed Windows process is not currently running"
    );
    Ok(())
}

fn creation_ticks(handle: &OwnedHandle) -> Result<u64> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    ensure!(
        unsafe {
            GetProcessTimes(
                handle.as_raw_handle() as _,
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        } != 0,
        "installed Windows process creation time unavailable"
    );
    let value = ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
    value
        .checked_add(DOTNET_FILETIME_EPOCH_TICKS)
        .context("installed Windows creation time exceeds its bound")
}

fn image_path(handle: &OwnedHandle) -> Result<PathBuf> {
    // The Win32 maximum full path is bounded by 32,767 UTF-16 code units.
    // Preserve UTF-16 OS strings; lossy UTF-8 conversion could conflate paths.
    let mut bytes = vec![0u16; 32_768];
    let mut length = bytes.len() as u32;
    ensure!(
        unsafe {
            QueryFullProcessImageNameW(
                handle.as_raw_handle() as _,
                0,
                bytes.as_mut_ptr(),
                &mut length,
            )
        } != 0
            && length > 0
            && length as usize <= bytes.len(),
        "installed Windows process image path unavailable"
    );
    bytes.truncate(length as usize);
    fs::canonicalize(PathBuf::from(OsString::from_wide(&bytes)))
        .context("installed Windows process image cannot be resolved")
}

fn capture_one(prefix: &Path, config: &str, program: &str, pid: u32) -> Result<OwnedHandle> {
    let record = prefix
        .join("runtime")
        .join(format!("{}.pid.identity", config.trim_end_matches(".yaml")));
    let expected_ticks = decimal_record(&record)?;
    let expected_image = fs::canonicalize(prefix.join("bin").join(format!("{program}.exe")))
        .context("expected installed Windows executable unavailable")?;
    let raw = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    ensure!(
        !raw.is_null(),
        "installed Windows process handle unavailable"
    );
    // The handle is newly returned by OpenProcess; this owner closes it once.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw as _) };
    running(&handle)?;
    ensure!(
        image_path(&handle)? == expected_image,
        "recorded Windows process executable differs from installation"
    );
    ensure!(
        creation_ticks(&handle)? == expected_ticks,
        "recorded Windows process creation identity differs"
    );
    running(&handle)?;
    Ok(handle)
}

pub(super) fn capture(prefix: &Path, pids: &[u32]) -> Result<ProcessIdentities> {
    let provided: BTreeSet<_> = pids.iter().copied().collect();
    ensure!(
        pids.len() == 3 && provided.len() == 3 && !provided.contains(&0),
        "exactly three distinct nonzero Windows process IDs required"
    );
    let runtime = prefix.join("runtime");
    let mut records = Vec::new();
    for (config, program) in CONFIGS.iter().zip(PROGRAMS) {
        let pid =
            decimal_record(&runtime.join(format!("{}.pid", config.trim_end_matches(".yaml"))))?;
        let pid = u32::try_from(pid).context("recorded Windows process ID exceeds its bound")?;
        ensure!(pid > 0, "recorded Windows process ID must be nonzero");
        records.push((*config, program, pid));
    }
    let recorded: BTreeSet<_> = records.iter().map(|(_, _, pid)| *pid).collect();
    ensure!(
        recorded == provided,
        "Windows service PID records differ from the provided set"
    );
    let pid_files = fs::read_dir(&runtime)
        .context("Windows runtime records unavailable")?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .context("Windows runtime records cannot be enumerated")?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "pid"))
        .count();
    ensure!(
        pid_files == 3,
        "Windows runtime must contain exactly three PID records"
    );
    let handles = records
        .into_iter()
        .map(|(config, program, pid)| capture_one(prefix, config, program, pid))
        .collect::<Result<Vec<_>>>()?;
    for handle in &handles {
        running(handle)?;
    }
    Ok(ProcessIdentities { handles })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    const CONFIG: &str = "vpn233-client.yaml";
    const PROGRAM: &str = "vpn233-client";

    #[test]
    fn native_process_fixture() {
        let Some(work) = env::var_os("VPN233_NATIVE_PROCESS_TEST_WORK") else {
            return;
        };
        let work = PathBuf::from(work);
        fs::write(work.join("ready"), b"ready").unwrap();
        let start = Instant::now();
        while !work.join("exit").exists() {
            if start.elapsed() >= Duration::from_secs(15) {
                std::process::exit(3);
            }
            thread::sleep(Duration::from_millis(5));
        }
        std::process::exit(259);
    }

    struct Fixture {
        child: Child,
        work: tempfile::TempDir,
    }
    impl Fixture {
        fn start() -> Self {
            let executable = env::current_exe().unwrap();
            // Same-volume hard linking reuses the Cargo-built native fixture
            // without another compiler or copying the complete test executable.
            let work = tempfile::Builder::new()
                .prefix("windows-process-proof-")
                .tempdir_in(executable.parent().unwrap())
                .unwrap();
            fs::create_dir(work.path().join("bin")).unwrap();
            fs::create_dir(work.path().join("runtime")).unwrap();
            let binary = work.path().join("bin/vpn233-client.exe");
            fs::hard_link(&executable, &binary).unwrap();
            let test_name = format!(
                "{}::native_process_fixture",
                module_path!().split_once("::").unwrap().1
            );
            let mut command = Command::new(binary);
            command.env_clear();
            for key in [
                "SYSTEMROOT",
                "WINDIR",
                "PATH",
                "TEMP",
                "TMP",
                "OS",
                "USERPROFILE",
            ] {
                if let Some(value) = env::var_os(key) {
                    command.env(key, value);
                }
            }
            let child = command
                .args(["--exact", &test_name, "--nocapture"])
                .env("VPN233_NATIVE_PROCESS_TEST_WORK", work.path())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let fixture = Self { child, work };
            let start = Instant::now();
            while !fixture.work.path().join("ready").is_file() {
                assert!(start.elapsed() < Duration::from_secs(5));
                thread::sleep(Duration::from_millis(5));
            }
            let raw = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    0,
                    fixture.child.id(),
                )
            };
            assert!(!raw.is_null());
            let handle = unsafe { OwnedHandle::from_raw_handle(raw as _) };
            fs::write(
                fixture.record(),
                creation_ticks(&handle).unwrap().to_string(),
            )
            .unwrap();
            fixture
        }

        fn record(&self) -> PathBuf {
            self.work.path().join("runtime/vpn233-client.pid.identity")
        }
        fn capture(&self) -> Result<OwnedHandle> {
            capture_one(self.work.path(), CONFIG, PROGRAM, self.child.id())
        }
        fn exit(&mut self) {
            fs::write(self.work.path().join("exit"), b"exit").unwrap();
            let start = Instant::now();
            loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    assert_eq!(status.code(), Some(259));
                    break;
                }
                assert!(start.elapsed() < Duration::from_secs(5));
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[test]
    fn held_original_identity_proves_running_then_exit_including_exit_code_259() {
        let mut fixture = Fixture::start();
        let identities = ProcessIdentities {
            handles: vec![fixture.capture().unwrap()],
        };
        assert!(identities.ensure_exited().is_err());
        fixture.exit();
        // Child::try_wait already observed and reaped it. The retained kernel
        // handle still identifies that original instance, independently of its PID.
        identities.ensure_exited().unwrap();
        assert!(fixture.capture().is_err());
    }

    #[test]
    fn executable_and_creation_ticks_must_both_match_the_service_record() {
        let fixture = Fixture::start();
        assert!(fixture.capture().is_ok());
        let record = fixture.record();
        let correct = fs::read(&record).unwrap();
        fs::write(&record, b"1\r\n").unwrap();
        assert!(fixture.capture().is_err());
        fs::write(&record, correct).unwrap();
        let different = fixture.work.path().join("bin/unrelated.exe");
        fs::hard_link(env::current_exe().unwrap(), &different).unwrap();
        assert!(capture_one(fixture.work.path(), CONFIG, "unrelated", fixture.child.id()).is_err());
    }

    #[test]
    fn records_are_bounded_regular_files_with_strict_decimal_syntax() {
        let work = tempfile::tempdir().unwrap();
        let file = work.path().join("record");
        for invalid in [
            b"".as_slice(),
            b" 12",
            b"12 ",
            b"+12",
            b"12\n\n",
            b"12\r",
            b"0x12",
            b"18446744073709551616",
            &[b'1'; 65],
        ] {
            fs::write(&file, invalid).unwrap();
            assert!(decimal_record(&file).is_err());
        }
        for valid in [b"12".as_slice(), b"12\n", b"12\r\n"] {
            fs::write(&file, valid).unwrap();
            assert_eq!(decimal_record(&file).unwrap(), 12);
        }
        assert!(decimal_record(work.path()).is_err());
    }

    #[test]
    fn invalid_pid_sets_and_mismatched_service_assignments_are_rejected() {
        let work = tempfile::tempdir().unwrap();
        for pids in [
            vec![],
            vec![1, 2],
            vec![0, 1, 2],
            vec![1, 1, 2],
            vec![1, 2, 3, 4],
        ] {
            assert!(capture(work.path(), &pids).is_err());
        }
        fs::create_dir(work.path().join("runtime")).unwrap();
        for (config, pid) in CONFIGS.iter().zip([1, 2, 4]) {
            fs::write(
                work.path()
                    .join("runtime")
                    .join(format!("{}.pid", config.trim_end_matches(".yaml"))),
                pid.to_string(),
            )
            .unwrap();
        }
        assert!(capture(work.path(), &[1, 2, 3]).is_err());
    }
}
