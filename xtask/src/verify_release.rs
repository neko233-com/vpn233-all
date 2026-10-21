//! Fail-closed, offline release inspection. Archives are neither extracted nor run.
use anyhow::{anyhow, bail, Result};
use flate2::bufread::GzDecoder;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::LazyLock,
};

pub const PLATFORMS: &[&str] = &[
    "linux-x86_64",
    "windows-x86_64",
    "macos-x86_64",
    "macos-aarch64",
];
pub const BINARIES: &[&str] = &[
    "vpn233-provider-server",
    "vpn233-subscribe-server",
    "vpn233-client",
];
pub const COMMON: &[&str] = &[
    "version.txt",
    "README.md",
    "README.zh-CN.md",
    "LICENSE",
    "SECURITY.md",
    "CONTRIBUTING.md",
    "CHANGELOG.md",
    "CODE_OF_CONDUCT.md",
    "admin-web/dist/index.html",
    "docs/getting-started.md",
    "docs/deployment.md",
    "docs/mirrors.md",
    "docs/api.md",
    "docs/index.md",
    "docs/privacy.md",
    "docs/protocols.md",
    "docs/testing.md",
    "docs/troubleshooting.md",
    "docs/_config.yml",
    "examples/provider-server.yaml",
    "examples/subscribe-server.yaml",
    "examples/vpn233-client.yaml",
    "scripts/lib/mirrors.sh",
    "scripts/lib/mirrors.ps1",
    "scripts/lib/services.sh",
    "scripts/lib/layout.sh",
    "scripts/lib/layout.ps1",
    "install.sh",
    "install.ps1",
    "deploy.sh",
    "deploy.ps1",
    "update.sh",
    "update.ps1",
    "uninstall.sh",
    "uninstall.ps1",
    "start.sh",
    "start.ps1",
    "stop.sh",
    "stop.ps1",
];
const WEB_SUFFIXES: &[&str] = &[
    "html", "js", "css", "svg", "png", "jpg", "jpeg", "webp", "ico", "woff", "woff2", "ttf", "txt",
];
const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const MAX_COMPRESSED: u64 = 1024 * 1024 * 1024;
static VC_RUNTIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\A(?:vcruntime.*|msvcp.*|msvcr(?:[0-9].*|td)|concrt[0-9].*|vcomp[0-9].*|vccorlib[0-9].*|mfcm?[0-9].*|atl[0-9].*)\.dll\z").unwrap()
});
static DLL_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\A[A-Za-z0-9_.-]+\.dll\z").unwrap());
static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\]\(([^()\s]+)\)").unwrap());

fn u16le(data: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        data.get(offset..offset.checked_add(2).ok_or_else(|| anyhow!("invalid PE"))?)
            .ok_or_else(|| anyhow!("invalid PE"))?
            .try_into()?,
    ))
}
fn u32le(data: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        data.get(offset..offset.checked_add(4).ok_or_else(|| anyhow!("invalid PE"))?)
            .ok_or_else(|| anyhow!("invalid PE"))?
            .try_into()?,
    ))
}

/// Bounded AMD64 PE32+ ordinary and delay-import parsing. Never loads executable code.
pub fn pe_imports(data: &[u8]) -> Result<BTreeSet<String>> {
    if data.len() < 64 || &data[..2] != b"MZ" {
        bail!("invalid PE");
    }
    let pe = u32le(data, 0x3c)? as usize;
    if pe < 64
        || data.get(pe..pe.checked_add(4).ok_or_else(|| anyhow!("invalid PE"))?) != Some(b"PE\0\0")
    {
        bail!("invalid PE");
    }
    let count = usize::from(u16le(data, pe + 6)?);
    let optional_size = usize::from(u16le(data, pe + 20)?);
    let flags = u16le(data, pe + 22)?;
    let optional = pe + 24;
    let section_start = optional
        .checked_add(optional_size)
        .ok_or_else(|| anyhow!("invalid PE"))?;
    let section_end = section_start
        .checked_add(count * 40)
        .ok_or_else(|| anyhow!("invalid PE"))?;
    if u16le(data, pe + 4)? != 0x8664
        || !(1..=96).contains(&count)
        || flags & 2 == 0
        || flags & 0x2000 != 0
        || optional_size < 112
        || section_end > data.len()
        || u16le(data, optional)? != 0x20b
    {
        bail!("invalid PE");
    }
    let entry = u64::from(u32le(data, optional + 16)?);
    let headers = u64::from(u32le(data, optional + 60)?);
    let directory_count = u32le(data, optional + 108)? as usize;
    if !(2..=16).contains(&directory_count)
        || 112 + directory_count * 8 > optional_size
        || headers < section_end as u64
        || headers > data.len() as u64
    {
        bail!("invalid PE");
    }
    let mut sections = Vec::new();
    let mut virtual_ranges = vec![(0, headers)];
    let mut raw_ranges = vec![(0, headers)];
    let mut executable_entry = false;
    for number in 0..count {
        let start = section_start + number * 40;
        let virtual_size = u64::from(u32le(data, start + 8)?);
        let rva = u64::from(u32le(data, start + 12)?);
        let raw_size = u64::from(u32le(data, start + 16)?);
        let raw = u64::from(u32le(data, start + 20)?);
        let span = virtual_size.max(raw_size);
        if span == 0
            || rva + span > 0x1_0000_0000
            || raw + raw_size > data.len() as u64
            || virtual_ranges
                .iter()
                .any(|(begin, end)| rva < *end && rva + span > *begin)
            || raw_size != 0
                && raw_ranges
                    .iter()
                    .any(|(begin, end)| raw < *end && raw + raw_size > *begin)
        {
            bail!("invalid PE");
        }
        virtual_ranges.push((rva, rva + span));
        if raw_size != 0 {
            raw_ranges.push((raw, raw + raw_size));
        }
        sections.push((rva, raw, raw_size));
        executable_entry |=
            u32le(data, start + 36)? & 0x2000_0000 != 0 && rva <= entry && entry < rva + raw_size;
    }
    if !executable_entry {
        bail!("invalid PE");
    }
    let locate = |rva: u32, size: u32| -> Result<(usize, usize)> {
        let (rva, size) = (u64::from(rva), u64::from(size));
        if size == 0 || rva + size > 0x1_0000_0000 {
            bail!("invalid PE");
        }
        if rva + size <= headers {
            return Ok((rva as usize, headers as usize));
        }
        for &(base, raw, raw_size) in &sections {
            if base <= rva && rva + size <= base + raw_size {
                return Ok(((raw + rva - base) as usize, (raw + raw_size) as usize));
            }
        }
        bail!("invalid PE")
    };
    let dll_name = |rva| -> Result<String> {
        let (start, end) = locate(rva, 1)?;
        let limit = end.min(start + 261);
        let length = data[start..limit]
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| anyhow!("invalid PE"))?;
        let name = std::str::from_utf8(&data[start..start + length])?;
        if !name.is_ascii() || !DLL_NAME.is_match(name) {
            bail!("invalid PE");
        }
        Ok(name.to_ascii_lowercase())
    };
    let mut imports = BTreeSet::new();
    for (index, width) in [(1, 20), (13, 32)] {
        if index >= directory_count {
            continue;
        }
        let rva = u32le(data, optional + 112 + index * 8)?;
        let size = u32le(data, optional + 116 + index * 8)?;
        if rva == 0 && size == 0 {
            continue;
        }
        if rva == 0 || size < width || size > 1024 * 1024 {
            bail!("invalid PE");
        }
        let (start, _) = locate(rva, size)?;
        let mut terminated = false;
        for offset in (0..=size - width).step_by(width as usize) {
            let descriptor = &data[start + offset as usize..start + (offset + width) as usize];
            if descriptor.iter().all(|byte| *byte == 0) {
                terminated = true;
                break;
            }
            let name = if index == 13 {
                if u32le(descriptor, 0)? != 1 {
                    bail!("invalid PE");
                }
                u32le(descriptor, 4)?
            } else {
                u32le(descriptor, 12)?
            };
            imports.insert(dll_name(name)?);
        }
        if !terminated {
            bail!("invalid PE");
        }
    }
    Ok(imports)
}

pub fn windows_binary_rules(data: &[u8]) -> Vec<String> {
    match pe_imports(data) {
        Err(_) => vec!["windows-invalid-amd64-pe".into()],
        Ok(imports) if imports.iter().any(|name| VC_RUNTIME.is_match(name)) => {
            vec!["windows-extra-vc-runtime".into()]
        }
        _ => Vec::new(),
    }
}

pub fn architecture_rules(platform: &str, data: &[u8]) -> Vec<String> {
    let valid = match platform {
        "windows-x86_64" => pe_imports(data).is_ok(),
        "linux-x86_64" => {
            data.len() >= 64
                && data.get(..7) == Some(b"\x7fELF\x02\x01\x01")
                && u16le(data, 18).ok() == Some(62)
                && u16le(data, 52).ok() == Some(64)
        }
        "macos-x86_64" | "macos-aarch64" => {
            data.len() >= 32
                && u32le(data, 0).ok() == Some(0xfeedfacf)
                && u32le(data, 4).ok()
                    == Some(if platform == "macos-x86_64" {
                        0x01000007
                    } else {
                        0x0100000c
                    })
                && u32le(data, 12).ok() == Some(2)
        }
        _ => false,
    };
    if valid {
        Vec::new()
    } else {
        vec!["binary-architecture-mismatch".into()]
    }
}
struct ArchiveReport {
    problems: Vec<String>,
    payloads: BTreeMap<String, String>,
}
struct Inspection {
    name: String,
    root: String,
    platform: String,
    required: BTreeSet<String>,
    binaries: BTreeSet<String>,
    found: BTreeSet<String>,
    paths: BTreeSet<String>,
    payloads: BTreeMap<String, String>,
    markdown: BTreeMap<String, String>,
    problems: Vec<String>,
    version: String,
    total: u64,
    count: usize,
}
fn reserved_component(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|number| number.len() == 1 && matches!(number.as_bytes()[0], b'1'..=b'9'))
}
impl Inspection {
    fn problem(&mut self, rule: &str) {
        self.problems.push(format!("{}: {rule}", self.name));
    }
    fn member(&mut self, name: &str, data: Option<&[u8]>, regular: bool) -> Result<()> {
        self.count += 1;
        if self.count > MAX_ENTRIES {
            bail!("archive member bound exceeded");
        }
        let path = name.strip_suffix('/').unwrap_or(name);
        let parts: Vec<_> = path.split('/').collect();
        if name.len() > 4096
            || name.contains(['\\', ':', '\0'])
            || name.chars().any(char::is_control)
            || parts.first().copied() != Some(self.root.as_str())
            || parts.iter().any(|part| {
                part.is_empty()
                    || matches!(*part, "." | "..")
                    || part.ends_with(['.', ' '])
                    || reserved_component(part)
            })
        {
            self.problem("unsafe archive member path");
            return Ok(());
        }
        if !regular {
            self.problem("links/devices are forbidden");
            return Ok(());
        }
        // A case-insensitive collision would overwrite on Windows during deployment.
        if !self.paths.insert(path.to_lowercase()) {
            self.problem("duplicate archive member");
        }
        let relative = parts[1..].join("/");
        let Some(data) = data else {
            for rule in crate::privacy::inspect_bytes(&format!("{relative}/placeholder"), b"", true)
            {
                self.problem(&format!("{rule} in directory (path redacted)"));
            }
            if !relative.is_empty() {
                let ancestor = self
                    .required
                    .iter()
                    .any(|name| name.starts_with(&format!("{relative}/")));
                let assets = relative == "admin-web/dist/assets"
                    || relative.starts_with("admin-web/dist/assets/");
                if !ancestor && !assets {
                    self.problem("non-allowlisted directory (path redacted)");
                }
                if crate::privacy::sensitive_path(&relative)
                    || crate::privacy::sensitive_path(&format!("{relative}/placeholder"))
                {
                    self.problem("sensitive-file-path in directory (path redacted)");
                }
            }
            return Ok(());
        };
        self.total = self
            .total
            .checked_add(data.len() as u64)
            .ok_or_else(|| anyhow!("archive size overflow"))?;
        if data.len() as u64 > crate::privacy::MAX_INPUT_BYTES || self.total > MAX_TOTAL {
            bail!("archive size bound exceeded");
        }
        self.found.insert(relative.clone());
        self.payloads
            .insert(relative.clone(), format!("{:x}", Sha256::digest(data)));
        let public_asset = relative.starts_with("admin-web/dist/assets/")
            && relative.rsplit_once('.').is_some_and(|(_, extension)| {
                WEB_SUFFIXES.contains(&extension.to_ascii_lowercase().as_str())
            });
        let allowlisted = self.required.contains(&relative) || public_asset;
        if !allowlisted {
            self.problem("non-allowlisted member (path redacted)");
        }
        // Only fixed allowlisted paths are used in diagnostics; attacker-controlled
        // filenames may themselves contain credentials.
        let label = if self.required.contains(&relative) {
            relative.as_str()
        } else {
            "public asset (path redacted)"
        };
        for rule in crate::privacy::inspect_bytes(&relative, data, true) {
            self.problems
                .push(format!("{}: {rule} in {label} (value redacted)", self.name));
        }
        if self.binaries.contains(&relative) {
            let rules = if self.platform == "windows-x86_64" {
                windows_binary_rules(data)
            } else {
                architecture_rules(&self.platform, data)
            };
            for rule in rules {
                self.problems.push(format!(
                    "{}: {rule} in {label} (dependency redacted)",
                    self.name
                ));
            }
        }
        if relative == "version.txt"
            && std::str::from_utf8(data).ok().map(str::trim) != Some(self.version.as_str())
        {
            self.problem("version.txt does not match the release version");
        }
        if relative.ends_with(".md") {
            self.markdown
                .insert(relative, String::from_utf8_lossy(data).into_owned());
        }
        Ok(())
    }
    fn finish(mut self) -> ArchiveReport {
        for missing in self.required.difference(&self.found) {
            self.problems
                .push(format!("{}: missing {missing}", self.name));
        }
        for (source, text) in &self.markdown {
            for capture in LINK.captures_iter(text) {
                let target = &capture[1];
                if ["http://", "https://", "mailto:", "tel:", "#"]
                    .iter()
                    .any(|prefix| target.starts_with(prefix))
                {
                    continue;
                }
                let clean = target
                    .split('#')
                    .next()
                    .unwrap_or("")
                    .split('?')
                    .next()
                    .unwrap_or("");
                let decoded = percent_decode(clean);
                let resolved = decoded
                    .as_deref()
                    .and_then(|value| relative_link(source, value));
                if !clean.is_empty() && resolved.is_none_or(|value| !self.found.contains(&value)) {
                    self.problems.push(format!(
                        "{}: broken bundle link (source and target redacted)",
                        self.name
                    ));
                }
            }
        }
        ArchiveReport {
            problems: self.problems,
            payloads: self.payloads,
        }
    }
}
fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let pair = bytes.get(index + 1..index + 3)?;
            let hex = std::str::from_utf8(pair).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}
fn relative_link(source: &str, target: &str) -> Option<String> {
    if target.starts_with('/') || target.contains(['\\', '\0']) {
        return None;
    }
    let mut parts: Vec<_> = source.split('/').collect();
    parts.pop();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}
fn bounded_read(input: impl Read, size: u64) -> Result<Vec<u8>> {
    if size > crate::privacy::MAX_INPUT_BYTES {
        bail!("oversized archive member");
    }
    let mut data = Vec::new();
    input
        .take(crate::privacy::MAX_INPUT_BYTES + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 != size {
        bail!("archive member length mismatch");
    }
    Ok(data)
}

fn read_at(input: &mut fs::File, offset: u64, length: usize) -> Result<Vec<u8>> {
    input.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}
/// Inspect the physical ZIP envelope as well as the logical archive. The crate
/// intentionally tolerates SFX prefixes, unknown extra fields and some unused
/// bytes; none are produced by this project's deterministic writer.
fn zip_envelope(path: &Path) -> Result<()> {
    let mut input = open_regular(path)?;
    let length = input.metadata()?.len();
    let tail_start = length.saturating_sub(65535 + 22);
    let tail = read_at(&mut input, tail_start, (length - tail_start) as usize)?;
    let end = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&start| {
            tail.get(start..start + 4) == Some(b"PK\x05\x06")
                && u16le(&tail, start + 20)
                    .ok()
                    .is_some_and(|comment| start + 22 + usize::from(comment) == tail.len())
        })
        .ok_or_else(|| anyhow!("invalid ZIP ending"))?;
    let ending = &tail[end..end + 22];
    let end_position = tail_start + end as u64;
    let count = usize::from(u16le(ending, 10)?);
    let central_size = u64::from(u32le(ending, 12)?);
    let central_start = u64::from(u32le(ending, 16)?);
    if u16le(ending, 4)? != 0
        || u16le(ending, 6)? != 0
        || usize::from(u16le(ending, 8)?) != count
        || count > MAX_ENTRIES
        || central_start + central_size != end_position
    {
        bail!("invalid or multipart ZIP envelope");
    }
    let mut cursor = central_start;
    let mut ranges = Vec::new();
    for _ in 0..count {
        let central = read_at(&mut input, cursor, 46)?;
        if central.get(..4) != Some(b"PK\x01\x02") {
            bail!("invalid central ZIP header");
        }
        let name_length = usize::from(u16le(&central, 28)?);
        let extra_length = usize::from(u16le(&central, 30)?);
        let comment_length = usize::from(u16le(&central, 32)?);
        if name_length > 4096 || extra_length != 0 || u16le(&central, 34)? != 0 {
            bail!("ZIP extra fields or multipart entries forbidden");
        }
        let name = read_at(&mut input, cursor + 46, name_length)?;
        let offset = u64::from(u32le(&central, 42)?);
        let local = read_at(&mut input, offset, 30)?;
        if local.get(..4) != Some(b"PK\x03\x04")
            || usize::from(u16le(&local, 26)?) != name_length
            || u16le(&local, 28)? != 0
            || u16le(&local, 6)? & 9 != 0
        {
            bail!("invalid ZIP local metadata");
        }
        if read_at(&mut input, offset + 30, name_length)? != name {
            bail!("ZIP local and central names differ");
        }
        let data_end = offset + 30 + name_length as u64 + u64::from(u32le(&central, 20)?);
        if data_end > central_start {
            bail!("overlapping ZIP metadata");
        }
        ranges.push((offset, data_end));
        cursor = cursor
            .checked_add(46 + name_length as u64 + extra_length as u64 + comment_length as u64)
            .ok_or_else(|| anyhow!("ZIP metadata overflow"))?;
        if cursor > end_position {
            bail!("ZIP central header bounds exceeded");
        }
    }
    if cursor != end_position {
        bail!("unclaimed ZIP central metadata");
    }
    ranges.sort_unstable();
    let mut position = 0;
    for (start, end) in ranges {
        if start != position || end < start {
            bail!("unclaimed or overlapping ZIP bytes");
        }
        position = end;
    }
    if position != central_start {
        bail!("unclaimed ZIP prefix or payload gap");
    }
    Ok(())
}

/// Tar member padding is outside the logical payload. Require the producer's
/// zero padding so plaintext credentials cannot hide in skipped alignment bytes.
fn tar_padding(path: &Path) -> Result<()> {
    let mut decoder = GzDecoder::new(BufReader::new(open_regular(path)?));
    let mut header = [0; 512];
    let mut buffer = [0; 65536];
    let mut count = 0;
    let mut total = 0u64;
    loop {
        decoder.read_exact(&mut header)?;
        if header.iter().all(|byte| *byte == 0) {
            decoder.read_exact(&mut header)?;
            if header.iter().any(|byte| *byte != 0) {
                bail!("tar requires two end-of-archive zero blocks");
            }
            break;
        }
        count += 1;
        let parsed = tar::Header::from_byte_slice(&header);
        let size = parsed.size()?;
        let kind = parsed.entry_type();
        total = total
            .checked_add(size)
            .ok_or_else(|| anyhow!("tar length overflow"))?;
        if count > MAX_ENTRIES || size > crate::privacy::MAX_INPUT_BYTES || total > MAX_TOTAL {
            bail!("tar bounds exceeded");
        }
        let extension = kind.is_gnu_longname()
            || kind.is_gnu_longlink()
            || kind.is_pax_local_extensions()
            || kind.is_pax_global_extensions();
        if extension && size > 65536 {
            bail!("tar extension exceeds bound");
        }
        let mut metadata = Vec::new();
        let mut remaining = size;
        while remaining != 0 {
            let take = remaining.min(buffer.len() as u64) as usize;
            decoder.read_exact(&mut buffer[..take])?;
            if extension {
                metadata.extend_from_slice(&buffer[..take]);
            }
            remaining -= take as u64;
        }
        if extension {
            let normalized: Vec<_> = metadata
                .into_iter()
                .map(|byte| if byte == 0 { b' ' } else { byte })
                .collect();
            if !crate::privacy::content_rules(&normalized).is_empty() {
                bail!("private data in tar extension (value redacted)");
            }
        }
        let padding = ((512 - size % 512) % 512) as usize;
        decoder.read_exact(&mut buffer[..padding])?;
        if buffer[..padding].iter().any(|byte| *byte != 0) {
            bail!("nonzero tar member padding forbidden");
        }
    }
    let mut remaining = Vec::new();
    (&mut decoder)
        .take(1024 * 1024 + 1)
        .read_to_end(&mut remaining)?;
    if remaining.len() > 1024 * 1024 || remaining.iter().any(|byte| *byte != 0) {
        bail!("invalid tar trailer");
    }
    let mut trailing = [0; 1];
    if decoder.into_inner().read(&mut trailing)? != 0 {
        bail!("trailing gzip data forbidden");
    }
    Ok(())
}

fn archive_report(archive: &Path, version: &str) -> Result<ArchiveReport> {
    let metadata = fs::symlink_metadata(archive)?;
    if !metadata.is_file() || metadata.len() > MAX_COMPRESSED {
        bail!("invalid or oversized compressed archive");
    }
    let name = archive
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("invalid archive name"))?;
    let platform = name
        .strip_prefix("vpn233-rust-")
        .and_then(|value| {
            value
                .strip_suffix(".tar.gz")
                .or_else(|| value.strip_suffix(".zip"))
        })
        .ok_or_else(|| anyhow!("unsupported archive name"))?;
    if !PLATFORMS.contains(&platform) {
        bail!("unsupported archive platform");
    }
    let suffix = if platform.starts_with("windows-") {
        ".exe"
    } else {
        ""
    };
    let mut required: BTreeSet<String> = COMMON.iter().map(|value| (*value).into()).collect();
    let binary_paths: BTreeSet<_> = BINARIES
        .iter()
        .map(|binary| format!("bin/{binary}{suffix}"))
        .collect();
    required.extend(binary_paths.clone());
    let mut inspection = Inspection {
        name: name.into(),
        root: format!("vpn233-rust-{platform}"),
        platform: platform.into(),
        required,
        binaries: binary_paths,
        found: BTreeSet::new(),
        paths: BTreeSet::new(),
        payloads: BTreeMap::new(),
        markdown: BTreeMap::new(),
        problems: Vec::new(),
        version: version.into(),
        total: 0,
        count: 0,
    };
    if name.ends_with(".zip") {
        zip_envelope(archive)?;
        let mut opened = zip::ZipArchive::new(open_regular(archive)?)?;
        if opened.offset() != 0 {
            bail!("ZIP prefixes forbidden");
        }
        opened.validate_header_consistency()?;
        if !opened.comment().is_empty() {
            inspection.problem("ZIP comments are forbidden (value redacted)");
        }
        if opened.len() > MAX_ENTRIES {
            bail!("archive member bound exceeded");
        }
        for index in 0..opened.len() {
            let mut member = opened.by_index(index)?;
            if !member.comment().is_empty() {
                inspection.problem("ZIP member comments are forbidden (value redacted)");
            }
            if member
                .extra_data()
                .is_some_and(|extra| !crate::privacy::content_rules(&extra).is_empty())
            {
                inspection.problem("private data in ZIP metadata (value redacted)");
            }
            let name = std::str::from_utf8(member.name_raw())?.to_owned();
            let kind = member.unix_mode().unwrap_or(0) & 0o170000;
            let directory = member.is_dir();
            let regular =
                kind == 0 || kind == 0o100000 && !directory || kind == 0o040000 && directory;
            let data = if directory {
                if member.size() != 0 {
                    bail!("directory has payload");
                }
                None
            } else {
                let size = member.size();
                Some(bounded_read(&mut member, size)?)
            };
            inspection.member(&name, data.as_deref(), regular)?;
        }
    } else {
        let decoder = GzDecoder::new(BufReader::new(open_regular(archive)?));
        if decoder.header().is_some_and(|header| {
            header.filename().is_some_and(|value| !value.is_empty())
                || header.comment().is_some_and(|value| !value.is_empty())
        }) {
            inspection.problem("gzip names/comments are forbidden (value redacted)");
        }
        if decoder
            .header()
            .is_some_and(|header| header.extra().is_some_and(|value| !value.is_empty()))
        {
            inspection.problem("gzip extra fields are forbidden (value redacted)");
        }
        // raw entries expose GNU/PAX control records, so their sizes are checked
        // before the tar crate could allocate an attacker-specified extension.
        let mut opened = tar::Archive::new(decoder);
        let mut pending_path = None;
        let mut pending_size = None;
        let mut physical_count = 0;
        let mut physical_total = 0u64;
        for member in opened.entries()?.raw(true) {
            let mut member = member?;
            let normalized: Vec<_> = member
                .header()
                .as_bytes()
                .iter()
                .map(|byte| if *byte == 0 { b' ' } else { *byte })
                .collect();
            for rule in crate::privacy::content_rules(&normalized) {
                inspection.problem(&format!("{rule} in tar header (value redacted)"));
            }
            let kind = member.header().entry_type();
            if member
                .header()
                .username_bytes()
                .is_some_and(|value| !value.is_empty())
                || member
                    .header()
                    .groupname_bytes()
                    .is_some_and(|value| !value.is_empty())
            {
                inspection.problem("tar owner names are forbidden (value redacted)");
            }
            let size = member.size();
            physical_count += 1;
            physical_total = physical_total
                .checked_add(size)
                .ok_or_else(|| anyhow!("archive size overflow"))?;
            if physical_count > MAX_ENTRIES
                || size > crate::privacy::MAX_INPUT_BYTES
                || physical_total > MAX_TOTAL
            {
                bail!("archive bounds exceeded");
            }
            if kind.is_gnu_longname() || kind.is_pax_local_extensions() {
                if size > 65536 {
                    bail!("tar extension exceeds bound");
                }
                let bytes = bounded_read(&mut member, size)?;
                if !crate::privacy::content_rules(&bytes).is_empty() {
                    inspection.problem("private data in tar metadata (value redacted)");
                }
                if kind.is_gnu_longname() {
                    if pending_path.is_some() {
                        bail!("duplicate tar path extension");
                    }
                    let value = bytes.strip_suffix(&[0]).unwrap_or(&bytes);
                    pending_path = Some(std::str::from_utf8(value)?.to_owned());
                } else {
                    for (key, value) in pax_records(&bytes)? {
                        match key.as_str() {
                            "path" => {
                                if pending_path.replace(value).is_some() {
                                    bail!("duplicate tar path extension");
                                }
                            }
                            "size" => {
                                if pending_size.replace(value.parse::<u64>()?).is_some() {
                                    bail!("duplicate tar size extension");
                                }
                            }
                            "linkpath" => bail!("tar links forbidden"),
                            key if key.starts_with("GNU.sparse") => {
                                bail!("sparse tar members forbidden")
                            }
                            _ => {}
                        }
                    }
                }
                continue;
            }
            if !kind.is_file() && !kind.is_dir() {
                inspection.problem("links/devices are forbidden");
                continue;
            }
            if pending_size.take().is_some_and(|value| value != size) {
                bail!("tar extension size mismatch");
            }
            let bytes = member.path_bytes();
            let name = match pending_path.take() {
                Some(name) => name,
                None => std::str::from_utf8(&bytes)?.to_owned(),
            };
            let data = if kind.is_dir() {
                if size != 0 {
                    bail!("directory has payload");
                }
                None
            } else {
                Some(bounded_read(&mut member, size)?)
            };
            inspection.member(&name, data.as_deref(), true)?;
        }
        if pending_path.is_some() || pending_size.is_some() {
            bail!("orphan tar extension");
        }
        let mut tail = Vec::new();
        let mut decoder = opened.into_inner();
        (&mut decoder)
            .take(1024 * 1024 + 1)
            .read_to_end(&mut tail)?;
        if tail.len() > 1024 * 1024 || tail.iter().any(|byte| *byte != 0) {
            bail!("invalid tar trailer");
        }
        let mut trailing = [0; 1];
        if decoder.into_inner().read(&mut trailing)? != 0 {
            bail!("concatenated gzip members or trailing bytes are forbidden");
        }
        tar_padding(archive)?;
    }
    Ok(inspection.finish())
}
fn pax_records(data: &[u8]) -> Result<Vec<(String, String)>> {
    let mut index = 0;
    let mut records = Vec::new();
    while index < data.len() {
        let space = data[index..]
            .iter()
            .position(|byte| *byte == b' ')
            .ok_or_else(|| anyhow!("invalid PAX record"))?
            + index;
        let length = std::str::from_utf8(&data[index..space])?.parse::<usize>()?;
        let end = index
            .checked_add(length)
            .ok_or_else(|| anyhow!("invalid PAX record"))?;
        if end > data.len() || end <= space + 1 || data[end - 1] != b'\n' {
            bail!("invalid PAX record");
        }
        let record = std::str::from_utf8(&data[space + 1..end - 1])?;
        let (key, value) = record
            .split_once('=')
            .ok_or_else(|| anyhow!("invalid PAX record"))?;
        if key.is_empty() {
            bail!("invalid PAX key");
        }
        records.push((key.into(), value.into()));
        index = end;
    }
    Ok(records)
}
pub fn inspect_archive(archive: &Path, version: &str) -> Result<Vec<String>> {
    Ok(archive_report(archive, version)?.problems)
}
fn open_regular(path: &Path) -> Result<fs::File> {
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
    let input = options.open(path)?;
    let metadata = input.metadata()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("release input must be a regular file");
    }
    Ok(input)
}
fn small_text(path: &Path, limit: u64) -> Result<String> {
    let input = open_regular(path)?;
    if input.metadata()?.len() > limit {
        bail!("release metadata exceeds bound");
    }
    let mut data = Vec::new();
    input.take(limit + 1).read_to_end(&mut data)?;
    if data.len() as u64 > limit {
        bail!("release metadata exceeds bound");
    }
    Ok(String::from_utf8(data)?)
}
fn snapshot(path: &Path, name: &str) -> Result<(tempfile::TempDir, PathBuf, String)> {
    let mut input = open_regular(path)?;
    if input.metadata()?.len() > MAX_COMPRESSED {
        bail!("compressed archive exceeds bound");
    }
    let directory = tempfile::tempdir()?;
    let target = directory.path().join(name);
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)?;
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > MAX_COMPRESSED {
            bail!("compressed archive exceeds bound");
        }
        hash.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
    }
    output.flush()?;
    drop(output);
    Ok((directory, target, format!("{:x}", hash.finalize())))
}

pub fn verify(directory: &Path, version: &str, partial: bool) -> Result<Vec<String>> {
    if !Regex::new(r"\A[0-9]+\.[0-9]+\.[0-9]+\z")?.is_match(version) {
        return Ok(vec!["invalid semantic release version".into()]);
    }
    if fs::symlink_metadata(directory)?.file_type().is_symlink() {
        bail!("release directory must not be a symlink");
    }
    let all: BTreeSet<String> = PLATFORMS
        .iter()
        .flat_map(|platform| {
            [
                format!("vpn233-rust-{platform}.tar.gz"),
                format!("vpn233-rust-{platform}.zip"),
            ]
        })
        .collect();
    let mut problems = Vec::new();
    let mut present = BTreeSet::new();
    let mut regular = BTreeSet::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow!("non-UTF8 release asset"))?;
        if !fs::symlink_metadata(entry.path())?.is_file() {
            problems.push("release asset must be a regular file".into());
        } else {
            regular.insert(name.clone());
        }
        present.insert(name);
    }
    let archives: BTreeSet<_> = present.intersection(&all).cloned().collect();
    let expected = if partial {
        if archives.is_empty() {
            problems.push("no platform archives found".into());
        }
        archives.clone()
    } else {
        all
    };
    let mut allowed = expected.clone();
    allowed.extend(["SHA256SUMS".into(), "version.txt".into()]);
    if present != allowed {
        problems.push("release asset set differs from the exact allowlist".into());
    }
    let manifest = directory.join("SHA256SUMS");
    if !regular.contains("SHA256SUMS") {
        problems.push("SHA256SUMS missing".into());
        return Ok(problems);
    }
    if fs::symlink_metadata(&manifest)?.file_type().is_symlink() {
        bail!("checksum manifest must not be a symlink");
    }
    let manifest = small_text(&manifest, 65536)?;
    let pattern = Regex::new(r"\A([a-fA-F0-9]{64}) [ *]([^/\\\x00-\x1f\x7f]+)\z")?;
    let mut entries = BTreeMap::new();
    for (number, line) in manifest.lines().enumerate() {
        if number >= 100 {
            bail!("checksum manifest line bound exceeded");
        }
        if let Some(matched) = pattern.captures(line) {
            if entries
                .insert(matched[2].to_owned(), matched[1].to_ascii_lowercase())
                .is_some()
            {
                problems.push("duplicate SHA256SUMS entry".into());
            }
        } else {
            problems.push("invalid SHA256SUMS entry".into());
        }
    }
    if entries.keys().cloned().collect::<BTreeSet<_>>() != expected {
        problems.push("SHA256SUMS must cover each expected archive exactly once".into());
    }
    let stamp = directory.join("version.txt");
    if !regular.contains("version.txt") || small_text(&stamp, 1024)?.trim() != version {
        problems.push("release version.txt mismatch".into());
    }
    let mut reports = BTreeMap::new();
    for name in archives {
        let archive = directory.join(&name);
        if !regular.contains(&name) {
            problems.push(format!("{name}: archive must be a regular file"));
            continue;
        }
        // Hash and inspect the same bounded private snapshot. A later source
        // replacement cannot switch bytes between verification and parsing.
        let (_temporary, archive, digest) = snapshot(&archive, &name)?;
        if entries.get(&name) != Some(&digest) {
            problems.push(format!("{name}: SHA256 mismatch"));
            continue;
        }
        match archive_report(&archive, version) {
            Ok(found) => {
                problems.extend(found.problems);
                reports.insert(name, found.payloads);
            }
            Err(_) => problems.push(format!(
                "{name}: validation could not complete (archive contents redacted)"
            )),
        }
    }
    for platform in PLATFORMS {
        let prefix = format!("vpn233-rust-{platform}");
        if let (Some(tar), Some(zip)) = (
            reports.get(&format!("{prefix}.tar.gz")),
            reports.get(&format!("{prefix}.zip")),
        ) {
            if tar != zip {
                problems.push(format!("{prefix}: tar/ZIP payload names or SHA256 differ"));
            }
        }
    }
    Ok(problems)
}

pub fn run(args: &[String]) -> Result<()> {
    if !(2..=3).contains(&args.len()) || args.get(2).is_some_and(|value| value != "--partial") {
        bail!("usage: verify-release DIR VERSION [--partial]");
    }
    let version = args[1].strip_prefix('v').unwrap_or(&args[1]);
    let problems = verify(Path::new(&args[0]), version, args.len() == 3)?;
    for problem in &problems {
        println!("FAIL {problem}");
    }
    println!(
        "release assets: {} ({} finding(s))",
        if problems.is_empty() {
            "PASSED"
        } else {
            "FAILED"
        },
        problems.len()
    );
    if !problems.is_empty() {
        bail!("release gate rejected findings");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checksum_snapshot_cannot_change_with_source() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.zip");
        fs::write(&source, b"original public bytes").unwrap();
        let (_temporary, path, digest) = snapshot(&source, "vpn233-rust-linux-x86_64.zip").unwrap();
        fs::write(&source, b"changed public bytes").unwrap();
        assert_eq!(fs::read(path).unwrap(), b"original public bytes");
        assert_eq!(
            digest,
            format!("{:x}", Sha256::digest(b"original public bytes"))
        );
    }
}
