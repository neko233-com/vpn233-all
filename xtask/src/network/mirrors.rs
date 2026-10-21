use super::{clean_command, option, path_arg, private_write, program};
use anyhow::{anyhow, bail, ensure, Result};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Cursor, Read, Write},
    net::{Ipv4Addr, Shutdown, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const CHILD_DEADLINE: Duration = Duration::from_secs(45);
const IO_POLL: Duration = Duration::from_millis(100);

struct ConnectionTrace {
    phase: &'static str,
    error: Option<io::ErrorKind>,
    header_bytes: usize,
    body_bytes: usize,
    begin: Instant,
    finished: Option<Duration>,
}
impl ConnectionTrace {
    fn new() -> Self {
        Self {
            phase: "accepted",
            error: None,
            header_bytes: 0,
            body_bytes: 0,
            begin: Instant::now(),
            finished: None,
        }
    }
}

// A nonblocking listening socket is needed for shutdown. Winsock accept inherits
// that mode, so reset each accepted stream explicitly. Short socket polls let
// stop interrupt idle clients without imposing an earlier fixture-only deadline.
struct FixtureStream<'a> {
    stream: TcpStream,
    stop: &'a AtomicBool,
    trace: &'a Mutex<ConnectionTrace>,
    deadline: Instant,
}
impl<'a> FixtureStream<'a> {
    fn new(
        stream: TcpStream,
        stop: &'a AtomicBool,
        trace: &'a Mutex<ConnectionTrace>,
        deadline: Instant,
    ) -> io::Result<Self> {
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(IO_POLL))?;
        stream.set_write_timeout(Some(IO_POLL))?;
        Ok(Self {
            stream,
            stop,
            trace,
            deadline,
        })
    }
    fn check_deadline(&self) -> io::Result<()> {
        if self.stop.load(Ordering::Relaxed) || Instant::now() >= self.deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        Ok(())
    }
    fn phase(&self, phase: &'static str) {
        self.trace.lock().unwrap().phase = phase;
    }
}
impl Read for FixtureStream<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            self.check_deadline()?;
            match self.stream.read(buffer) {
                Ok(count) => {
                    self.trace.lock().unwrap().header_bytes += count;
                    return Ok(count);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error),
            }
        }
    }
}
impl Write for FixtureStream<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        loop {
            self.check_deadline()?;
            match self.stream.write(buffer) {
                Ok(count) => {
                    let mut trace = self.trace.lock().unwrap();
                    if trace.phase == "response-body" {
                        trace.body_bytes += count;
                    }
                    return Ok(count);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error),
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Sh,
    Ps,
}
#[derive(Clone, Debug)]
struct Request {
    parsed_at: Instant,
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
    connections: Arc<Mutex<Vec<Arc<Mutex<ConnectionTrace>>>>>,
    deadline: Arc<Mutex<Instant>>,
    listener_error: Arc<Mutex<Option<io::ErrorKind>>>,
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
        let connections = Arc::new(Mutex::new(Vec::new()));
        let traces = connections.clone();
        let deadline = Arc::new(Mutex::new(Instant::now() + CHILD_DEADLINE));
        let expiration = deadline.clone();
        let listener_error = Arc::new(Mutex::new(None));
        let accept_error = listener_error.clone();
        let body = Arc::new(payload);
        let worker = thread::spawn(move || {
            let mut clients = Vec::new();
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let trace = Arc::new(Mutex::new(ConnectionTrace::new()));
                        traces.lock().unwrap().push(trace.clone());
                        let records = records.clone();
                        let stopping = stopping.clone();
                        let body = body.clone();
                        let deadline = *expiration.lock().unwrap();
                        clients.push(thread::spawn(move || {
                            let result = FixtureStream::new(stream, &stopping, &trace, deadline)
                                .map_err(anyhow::Error::from)
                                .and_then(|stream| serve(stream, kind, &body, &records, &stopping));
                            let mut trace = trace.lock().unwrap();
                            if let Err(error) = result {
                                trace.error = Some(
                                    error
                                        .downcast_ref::<io::Error>()
                                        .map_or(io::ErrorKind::InvalidData, io::Error::kind),
                                );
                            } else {
                                trace.phase = "complete";
                            }
                            trace.finished = Some(trace.begin.elapsed());
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => {
                        *accept_error.lock().unwrap() = Some(error.kind());
                        break;
                    }
                }
            }
            clients
        });
        Ok(Self {
            port,
            requests,
            stop,
            worker: Some(worker),
            connections,
            deadline,
            listener_error,
        })
    }
    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
    fn begin_child(&self) -> Instant {
        let begin = Instant::now();
        *self.deadline.lock().unwrap() = begin + CHILD_DEADLINE;
        begin
    }
    fn report(&self) {
        let connections = self.connections.lock().unwrap();
        let requests = self.requests();
        eprintln!(
            "mirror-http: server connections={} requests={} listener-error={:?}",
            connections.len(),
            requests.len(),
            *self.listener_error.lock().unwrap()
        );
        for (index, trace) in connections.iter().enumerate() {
            let trace = trace.lock().unwrap();
            eprintln!("mirror-http: connection {:02} phase={} error={:?} header-bytes={} body-bytes={} elapsed-ms={}", index + 1, trace.phase, trace.error, trace.header_bytes, trace.body_bytes, trace.finished.unwrap_or_else(|| trace.begin.elapsed()).as_millis());
        }
        for (index, request) in requests.iter().enumerate() {
            let route = controlled_route(&request.route);
            eprintln!(
                "mirror-http: request {:02} route={} offset={:?} identity={}",
                index + 1,
                route,
                request.offset,
                request.encoding.as_deref() == Some("identity")
            );
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            if let Ok(clients) = worker.join() {
                for client in clients {
                    if client.join().is_err() {
                        eprintln!("mirror-http: fixture client worker panicked");
                    }
                }
            } else {
                eprintln!("mirror-http: fixture listener worker panicked");
            }
        }
    }
}

fn controlled_route(route: &str) -> &'static str {
    match route {
        "resume" => "resume",
        "resume-twice" => "resume-twice",
        "chunked" => "chunked",
        "complete-eof" => "complete-eof",
        "no-range" => "no-range",
        "bad-position" => "bad-position",
        "missing-range" => "missing-range",
        "range-416" => "range-416",
        "changed" => "changed",
        "wrong" => "wrong",
        "good" => "good",
        "bounded" => "bounded",
        "slow" => "slow",
        "reset" => "reset",
        "resetafterfull" => "resetafterfull",
        "wrongrange" => "wrongrange",
        "wronglength" => "wronglength",
        "changedtotal" => "changedtotal",
        "encoded" => "encoded",
        "oversized" => "oversized",
        "unavailable" => "unavailable",
        "alwaysshort" => "alwaysshort",
        "stall" => "stall",
        "trickle" => "trickle",
        _ => "unknown",
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
    mut stream: FixtureStream<'_>,
    kind: Kind,
    payload: &[u8],
    records: &Mutex<Vec<Request>>,
    stop: &AtomicBool,
) -> Result<()> {
    stream.phase("request-headers");
    let mut header = Vec::new();
    while !header.windows(4).any(|part| part == b"\r\n\r\n") {
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
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
        parsed_at: Instant::now(),
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
        stream.phase("response-headers");
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
    stream.phase("response-headers");
    stream.write_all(response.as_bytes())?;
    stream.phase("response-body");
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
    let _ = stream.stream.shutdown(Shutdown::Both);
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
    child_elapsed: Duration,
    request_elapsed: Option<Duration>,
}

fn check_source_budget(
    transfer: &Transfer,
    route: &str,
    min_requests: usize,
    max_requests: usize,
    limit: Duration,
) -> Result<()> {
    let valid = (min_requests..=max_requests).contains(&transfer.requests.len())
        && transfer
            .requests
            .iter()
            .all(|request| request.route == route)
        && transfer
            .request_elapsed
            .is_some_and(|elapsed| elapsed < limit);
    if !valid {
        let window = transfer
            .request_elapsed
            .map(|elapsed| elapsed.as_millis().to_string())
            .unwrap_or_else(|| "-1".into());
        eprintln!(
            "mirror-http: source-budget requests={} request-window-ms={} child-ms={}",
            transfer.requests.len(),
            window,
            transfer.child_elapsed.as_millis()
        );
        bail!("controlled source request count, route or budget window differs");
    }
    Ok(())
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
            let script = temp.path().join("receive.ps1");
            private_write(&script, br#"param([string]$Library,[string]$Destination,[switch]$NoHash,[string]$Installer,[string]$Prefix)
$ErrorActionPreference='Stop'
[Console]::Out.WriteLine(('VPN233_HTTP_RUNTIME=' + $PSVersionTable.PSVersion.ToString()))
[Console]::Out.WriteLine('VPN233_HTTP_STAGE=library')
if($Installer){
    [Console]::Out.WriteLine('VPN233_HTTP_STAGE=installer')
    & $Installer -Release -Version '2.1.0' -Prefix $Prefix -NoStart -Mirror auto
}else{
    . $Library
    [Console]::Out.WriteLine('VPN233_HTTP_STAGE=transfer')
    if($NoHash){Receive-Vpn233ReleaseAsset fixture.bin $Destination '2.1.0' -Quiet -WarningAction SilentlyContinue}else{Receive-Vpn233ReleaseAsset fixture.bin $Destination '2.1.0' -ExpectedSha256 $env:VPN233_TEST_DIGEST -Quiet -WarningAction SilentlyContinue}
}
[Console]::Out.WriteLine('VPN233_HTTP_STAGE=complete')
"#)?;
            command.arg(script);
            if installer {
                command
                    .arg("-Installer")
                    .arg(self.root.join("install.ps1"))
                    .arg("-Prefix")
                    .arg(temp.path().join("installed"));
            } else {
                command
                    .arg("-Library")
                    .arg(self.root.join("scripts/lib/mirrors.ps1"))
                    .arg("-Destination")
                    .arg(&destination);
                if hash.is_none() {
                    command.arg("-NoHash");
                }
            }
        }
        let begin = server.begin_child();
        self.stage.set("child-process");
        let output = match crate::support::process_output(&mut command, None, CHILD_DEADLINE) {
            Ok(output) => output,
            Err(_) => {
                eprintln!(
                    "mirror-http: child execution failed elapsed-ms={}",
                    begin.elapsed().as_millis()
                );
                server.report();
                bail!("controlled HTTP downloader process exceeded deadline or could not start");
            }
        };
        let finished = Instant::now();
        let child_elapsed = finished.duration_since(begin);
        let requests = server.requests();
        // Use the earliest parsed request rather than worker insertion order.
        // No request, including one parsed after child completion, has no window.
        let request_elapsed = requests
            .iter()
            .map(|request| request.parsed_at)
            .min()
            .and_then(|first| finished.checked_duration_since(first));
        let result = (|| {
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
                requests,
                child_elapsed,
                request_elapsed,
            })
        })();
        if result.is_err() {
            let text = String::from_utf8_lossy(&output.stdout);
            let runtime = text
                .lines()
                .filter_map(|line| line.strip_prefix("VPN233_HTTP_RUNTIME="))
                .find(|value| {
                    !value.is_empty()
                        && value.len() <= 64
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || byte == b'.')
                })
                .unwrap_or("unavailable");
            let stage = text
                .lines()
                .filter_map(|line| line.strip_prefix("VPN233_HTTP_STAGE="))
                .rfind(|value| ["library", "transfer", "installer", "complete"].contains(value))
                .unwrap_or("unavailable");
            eprintln!("mirror-http: child runtime={} phase={} exit={:?} expected-success={} elapsed-ms={} stdout-bytes={} stderr-bytes={}", runtime, stage, output.status.code(), success, child_elapsed.as_millis(), output.stdout.len(), output.stderr.len());
            server.report();
        }
        result
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
    check_source_budget(&result, "slow", 1, 3, Duration::from_millis(3200))?;
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
        check_source_budget(&result, route, 1, 1, Duration::from_secs(4))?;
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
    fn source_budget_rejects_an_unrecorded_request_window() {
        let transfer = Transfer {
            requests: vec![Request {
                parsed_at: Instant::now(),
                route: "slow".into(),
                offset: None,
                encoding: Some("identity".into()),
                path: "/slow/fixture".into(),
            }],
            child_elapsed: Duration::from_secs(1),
            request_elapsed: None,
        };
        assert!(check_source_budget(&transfer, "slow", 1, 3, Duration::from_millis(3200)).is_err());
    }

    #[cfg(target_os = "linux")]
    fn actual_slow_shell_transfer(root: &Path, shell: &Path) -> Result<Transfer> {
        let case = Case {
            root,
            shell,
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
        let transfer = case.transfer(
            &["slow"],
            &payload,
            TransferOptions {
                hash: Some(&sha),
                budget: "2",
                success: false,
                prior_cache: false,
                installer: false,
            },
        )?;
        case.passed.set(true);
        Ok(transfer)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn actual_shell_startup_delay_does_not_consume_the_source_request_window() {
        use std::os::unix::fs::PermissionsExt;

        let root = crate::support::root().unwrap();
        let private = tempfile::tempdir().unwrap();
        crate::network::protect_private_directory(private.path()).unwrap();
        let shell = private.path().join("delayed-shell");
        private_write(
            &shell,
            b"#!/bin/sh\n/bin/sleep 1.5\nexec /bin/bash \"$@\"\n",
        )
        .unwrap();
        fs::set_permissions(&shell, fs::Permissions::from_mode(0o700)).unwrap();
        let transfer = actual_slow_shell_transfer(&root, &shell).unwrap();
        assert!(
            transfer.child_elapsed > Duration::from_millis(3200),
            "real delayed shell must exceed the former whole-child threshold"
        );
        check_source_budget(&transfer, "slow", 1, 3, Duration::from_millis(3200)).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn actual_per_attempt_budget_reset_is_rejected_by_the_source_request_window() {
        let root = crate::support::root().unwrap();
        let source = fs::read_to_string(root.join("scripts/lib/mirrors.sh"))
            .unwrap()
            .replace("\r\n", "\n");
        const ATTEMPT: &str = "    attempt=$((attempt + 1))";
        assert!(
            source.matches(ATTEMPT).count() == 1,
            "controlled budget-reset mutation requires one retry increment"
        );
        let mutated = source.replacen(
            ATTEMPT,
            "    remaining=\"$VPN233_GH_MAX_TIME\"\n    attempt=$((attempt + 1))",
            1,
        );
        let private = tempfile::tempdir().unwrap();
        crate::network::protect_private_directory(private.path()).unwrap();
        fs::create_dir_all(private.path().join("scripts/lib")).unwrap();
        private_write(
            &private.path().join("scripts/lib/mirrors.sh"),
            mutated.as_bytes(),
        )
        .unwrap();
        let transfer = actual_slow_shell_transfer(private.path(), Path::new("/bin/bash")).unwrap();
        assert!(
            transfer.requests.len() == 3
                && transfer
                    .requests
                    .iter()
                    .all(|request| request.route == "slow")
                && transfer
                    .request_elapsed
                    .is_some_and(|elapsed| elapsed >= Duration::from_millis(3200)),
            "real per-attempt reset must expose three delayed requests beyond the shared budget"
        );
        assert!(check_source_budget(&transfer, "slow", 1, 3, Duration::from_millis(3200)).is_err());
    }

    fn wait_for_connection(server: &Server) {
        let begin = Instant::now();
        while server.connections.lock().unwrap().is_empty() {
            assert!(
                begin.elapsed() < Duration::from_secs(5),
                "loopback connection was not accepted"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn accepted_connections_wait_for_delayed_headers_within_client_budget() {
        // 100ms exposes inherited Winsock nonblocking mode; 3200ms also
        // exceeds the former fixture-only timeout while fitting the real 4s.
        for delay in [100, 3200] {
            let payload = b"synthetic delayed-header fixture";
            let server = Server::new(Kind::Ps, payload.to_vec()).unwrap();
            let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port)).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(4)))
                .unwrap();
            wait_for_connection(&server);
            let begin = Instant::now();
            thread::sleep(Duration::from_millis(delay));
            client
                .write_all(
                    b"GET /good/fixture HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            let mut response = Vec::new();
            client.read_to_end(&mut response).unwrap();
            assert!(
                begin.elapsed() < Duration::from_secs(4),
                "valid delayed headers exceeded client budget"
            );
            assert!(response.starts_with(b"HTTP/1.1 200"));
            assert!(response.ends_with(payload));
            assert_eq!(server.requests().len(), 1);
        }
    }

    #[test]
    fn stopping_fixture_interrupts_an_accepted_idle_connection() {
        let server = Server::new(Kind::Ps, b"synthetic idle fixture".to_vec()).unwrap();
        let _client = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port)).unwrap();
        wait_for_connection(&server);
        let begin = Instant::now();
        drop(server);
        assert!(
            begin.elapsed() < Duration::from_secs(1),
            "fixture stop did not interrupt idle IO"
        );
    }

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
