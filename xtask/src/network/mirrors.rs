use super::{clean_command, option, path_arg, private_write, program};
use anyhow::{anyhow, bail, ensure, Result};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read, Write},
    net::{Ipv4Addr, Shutdown, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Sh,
    Ps,
}
#[derive(Clone, Debug)]
struct Request {
    route: String,
    offset: Option<usize>,
    encoding: Option<String>,
    path: String,
}
struct Server {
    port: u16,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<Vec<JoinHandle<()>>>>,
}
impl Server {
    fn new(kind: Kind, payload: Vec<u8>) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let records = requests.clone();
        let stopping = stop.clone();
        let body = Arc::new(payload);
        let worker = thread::spawn(move || {
            let mut clients = Vec::new();
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let records = records.clone();
                        let stopping = stopping.clone();
                        let body = body.clone();
                        clients.push(thread::spawn(move || {
                            let _ = serve(stream, kind, &body, &records, &stopping);
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
            clients
        });
        Ok(Self {
            port,
            requests,
            stop,
            worker: Some(worker),
        })
    }
    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            if let Ok(clients) = worker.join() {
                for client in clients {
                    let _ = client.join();
                }
            }
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn stopped_sleep(stop: &AtomicBool, duration: Duration) {
    let begin = Instant::now();
    while begin.elapsed() < duration && !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(5));
    }
}

fn serve(
    mut stream: TcpStream,
    kind: Kind,
    payload: &[u8],
    records: &Mutex<Vec<Request>>,
    stop: &AtomicBool,
) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let mut header = Vec::new();
    while !header.windows(4).any(|part| part == b"\r\n\r\n") {
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        header.extend_from_slice(&buffer[..count]);
        ensure!(header.len() < 32768, "fixture request too large");
    }
    let header = String::from_utf8(header)?;
    let path = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_owned();
    let route = path.split('/').nth(1).unwrap_or("").to_owned();
    let headers: Vec<_> = header
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .collect();
    let field = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim().to_owned())
    };
    let offset = field("Range").and_then(|value| {
        value
            .strip_prefix("bytes=")
            .and_then(|s| s.strip_suffix('-'))
            .and_then(|s| s.parse::<usize>().ok())
    });
    records.lock().unwrap().push(Request {
        route: route.clone(),
        offset,
        encoding: field("Accept-Encoding"),
        path: path.clone(),
    });
    let mut body = payload.to_vec();
    let mut start = offset.unwrap_or(0);
    let mut status = if offset.is_some() { 206 } else { 200 };
    let chunk = if kind == Kind::Sh { 65536 } else { 4093 };
    if route.starts_with("manifest-") {
        let names = [
            "linux-x86_64.tar.gz",
            "windows-x86_64.zip",
            "macos-x86_64.tar.gz",
            "macos-aarch64.tar.gz",
        ];
        body = match route.as_str() {
            "manifest-missing" => format!("{}  unrelated.bin\n", digest(payload)),
            "manifest-invalid" => names
                .iter()
                .map(|name| format!("invalid-digest  vpn233-rust-{name}\n"))
                .collect(),
            _ => names
                .iter()
                .map(|name| {
                    format!(
                        "{0}  vpn233-rust-{name}\n{0}  vpn233-rust-{name}\n",
                        digest(payload)
                    )
                })
                .collect(),
        }
        .into_bytes();
    } else if kind == Kind::Ps && path.ends_with("/SHA256SUMS") {
        body = format!("{}  vpn233-rust-windows-x86_64.zip\n", digest(payload)).into_bytes();
        start = 0;
        status = 200;
    } else if route == "wrong" {
        body = b"synthetic wrong archive".to_vec();
        start = 0;
        status = 200;
    } else if (route == "no-range" || route == "reset") && offset.is_some() {
        start = 0;
        status = 200;
    } else if (route == "bad-position" || route == "wrongrange") && offset.is_some() {
        start = 0;
    } else if route == "changed" && offset.is_none() {
        body.fill(b'x');
    }
    if route == "range-416" && offset.is_some() {
        stream.write_all(format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nContent-Range: bytes */{}\r\nConnection: close\r\n\r\n", body.len()).as_bytes())?;
        return Ok(());
    }
    if route == "unavailable" {
        body = b"synthetic HTTP failure".to_vec();
        start = 0;
        status = 500;
    }
    let start_data = if let Some(offset) = offset.filter(|_| route == "wrongrange") {
        offset
    } else {
        start
    };
    let data = body.get(start_data..).unwrap_or(&[]);
    let chunked = route == "chunked" && offset.is_some();
    let declared = if route == "oversized" {
        1073741825
    } else {
        data.len()
            + usize::from(route == "complete-eof")
            + if route == "resetafterfull" { 16 } else { 0 }
            - usize::from(route == "wronglength" && offset.is_some())
    };
    let mut response = format!("HTTP/1.1 {status} OK\r\nConnection: close\r\n");
    if chunked {
        response.push_str("Transfer-Encoding: chunked\r\n");
    } else {
        response.push_str(&format!("Content-Length: {declared}\r\n"));
    }
    if status == 206 && route != "missing-range" {
        response.push_str(&format!(
            "Content-Range: bytes {start}-{}/{}\r\n",
            body.len() - 1,
            body.len() + usize::from(route == "changedtotal")
        ));
    }
    if route == "encoded" {
        response.push_str("Content-Encoding: gzip\r\n");
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes())?;
    if route == "stall" {
        stopped_sleep(stop, Duration::from_secs(5));
        return Ok(());
    }
    if route == "trickle" {
        for byte in data {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            stream.write_all(&[*byte])?;
            stream.flush()?;
            stopped_sleep(stop, Duration::from_millis(40));
        }
        return Ok(());
    }
    let mut truncate = offset.is_none()
        && [
            "resume",
            "chunked",
            "no-range",
            "reset",
            "bad-position",
            "wrongrange",
            "wronglength",
            "changedtotal",
            "missing-range",
            "changed",
            "bounded",
            "range-416",
            "alwaysshort",
        ]
        .contains(&route.as_str());
    if route == "bounded"
        || route == "alwaysshort"
        || (route == "resume-twice" && offset.unwrap_or(0) < 2 * chunk)
    {
        truncate = true;
    }
    if route == "slow" {
        truncate = true;
        stopped_sleep(stop, Duration::from_millis(1200));
    }
    if chunked {
        write!(stream, "{:x}\r\n", data.len())?;
        stream.write_all(data)?;
        stream.write_all(b"\r\n0\r\n\r\n")?;
    } else {
        stream.write_all(if truncate {
            &data[..data.len().min(chunk)]
        } else {
            data
        })?;
    }
    stream.flush()?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}

struct Case<'a> {
    root: &'a Path,
    shell: &'a Path,
    kind: Kind,
    sequence: std::cell::Cell<usize>,
    stage: std::cell::Cell<&'static str>,
    passed: std::cell::Cell<bool>,
}
impl Drop for Case<'_> {
    fn drop(&mut self) {
        if !self.passed.get() {
            eprintln!(
                "mirror-http: {} case {:02} failed during {}",
                if self.kind == Kind::Sh { "SH" } else { "PS" },
                self.sequence.get(),
                self.stage.get()
            );
        }
    }
}
struct Transfer {
    requests: Vec<Request>,
    elapsed: Duration,
}
struct TransferOptions<'a> {
    hash: Option<&'a str>,
    budget: &'a str,
    success: bool,
    prior_cache: bool,
    installer: bool,
}
impl Case<'_> {
    fn advance(&self, stage: &'static str) {
        self.sequence.set(self.sequence.get() + 1);
        self.stage.set(stage);
    }
    fn transfer(
        &self,
        routes: &[&str],
        payload: &[u8],
        options: TransferOptions<'_>,
    ) -> Result<Transfer> {
        self.advance("fixture");
        let TransferOptions {
            hash,
            budget,
            success,
            prior_cache,
            installer,
        } = options;
        let server = Server::new(self.kind, payload.to_vec())?;
        let temp = tempfile::tempdir()?;
        let destination = temp.path().join("artifact.bin");
        fs::write(&destination, b"synthetic prior destination")?;
        let home = temp.path().join("home");
        let cache = home.join("mirror-cache");
        let prefixes: Vec<_> = routes
            .iter()
            .map(|route| format!("http://127.0.0.1:{}/{route}/", server.port))
            .collect();
        let previous = if self.kind == Kind::Ps {
            "release http://127.0.0.1/remembered/\n".to_owned()
        } else {
            format!(
                "raw https://previous.example.invalid/\nrelease {}\n",
                prefixes[0]
            )
        };
        if prior_cache {
            fs::create_dir(&home)?;
            fs::write(&cache, &previous)?;
        }
        let mut command = clean_command(self.shell);
        command
            .env("VPN233_HOME", &home)
            .env("VPN233_GH_MIRRORS", prefixes.join("|"))
            .env("VPN233_GH_MAX_TIME", budget)
            .env("VPN233_GH_CONNECT_TIMEOUT", "2")
            .env("VPN233_TEST_DIGEST", hash.unwrap_or(""));
        if self.kind == Kind::Sh {
            if installer {
                command
                    .arg(path_arg(&self.root.join("install.sh")))
                    .args([
                        "--release",
                        "--version",
                        "2.1.0",
                        "--no-start",
                        "--yes",
                        "--prefix",
                    ])
                    .arg(path_arg(&temp.path().join("installed")))
                    .args(["--mirror", &prefixes[0]]);
            } else {
                command.args(["-c", "set -Eeuo pipefail; . \"$VPN233_TEST_LIBRARY\"; vpn233_download_release_asset artifact.bin \"$VPN233_TEST_DEST\" 2.1.0 0 \"$VPN233_TEST_DIGEST\""]).env("VPN233_TEST_LIBRARY", path_arg(&self.root.join("scripts/lib/mirrors.sh"))).env("VPN233_TEST_DEST", path_arg(&destination));
            }
        } else {
            #[cfg(windows)]
            if let Some(system) = std::env::var_os("SystemRoot") {
                command.env(
                    "PSModulePath",
                    PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/Modules"),
                );
            }
            command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
            if installer {
                command
                    .arg(self.root.join("install.ps1"))
                    .args(["-Release", "-Version", "2.1.0", "-Prefix"])
                    .arg(temp.path().join("installed"))
                    .args(["-NoStart", "-Mirror", "auto"]);
            } else {
                let script = temp.path().join("receive.ps1");
                private_write(&script, b"param([string]$Library,[string]$Destination,[switch]$NoHash)\n$ErrorActionPreference='Stop'\n. $Library\nif($NoHash){Receive-Vpn233ReleaseAsset fixture.bin $Destination '2.1.0' -Quiet -WarningAction SilentlyContinue}else{Receive-Vpn233ReleaseAsset fixture.bin $Destination '2.1.0' -ExpectedSha256 $env:VPN233_TEST_DIGEST -Quiet -WarningAction SilentlyContinue}\n")?;
                command
                    .arg(script)
                    .arg("-Library")
                    .arg(self.root.join("scripts/lib/mirrors.ps1"))
                    .arg("-Destination")
                    .arg(&destination);
                if hash.is_none() {
                    command.arg("-NoHash");
                }
            }
        }
        let begin = Instant::now();
        self.stage.set("child-process");
        let output = crate::support::process_output(&mut command, None, Duration::from_secs(45))
            .map_err(|_| {
                anyhow!("controlled HTTP downloader process exceeded deadline or could not start")
            })?;
        let elapsed = begin.elapsed();
        self.stage.set("child-outcome");
        ensure!(
            output.status.success() == success,
            "controlled {:?} HTTP transfer outcome differs; private output withheld",
            if self.kind == Kind::Sh { "SH" } else { "PS" }
        );
        let text = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
        self.stage.set("log-privacy");
        ensure!(
            !text
                .windows(prefixes[0].len())
                .any(|window| window == prefixes[0].as_bytes()),
            "controlled downloader leaked a request URL"
        );
        for entry in fs::read_dir(temp.path())? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            ensure!(
                !name.contains(".part"),
                "partial file survived source cleanup"
            );
        }
        if success {
            self.stage.set("verified-content");
            if installer {
                ensure!(
                    temp.path()
                        .join("installed/bin/vpn233-provider-server.exe")
                        .is_file(),
                    "real installer did not install verified fixture"
                );
            } else {
                ensure!(
                    fs::read(&destination)? == payload,
                    "verified output bytes differ"
                );
            }
            ensure!(
                fs::read_to_string(&cache)?.trim()
                    == format!("release {}", prefixes.last().unwrap()),
                "unverified mirror was cached"
            );
        } else {
            self.stage.set("failure-preservation");
            ensure!(
                fs::read(&destination)? == b"synthetic prior destination",
                "failed transfer replaced previous destination"
            );
            if prior_cache {
                ensure!(
                    fs::read_to_string(cache)? == previous,
                    "failed transfer changed previous cache"
                );
            } else {
                ensure!(!cache.exists(), "failed mirror cached");
            }
        }
        self.stage.set("response-assertions");
        Ok(Transfer {
            requests: server.requests(),
            elapsed,
        })
    }
}

pub(super) fn run(args: &[String]) -> Result<()> {
    let root = resolve_root(args).inspect_err(|_| {
        eprintln!("mirror-http: setup failed during root-validation");
    })?;
    match option(args, "--shell")?.as_deref().unwrap_or("all") {
        "sh" => sh(&root),
        "ps" => ps(&root),
        "all" => {
            sh(&root)?;
            ps(&root)
        }
        _ => bail!("mirror shell must be sh, ps or all"),
    }
}
fn resolve_root(args: &[String]) -> Result<PathBuf> {
    let root = option(args, "--root")?
        .map(|value| {
            #[cfg(windows)]
            {
                // Git Bash can leave /c/... unconverted when the caller sets
                // MSYS2_ARG_CONV_EXCL. Normalize it before native filesystem IO.
                let bytes = value.as_bytes();
                if bytes.len() >= 3
                    && bytes[0] == b'/'
                    && bytes[1].is_ascii_alphabetic()
                    && bytes[2] == b'/'
                {
                    return PathBuf::from(format!("{}:{}", bytes[1] as char, &value[2..]));
                }
            }
            PathBuf::from(value)
        })
        .map(Ok)
        .unwrap_or_else(crate::support::root)?;
    let root = crate::support::canonical_path(&root)?;
    ensure!(
        root.join("scripts/lib/mirrors.sh").is_file()
            && root.join("scripts/lib/mirrors.ps1").is_file()
            && root.join("install.sh").is_file(),
        "HTTP verification root is not a complete source checkout"
    );
    Ok(root)
}

fn sh(root: &Path) -> Result<()> {
    let shell =
        program(&["bash"]).ok_or_else(|| anyhow!("Bash is required for real HTTP verification"))?;
    let case = Case {
        root,
        shell: &shell,
        kind: Kind::Sh,
        sequence: std::cell::Cell::new(0),
        stage: std::cell::Cell::new("initialization"),
        passed: std::cell::Cell::new(false),
    };
    let payload: Vec<_> = (0..256u16)
        .map(|n| n as u8)
        .cycle()
        .take(512 * 1024)
        .collect();
    let sha = digest(&payload);
    let transfer = |routes: &[&str], hash: &str, budget: &str, success, cache| {
        case.transfer(
            routes,
            &payload,
            TransferOptions {
                hash: if hash.is_empty() { None } else { Some(hash) },
                budget,
                success,
                prior_cache: cache,
                installer: false,
            },
        )
    };
    let mut count = 0;
    for (route, offsets) in [
        ("resume", vec![None, Some(65536)]),
        ("resume-twice", vec![None, Some(65536), Some(131072)]),
        ("chunked", vec![None, Some(65536)]),
        ("complete-eof", vec![None]),
    ] {
        let result = transfer(&[route], &sha, "5", true, false)?;
        ensure!(
            result.requests.iter().map(|r| r.offset).collect::<Vec<_>>() == offsets,
            "SH actual resume offsets differ"
        );
        count += 1;
    }
    for route in [
        "no-range",
        "bad-position",
        "missing-range",
        "range-416",
        "changed",
        "wrong",
    ] {
        let result = transfer(&[route, "good"], &sha, "5", true, false)?;
        ensure!(
            result
                .requests
                .last()
                .is_some_and(|r| r.route == "good" && r.offset.is_none()),
            "SH fallback reused another source partial"
        );
        if route == "wrong" {
            ensure!(result.requests.len() == 2, "SH retried complete wrong hash");
        }
        count += 1;
    }
    let result = transfer(&["bounded"], &sha, "5", false, false)?;
    ensure!(
        result.requests.iter().map(|r| r.offset).collect::<Vec<_>>()
            == [None, Some(65536), Some(131072)],
        "SH exceeded request bound"
    );
    count += 1;
    let result = transfer(&["slow"], &sha, "2", false, false)?;
    ensure!(
        result.requests.len() <= 3 && result.elapsed < Duration::from_millis(3200),
        "SH multiplied source time budget"
    );
    count += 1;
    ensure!(
        transfer(&["wrong"], &sha, "5", false, false)?
            .requests
            .len()
            == 1,
        "SH retried complete wrong bytes"
    );
    count += 1;
    transfer(&["wrong"], &sha, "5", false, true)?;
    count += 1;
    ensure!(
        transfer(&["resume", "good"], "", "5", true, false)?
            .requests
            .iter()
            .all(|r| r.offset.is_none()),
        "unverified SH transfer sent Range"
    );
    count += 1;
    for invalid in [
        "invalid".into(),
        format!("{sha} "),
        "0".repeat(63),
        "g".repeat(64),
    ] {
        ensure!(
            transfer(&["good"], &invalid, "5", false, false)?
                .requests
                .is_empty(),
            "invalid SH hash reached HTTP"
        );
        count += 1;
    }
    ensure!(
        transfer(&["good"], &sha, "0.0001", false, false)?
            .requests
            .is_empty(),
        "unsafe submillisecond timeout reached HTTP"
    );
    count += 1;
    for route in ["manifest-missing", "manifest-invalid", "manifest-duplicate"] {
        case.advance("manifest-ordering");
        // Manifest retrieval is intentionally unverified; failure must precede
        // archive selection and any installation directory mutation.
        let server = Server::new(Kind::Sh, payload.clone())?;
        let temp = tempfile::tempdir()?;
        let prefix = temp.path().join("installed");
        let mut command = clean_command(&shell);
        command
            .arg(path_arg(&root.join("install.sh")))
            .args([
                "--release",
                "--version",
                "2.1.0",
                "--no-start",
                "--yes",
                "--prefix",
            ])
            .arg(path_arg(&prefix))
            .args([
                "--mirror",
                &format!("http://127.0.0.1:{}/{route}/", server.port),
            ])
            .env("VPN233_HOME", temp.path().join("home"))
            .env("XDG_CONFIG_HOME", temp.path().join("xdg"));
        let result = crate::support::process_output(&mut command, None, Duration::from_secs(30))?;
        let requests = server.requests();
        ensure!(
            !result.status.success()
                && !prefix.exists()
                && requests.len() == 1
                && requests[0].path.ends_with("/SHA256SUMS"),
            "SH installer requested archive before valid manifest hash"
        );
        count += 1;
    }
    ensure!(count == 23, "SH HTTP coverage count differs");
    case.passed.set(true);
    println!("SH real loopback HTTP: {count} cases passed");
    Ok(())
}

fn ps(root: &Path) -> Result<()> {
    let Some(shell) = program(if cfg!(windows) {
        &["powershell", "pwsh"]
    } else {
        &["pwsh"]
    }) else {
        println!("SKIP PowerShell real HTTP: runtime unavailable on this host");
        return Ok(());
    };
    let case = Case {
        root,
        shell: &shell,
        kind: Kind::Ps,
        sequence: std::cell::Cell::new(0),
        stage: std::cell::Cell::new("initialization"),
        passed: std::cell::Cell::new(false),
    };
    let mut payload = b"synthetic archive fixture\0".to_vec();
    payload.extend((0..256u16).map(|n| n as u8).cycle().take(256 * 512));
    let sha = digest(&payload);
    let transfer = |routes: &[&str], hash: Option<&str>, budget: &str, success| {
        case.transfer(
            routes,
            &payload,
            TransferOptions {
                hash,
                budget,
                success,
                prior_cache: true,
                installer: false,
            },
        )
    };
    for route in ["resume", "reset", "chunked"] {
        let result = transfer(&[route], Some(&sha), "4", true)?;
        ensure!(
            result.requests.iter().map(|r| r.offset).collect::<Vec<_>>() == [None, Some(4093)],
            "PS resume offsets differ"
        );
        if route == "resume" {
            ensure!(
                result
                    .requests
                    .iter()
                    .all(|r| r.encoding.as_deref() == Some("identity")),
                "PS transfer encoding is not identity"
            );
        }
    }
    ensure!(
        transfer(&["resetafterfull"], Some(&sha), "4", true)?
            .requests
            .len()
            == 1,
        "PS retried complete verified transfer"
    );
    for route in [
        "wrong",
        "wrongrange",
        "wronglength",
        "changedtotal",
        "encoded",
        "oversized",
    ] {
        let result = transfer(&[route, "good"], Some(&sha), "4", true)?;
        ensure!(
            result.requests.last().is_some_and(|r| r.route == "good"),
            "PS failed to fall back"
        );
        ensure!(
            result.requests.iter().filter(|r| r.route == route).count()
                == if ["wrong", "encoded", "oversized"].contains(&route) {
                    1
                } else {
                    2
                },
            "PS malformed response retry count differs"
        );
    }
    ensure!(
        transfer(&["unavailable", "good"], None, "4", true)?
            .requests
            .iter()
            .all(|r| r.offset.is_none()),
        "unverified PS transfer sent Range"
    );
    ensure!(
        transfer(&["alwaysshort"], Some(&sha), "4", false)?
            .requests
            .iter()
            .map(|r| r.offset)
            .collect::<Vec<_>>()
            == [None, Some(4093), Some(8186)],
        "PS request bound differs"
    );
    for route in ["stall", "trickle"] {
        let result = transfer(&[route], Some(&sha), "1", false)?;
        ensure!(
            result.requests.len() == 1 && result.elapsed < Duration::from_secs(4),
            "PS source deadline multiplied"
        );
    }
    for invalid in [
        "not-a-sha256".into(),
        format!("{}\n", "a".repeat(64)),
        format!("{}\r\n", "a".repeat(64)),
        format!(" {}", "a".repeat(64)),
    ] {
        ensure!(
            transfer(&["good"], Some(&invalid), "4", false)?
                .requests
                .is_empty(),
            "invalid PS hash reached HTTP"
        );
    }
    case.stage.set("installer-fixture");
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for name in [
        "vpn233-provider-server",
        "vpn233-subscribe-server",
        "vpn233-client",
    ] {
        writer.start_file(format!("bin/{name}.exe"), options)?;
        writer.write_all(b"synthetic executable fixture")?;
    }
    for name in ["provider-server", "subscribe-server", "vpn233-client"] {
        writer.start_file(format!("examples/{name}.yaml"), options)?;
        writer.write_all(&fs::read(root.join(name).join("config.example.yaml"))?)?;
    }
    writer.start_file("version.txt", options)?;
    writer.write_all(b"2.1.0\n")?;
    writer.start_file("admin-web/dist/index.html", options)?;
    writer.write_all(b"<!doctype html><title>synthetic fixture</title>")?;
    let fixture = writer.finish()?.into_inner();
    let sha = digest(&fixture);
    let result = case.transfer(
        &["wrong", "good"],
        &fixture,
        TransferOptions {
            hash: Some(&sha),
            budget: "4",
            success: true,
            prior_cache: true,
            installer: true,
        },
    )?;
    ensure!(
        result.requests.iter().any(|r| r.route == "good"),
        "real PS installer did not verify manifest hash before archive selection"
    );
    case.passed.set(true);
    println!("PowerShell real loopback HTTP: 10 test groups passed (all original subcases)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_roots_normalize_before_native_file_access() {
        let root = crate::support::root().unwrap();
        assert_eq!(resolve_root(&[]).unwrap(), root);
        for variant in [root.clone(), root.join("scripts/tests/../..")] {
            assert_eq!(
                resolve_root(&["--root".into(), variant.to_string_lossy().into_owned()]).unwrap(),
                root
            );
        }
        #[cfg(windows)]
        {
            let native = path_arg(&root);
            if native.as_bytes().get(1) == Some(&b':') {
                let msys = format!("/{}{}", native.as_bytes()[0] as char, &native[2..]);
                assert_eq!(resolve_root(&["--root".into(), msys]).unwrap(), root);
            }
        }
        let incomplete = tempfile::tempdir().unwrap();
        assert!(resolve_root(&["--root".into(), path_arg(incomplete.path())]).is_err());
        assert!(resolve_root(&[
            "--root".into(),
            path_arg(&incomplete.path().join("missing"))
        ])
        .is_err());
    }
}
