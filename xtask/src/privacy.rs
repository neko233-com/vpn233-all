//! Offline privacy inspection. Matching values and Git diagnostics are never printed.
use anyhow::{anyhow, bail, Context, Result};
use regex::{bytes::Regex as ByteRegex, Regex};
use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, Ipv6Addr},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{mpsc, Arc, LazyLock, Mutex},
    thread,
    time::Duration,
};

pub const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;
static RULES: LazyLock<Vec<(&str, ByteRegex)>> = LazyLock::new(|| {
    [
        ("github-token", r"(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})"),
        ("aws-access-key", r"(?:AKIA|ASIA)[A-Z0-9]{16}"),
        ("slack-token", r"xox[baprs]-[A-Za-z0-9-]{10,}"),
        ("private-key", r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----"),
        ("machine-home-path", r#"(?:[A-Za-z]:[\\/](?:Users|Documents and Settings)[\\/][^\s\\/'"<>]+|/(?:home|Users)/[^\s/'"<>]+)"#),
        ("credential-in-url", r#"https?://[^\s/@:'"]+:[^\s/@'"]+@(?P<host>[A-Za-z0-9.-]+)"#),
    ].into_iter().map(|(name, pattern)| {
        (name, regex::bytes::RegexBuilder::new(pattern).unicode(false).build().expect("constant privacy pattern"))
    }).collect()
});
static IPV4: LazyLock<ByteRegex> =
    LazyLock::new(|| ByteRegex::new(r"(?:[0-9]{1,3}\.){3}[0-9]{1,3}").unwrap());
static IPV6: LazyLock<ByteRegex> =
    LazyLock::new(|| ByteRegex::new(r"(?:[A-Fa-f0-9]{1,4}:){2,}[A-Fa-f0-9:]+").unwrap());
static PATH_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[MmZzLlHhVvCcSsQqTtAa]|[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?")
        .unwrap()
});
const DNS4: &[&str] = &[
    "1.1.1.1",
    "1.0.0.1",
    "8.8.8.8",
    "8.8.4.4",
    "9.9.9.9",
    "149.112.112.112",
    "223.5.5.5",
    "223.6.6.6",
    "119.29.29.29",
    "114.114.114.114",
    "114.114.115.115",
];
const DNS6: &[&str] = &[
    "2606:4700:4700::1111",
    "2606:4700:4700::1001",
    "2001:4860:4860::8888",
    "2001:4860:4860::8844",
];

pub fn sensitive_path(name: &str) -> bool {
    let normalized = name.replace('\\', "/");
    let parts: Vec<_> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let Some(last) = parts.last() else {
        return false;
    };
    let last = last.to_ascii_lowercase();
    parts[..parts.len() - 1].iter().any(|part| {
        matches!(
            part.to_ascii_lowercase().as_str(),
            "config" | "data" | "runtime" | "logs" | "backups" | "secrets" | ".ssh"
        )
    }) || matches!(
        last.as_str(),
        "config.yaml" | "config.json" | "links.txt" | "node-manifest.json" | ".vpn233-layout"
    ) || (last.starts_with(".env") && last != ".env.example")
        || [
            ".pem",
            ".key",
            ".ppk",
            ".p12",
            ".pfx",
            ".log",
            ".session",
            ".install.sh",
            ".install.ps1",
            ".credentials",
            ".secret",
        ]
        .iter()
        .any(|suffix| last.ends_with(suffix))
}

fn prefix4(value: u32, base: [u8; 4], bits: u32) -> bool {
    value >> (32 - bits) == u32::from_be_bytes(base) >> (32 - bits)
}
/// Python's previous is_global contract includes the IANA exceptions, not just RFC1918.
pub fn global_ipv4(ip: Ipv4Addr) -> bool {
    let value = u32::from(ip);
    if [
        u32::from_be_bytes([192, 0, 0, 9]),
        u32::from_be_bytes([192, 0, 0, 10]),
    ]
    .contains(&value)
    {
        return true;
    }
    ![
        ([0, 0, 0, 0], 8),
        ([10, 0, 0, 0], 8),
        ([100, 64, 0, 0], 10),
        ([127, 0, 0, 0], 8),
        ([169, 254, 0, 0], 16),
        ([172, 16, 0, 0], 12),
        ([192, 0, 0, 0], 24),
        ([192, 0, 2, 0], 24),
        ([192, 168, 0, 0], 16),
        ([198, 18, 0, 0], 15),
        ([198, 51, 100, 0], 24),
        ([203, 0, 113, 0], 24),
        ([240, 0, 0, 0], 4),
    ]
    .iter()
    .any(|(base, bits)| prefix4(value, *base, *bits))
}
fn prefix6(value: u128, base: u128, bits: u32) -> bool {
    value >> (128 - bits) == base >> (128 - bits)
}
pub fn global_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return global_ipv4(mapped);
    }
    let value = u128::from(ip);
    let exceptions = [
        (0x2001_0001_0000_0000_0000_0000_0000_0001, 128),
        (0x2001_0001_0000_0000_0000_0000_0000_0002, 128),
        (0x2001_0003_0000_0000_0000_0000_0000_0000, 32),
        (0x2001_0004_0112_0000_0000_0000_0000_0000, 48),
        (0x2001_0020_0000_0000_0000_0000_0000_0000, 28),
        (0x2001_0030_0000_0000_0000_0000_0000_0000, 28),
    ];
    if exceptions
        .iter()
        .any(|(base, bits)| prefix6(value, *base, *bits))
    {
        return true;
    }
    ![
        (0, 128),
        (1, 128),
        (0x0064_ff9b_0001_0000_0000_0000_0000_0000, 48),
        (0x0100_0000_0000_0000_0000_0000_0000_0000, 64),
        (0x2001_0000_0000_0000_0000_0000_0000_0000, 23),
        (0x2001_0db8_0000_0000_0000_0000_0000_0000, 32),
        (0x2002_0000_0000_0000_0000_0000_0000_0000, 16),
        (0x3fff_0000_0000_0000_0000_0000_0000_0000, 20),
        (0xfc00_0000_0000_0000_0000_0000_0000_0000, 7),
        (0xfe80_0000_0000_0000_0000_0000_0000_0000, 10),
    ]
    .iter()
    .any(|(base, bits)| prefix6(value, *base, *bits))
}

fn secrets(data: &[u8], found: &mut BTreeSet<String>) {
    for (name, regex) in RULES.iter() {
        for capture in regex.captures_iter(data) {
            if *name == "credential-in-url" {
                let host = capture
                    .name("host")
                    .unwrap()
                    .as_bytes()
                    .to_ascii_lowercase();
                if host == b"example.invalid" || host.ends_with(b".example.invalid") {
                    continue;
                }
            }
            found.insert((*name).into());
        }
    }
}
fn ips(data: &[u8], found: &mut BTreeSet<String>) {
    for matched in IPV4.find_iter(data) {
        let boundary = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.';
        if matched.start() > 0 && boundary(data[matched.start() - 1])
            || matched.end() < data.len() && boundary(data[matched.end()])
        {
            continue;
        }
        let value = std::str::from_utf8(matched.as_bytes()).unwrap();
        if value.parse::<Ipv4Addr>().is_ok_and(global_ipv4) && !DNS4.contains(&value) {
            found.insert("non-example-public-ipv4".into());
        }
    }
    for matched in IPV6.find_iter(data) {
        let boundary = |byte: u8| byte.is_ascii_alphanumeric() || byte == b':';
        if matched.start() > 0 && boundary(data[matched.start() - 1])
            || matched.end() < data.len() && boundary(data[matched.end()])
        {
            continue;
        }
        if let Ok(ip) = std::str::from_utf8(matched.as_bytes())
            .unwrap()
            .parse::<Ipv6Addr>()
        {
            if global_ipv6(ip) && !DNS6.contains(&ip.to_string().as_str()) {
                found.insert("non-example-public-ipv6".into());
            }
        }
    }
}

pub fn valid_svg_path(value: &str) -> bool {
    let mut tokens = Vec::new();
    let mut offset = 0;
    for matched in PATH_TOKEN.find_iter(value) {
        if !value[offset..matched.start()]
            .chars()
            .all(|c| c.is_whitespace() || c == ',')
        {
            return false;
        }
        tokens.push(matched.as_str());
        offset = matched.end();
    }
    if !value[offset..]
        .chars()
        .all(|c| c.is_whitespace() || c == ',')
        || !matches!(tokens.first(), Some(&"M" | &"m"))
    {
        return false;
    }
    fn arity(token: &str) -> Option<usize> {
        match token {
            "M" | "m" | "L" | "l" | "T" | "t" => Some(2),
            "H" | "h" | "V" | "v" => Some(1),
            "C" | "c" => Some(6),
            "S" | "s" | "Q" | "q" => Some(4),
            "A" | "a" => Some(7),
            "Z" | "z" => Some(0),
            _ => None,
        }
    }
    let mut index = 0;
    while index < tokens.len() {
        let command = tokens[index];
        let Some(count) = arity(command) else {
            return false;
        };
        index += 1;
        let start = index;
        while index < tokens.len() && arity(tokens[index]).is_none() {
            index += 1;
        }
        let numbers = index - start;
        if count == 0 && numbers != 0 || count != 0 && (numbers == 0 || numbers % count != 0) {
            return false;
        }
        if matches!(command, "A" | "a") {
            for point in (start..index).step_by(7) {
                if !matches!(tokens[point + 3], "0" | "1")
                    || !matches!(tokens[point + 4], "0" | "1")
                {
                    return false;
                }
            }
        }
    }
    true
}

pub fn content_rules(data: &[u8]) -> Vec<String> {
    let mut found = BTreeSet::new();
    secrets(data, &mut found);
    if !data[..data.len().min(8192)].contains(&0) {
        let mut svg = false;
        if data
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace())
            == Some(b'<')
        {
            if let Ok(text) = std::str::from_utf8(data) {
                if let Ok(document) = roxmltree::Document::parse_with_options(
                    text,
                    roxmltree::ParsingOptions {
                        nodes_limit: 100_000,
                        ..Default::default()
                    },
                ) {
                    let root = document.root_element();
                    let svg_name = |node: roxmltree::Node<'_, '_>| {
                        node.is_element()
                            && node.tag_name().name() == "svg"
                            && matches!(
                                node.tag_name().namespace(),
                                None | Some("http://www.w3.org/2000/svg")
                            )
                    };
                    if svg_name(root) {
                        svg = true;
                        for node in document.descendants() {
                            if let Some(value) = node.text().filter(|_| !node.is_element()) {
                                secrets(value.as_bytes(), &mut found);
                                ips(value.as_bytes(), &mut found);
                            }
                            if node.is_element() {
                                for namespace in node.namespaces() {
                                    secrets(namespace.uri().as_bytes(), &mut found);
                                    ips(namespace.uri().as_bytes(), &mut found);
                                }
                                for attribute in node.attributes() {
                                    let value = attribute.value();
                                    secrets(value.as_bytes(), &mut found);
                                    let geometry = node.tag_name().name() == "path"
                                        && matches!(
                                            node.tag_name().namespace(),
                                            None | Some("http://www.w3.org/2000/svg")
                                        )
                                        && attribute.name() == "d"
                                        && attribute.namespace().is_none()
                                        && valid_svg_path(value);
                                    if !geometry {
                                        ips(value.as_bytes(), &mut found);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if !svg {
            ips(data, &mut found);
        }
    }
    found.into_iter().collect()
}

pub fn inspect_bytes(name: &str, data: &[u8], path_gate: bool) -> Vec<String> {
    let mut found: BTreeSet<String> = content_rules(data).into_iter().collect();
    let canonical = name
        .trim_start_matches("index:")
        .trim_start_matches("history:")
        .trim_start_matches("artifact:");
    found.extend(content_rules(canonical.as_bytes()));
    if path_gate && sensitive_path(canonical) {
        found.insert("sensitive-file-path".into());
    }
    if canonical.to_ascii_lowercase().ends_with(".svg") {
        let valid = std::str::from_utf8(data)
            .ok()
            .and_then(|text| {
                roxmltree::Document::parse_with_options(
                    text,
                    roxmltree::ParsingOptions {
                        nodes_limit: 100_000,
                        ..Default::default()
                    },
                )
                .ok()
            })
            .is_some_and(|document| {
                let root = document.root_element();
                root.tag_name().name() == "svg"
                    && matches!(
                        root.tag_name().namespace(),
                        None | Some("http://www.w3.org/2000/svg")
                    )
            });
        if !valid {
            found.insert("invalid-svg-document".into());
        }
    }
    found.into_iter().collect()
}

fn git(repository: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = crate::support::process_output(
        Command::new("git").arg("-C").arg(repository).args(args),
        None,
        Duration::from_secs(60),
    )
    .map_err(|_| anyhow!("privacy Git command could not complete"))?;
    if !output.status.success() {
        bail!("privacy Git command failed; diagnostics redacted");
    }
    Ok(output.stdout)
}

/// One Git process, one bounded object at a time: requests never fill stdin
/// while the same thread is blocked waiting for a large stdout response.
struct ObjectReader {
    child: Arc<Mutex<Child>>,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    stop: Option<mpsc::Sender<()>>,
    watchdog: Option<thread::JoinHandle<()>>,
}
impl ObjectReader {
    fn new(repository: &Path) -> Result<Self> {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("privacy object reader could not start")?;
        let input = child.stdin.take();
        let output = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| anyhow!("privacy object reader pipe unavailable"))?,
        );
        let child = Arc::new(Mutex::new(child));
        let control = Arc::clone(&child);
        let (stop, receiver) = mpsc::channel();
        let watchdog = thread::spawn(move || {
            if receiver.recv_timeout(Duration::from_secs(300)).is_err() {
                if let Ok(mut child) = control.lock() {
                    let _ = child.kill();
                }
            }
        });
        Ok(Self {
            child,
            input,
            output,
            stop: Some(stop),
            watchdog: Some(watchdog),
        })
    }
    fn read(&mut self, oid: &str) -> Result<(String, Vec<u8>)> {
        if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("invalid Git object identity");
        }
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| anyhow!("Git object reader closed"))?;
        writeln!(input, "{oid}")?;
        input.flush()?;
        let mut header = Vec::new();
        self.output
            .by_ref()
            .take(512)
            .read_until(b'\n', &mut header)?;
        if header.last() != Some(&b'\n') {
            bail!("invalid Git object header");
        }
        let header =
            std::str::from_utf8(&header).map_err(|_| anyhow!("invalid Git object header"))?;
        let fields: Vec<_> = header.split_whitespace().collect();
        if fields.len() != 3
            || fields[0] != oid
            || !matches!(fields[1], "blob" | "tree" | "commit" | "tag")
        {
            bail!("missing or invalid Git object");
        }
        let size: u64 = fields[2]
            .parse()
            .map_err(|_| anyhow!("invalid Git object length"))?;
        if size > MAX_INPUT_BYTES {
            bail!("Git object exceeds privacy input bound");
        }
        let mut data = vec![0; size as usize];
        self.output.read_exact(&mut data)?;
        let mut terminator = [0];
        self.output.read_exact(&mut terminator)?;
        if terminator != *b"\n" {
            bail!("invalid Git object terminator");
        }
        Ok((fields[1].into(), data))
    }
    fn finish(&mut self) -> Result<()> {
        self.input.take();
        let started = std::time::Instant::now();
        loop {
            match self
                .child
                .lock()
                .map_err(|_| anyhow!("Git object reader lock failed"))?
                .try_wait()?
            {
                Some(status) => {
                    if !status.success() {
                        bail!("Git object reader failed; diagnostics redacted");
                    }
                    return Ok(());
                }
                None if started.elapsed() < Duration::from_secs(5) => {}
                _ => bail!("Git object reader shutdown exceeded deadline"),
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for ObjectReader {
    fn drop(&mut self) {
        self.input.take();
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(watchdog) = self.watchdog.take() {
            let _ = watchdog.join();
        }
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Default)]
struct Report {
    count: usize,
    findings: BTreeSet<(String, String)>,
}
impl Report {
    fn inspect(&mut self, name: String, data: &[u8], path_gate: bool) {
        self.count += 1;
        for rule in inspect_bytes(&name, data, path_gate) {
            self.findings.insert((name.clone(), rule));
        }
    }
    fn finding(&mut self, name: String, rule: &str) {
        self.findings.insert((name, rule.into()));
    }
}
fn read_file(path: &Path) -> Result<Vec<u8>> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000);
    }
    let file = options
        .open(path)
        .map_err(|_| anyhow!("privacy input could not be opened"))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("privacy input must be a regular file");
    }
    let mut data = Vec::new();
    file.take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|_| anyhow!("privacy input could not be read"))?;
    if data.len() as u64 > MAX_INPUT_BYTES {
        bail!("privacy input exceeds size bound");
    }
    Ok(data)
}
fn path_text(bytes: &[u8]) -> Result<String> {
    String::from_utf8(bytes.to_vec())
        .map_err(|_| anyhow!("non-UTF8 Git path; refusing incomplete scan"))
}
fn has_symlink_component(root: &Path, name: &str) -> Result<bool> {
    let mut path = root.to_path_buf();
    for component in Path::new(name).components() {
        let std::path::Component::Normal(component) = component else {
            bail!("invalid Git candidate path");
        };
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(_) => bail!("candidate metadata unavailable"),
        }
    }
    Ok(false)
}
fn artifact_tree(tree: &Path, current: &Path, report: &mut Report) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(current)
        .context("artifact directory unavailable")?
        .collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let name = format!(
            "artifact:{}",
            path.strip_prefix(tree)?
                .to_string_lossy()
                .replace('\\', "/")
        );
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            report.finding(name, "artifact-symlink");
        } else if metadata.is_dir() {
            artifact_tree(tree, &path, report)?;
        } else if metadata.is_file() {
            report.inspect(name, &read_file(&path)?, true);
        } else {
            report.finding(name, "artifact-special-file");
        }
    }
    Ok(())
}

pub fn run(args: &[String]) -> Result<()> {
    let mut repository = None;
    let mut artifacts = Vec::new();
    let mut history = false;
    let mut history_only = false;
    let mut artifacts_only = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--repository" | "--artifacts" => {
                let flag = &args[index];
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| anyhow!("privacy option requires a path"))?;
                if flag == "--repository" {
                    repository = Some(PathBuf::from(value));
                } else {
                    artifacts.push(PathBuf::from(value));
                }
            }
            "--history" => history = true,
            "--history-only" => history_only = true,
            "--artifacts-only" => artifacts_only = true,
            _ => bail!("unsupported privacy option"),
        }
        index += 1;
    }
    if artifacts_only && artifacts.is_empty() {
        bail!("artifacts-only requires at least one public tree");
    }
    let mut report = Report::default();
    if !artifacts_only {
        let repository = match repository {
            Some(value) => crate::support::canonical_path(&value)?,
            None => crate::support::root()?,
        };
        let mut objects = ObjectReader::new(&repository)?;
        if !history_only {
            let candidates = git(
                &repository,
                &[
                    "ls-files",
                    "-z",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                ],
            )?;
            let paths: BTreeSet<_> = candidates
                .split(|b| *b == 0)
                .filter(|p| !p.is_empty())
                .map(path_text)
                .collect::<Result<_>>()?;
            for name in paths {
                if has_symlink_component(&repository, &name)? {
                    report.finding(name, "repository-symlink");
                    continue;
                }
                let path = repository.join(&name);
                match fs::symlink_metadata(&path) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        report.finding(name, "repository-symlink")
                    }
                    Ok(metadata) if metadata.is_file() => {
                        report.inspect(name, &read_file(&path)?, true)
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => bail!("candidate metadata unavailable"),
                }
            }
            let entries = git(&repository, &["ls-files", "--stage", "-z"])?;
            for entry in entries.split(|b| *b == 0).filter(|entry| !entry.is_empty()) {
                let split = entry
                    .iter()
                    .position(|b| *b == b'\t')
                    .ok_or_else(|| anyhow!("invalid Git index entry"))?;
                let fields: Vec<_> = std::str::from_utf8(&entry[..split])?
                    .split_whitespace()
                    .collect();
                if fields.len() != 3 {
                    bail!("invalid Git index entry");
                }
                let (_, data) = objects.read(fields[1])?;
                report.inspect(
                    format!("index:{}", path_text(&entry[split + 1..])?),
                    &data,
                    true,
                );
            }
        }
        if history || history_only {
            let entries = git(&repository, &["rev-list", "--objects", "--all"])?;
            for entry in entries
                .split(|b| *b == b'\n')
                .filter(|entry| !entry.is_empty())
            {
                let split = entry.iter().position(|b| *b == b' ');
                let oid = std::str::from_utf8(&entry[..split.unwrap_or(entry.len())])?;
                let (kind, data) = objects.read(oid)?;
                if kind == "blob" {
                    if let Some(split) = split {
                        report.inspect(
                            format!("history:{}", path_text(&entry[split + 1..])?),
                            &data,
                            false,
                        );
                    }
                } else if matches!(kind.as_str(), "commit" | "tag") {
                    report.inspect(format!("history:{kind}:{}", &oid[..12]), &data, false);
                }
            }
        }
        objects.finish()?;
    }
    for directory in artifacts {
        let tree = crate::support::canonical_path(&directory)?;
        if !tree.is_dir() || fs::symlink_metadata(&directory)?.file_type().is_symlink() {
            bail!("artifact root must be a real directory");
        }
        artifact_tree(&tree, &tree, &mut report)?;
    }
    for (name, rule) in &report.findings {
        let safe: String = if content_rules(name.as_bytes()).is_empty() {
            name.chars()
                .flat_map(|c| {
                    if c.is_control() {
                        c.escape_default().collect::<Vec<_>>()
                    } else {
                        vec![c]
                    }
                })
                .collect()
        } else {
            "path redacted".into()
        };
        println!("FAIL {rule}: {safe}");
    }
    println!(
        "privacy: {} candidates checked, {} finding(s); values redacted",
        report.count,
        report.findings.len()
    );
    if !report.findings.is_empty() {
        bail!("privacy gate rejected findings (values redacted)");
    }
    Ok(())
}
