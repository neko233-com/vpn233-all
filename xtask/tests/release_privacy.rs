//! Synthetic fixtures only: no production configurations or network requests.
use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read, Write},
    path::Path,
    process::{Command, Output},
    time::Duration,
};
use vpn233_xtask::{archives, privacy, support, verify_release as release};

fn token() -> Vec<u8> {
    [b"gh".as_slice(), b"p_", &[b'Q'; 35]].concat()
}
fn address() -> String {
    [45, 33, 85, 32].map(|part| part.to_string()).join(".")
}
fn has(rules: Vec<String>, name: &str) -> bool {
    rules.iter().any(|rule| rule.contains(name))
}
fn put16(data: &mut [u8], offset: usize, value: u16) {
    data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(data: &mut [u8], offset: usize, value: u32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(data: &mut [u8], offset: usize, value: u64) {
    data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
fn synthetic_pe(dll: &str, delay: bool) -> Vec<u8> {
    let mut data = vec![0; 0x400];
    data[..2].copy_from_slice(b"MZ");
    put32(&mut data, 0x3c, 0x80);
    data[0x80..0x84].copy_from_slice(b"PE\0\0");
    put16(&mut data, 0x84, 0x8664);
    put16(&mut data, 0x86, 1);
    put16(&mut data, 0x94, 0xf0);
    put16(&mut data, 0x96, 0x22);
    let optional = 0x98;
    put16(&mut data, optional, 0x20b);
    put32(&mut data, optional + 4, 0x200);
    put32(&mut data, optional + 16, 0x1000);
    put32(&mut data, optional + 20, 0x1000);
    put64(&mut data, optional + 24, 0x140000000);
    put32(&mut data, optional + 32, 0x1000);
    put32(&mut data, optional + 36, 0x200);
    put16(&mut data, optional + 40, 6);
    put16(&mut data, optional + 48, 6);
    put32(&mut data, optional + 56, 0x2000);
    put32(&mut data, optional + 60, 0x200);
    put16(&mut data, optional + 68, 3);
    for (offset, value) in [(72, 0x100000), (80, 0x1000), (88, 0x100000), (96, 0x1000)] {
        put64(&mut data, optional + offset, value);
    }
    put32(&mut data, optional + 108, 16);
    let directory = optional + 112 + if delay { 13 } else { 1 } * 8;
    put32(&mut data, directory, 0x1020);
    put32(&mut data, directory + 4, if delay { 64 } else { 40 });
    let section = optional + 0xf0;
    data[section..section + 8].copy_from_slice(b".text\0\0\0");
    for (offset, value) in [
        (8, 0x200),
        (12, 0x1000),
        (16, 0x200),
        (20, 0x200),
        (36, 0x60000020),
    ] {
        put32(&mut data, section + offset, value);
    }
    data[0x200] = 0xc3;
    let descriptor: Vec<u32> = if delay {
        vec![1, 0x1080, 0x10b0, 0x10d0, 0x10c0, 0, 0, 0]
    } else {
        vec![0x10c0, 0, 0, 0x1080, 0x10d0]
    };
    for (index, value) in descriptor.into_iter().enumerate() {
        put32(&mut data, 0x220 + index * 4, value);
    }
    data[0x280..0x280 + dll.len()].copy_from_slice(dll.as_bytes());
    put64(&mut data, 0x2c0, 0x10e0);
    put64(&mut data, 0x2d0, 0x10e0);
    data[0x2e0..0x2ee].copy_from_slice(b"\0\0ExitProcess\0");
    data
}
fn native_header(platform: &str) -> Vec<u8> {
    let mut data = vec![0; 64];
    if platform == "linux-x86_64" {
        data[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        put16(&mut data, 16, 3);
        put16(&mut data, 18, 62);
        put16(&mut data, 52, 64);
    } else {
        put32(&mut data, 0, 0xfeedfacf);
        put32(
            &mut data,
            4,
            if platform == "macos-x86_64" {
                0x01000007
            } else {
                0x0100000c
            },
        );
        put32(&mut data, 12, 2);
    }
    data
}
fn git(repo: &Path, args: &[&str]) -> Output {
    let output = support::process_output(
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            ),
        None,
        Duration::from_secs(30),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "synthetic Git fixture command failed"
    );
    output
}
fn repo() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "--quiet"]);
    git(repo.path(), &["config", "user.name", "synthetic-test"]);
    git(
        repo.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    git(repo.path(), &["config", "commit.gpgsign", "false"]);
    git(repo.path(), &["config", "tag.gpgsign", "false"]);
    repo
}
fn privacy_cli(repo: &Path, flags: &[&str]) -> Output {
    support::process_output(
        Command::new(env!("CARGO_BIN_EXE_vpn233-xtask"))
            .arg("privacy")
            .arg("--repository")
            .arg(repo)
            .args(flags),
        None,
        Duration::from_secs(30),
    )
    .unwrap()
}
fn output_text(output: &Output) -> String {
    String::from_utf8_lossy(&[output.stdout.as_slice(), output.stderr.as_slice()].concat())
        .into_owned()
}

#[test]
fn commit_and_tag_messages_are_scanned_and_redacted() {
    for kind in ["commit", "tag"] {
        let repo = repo();
        fs::write(repo.path().join("public.txt"), b"public fixture").unwrap();
        git(repo.path(), &["add", "public.txt"]);
        let secret = String::from_utf8(token()).unwrap();
        git(
            repo.path(),
            &[
                "commit",
                "--quiet",
                "-m",
                if kind == "commit" {
                    &secret
                } else {
                    "public fixture"
                },
            ],
        );
        if kind == "tag" {
            git(repo.path(), &["tag", "-a", "v0.0.1", "-m", &secret]);
        }
        let output = privacy_cli(repo.path(), &["--history-only"]);
        assert!(!output.status.success());
        let text = output_text(&output);
        assert!(text.contains(&format!("history:{kind}:")));
        assert!(!text.contains(&secret));
    }
}
#[test]
fn runtime_paths_rejected() {
    for path in [
        "config.yaml",
        "nested/.env.prod",
        "data/users.json",
        "runtime/state.json",
        "backups/old.txt",
        "logs/output.log",
    ] {
        assert!(privacy::sensitive_path(path));
    }
}
#[test]
fn public_paths_allowed() {
    for path in [
        "provider-server/config.example.yaml",
        "examples/provider-server.yaml",
        "scripts/lib/mirrors.sh",
    ] {
        assert!(!privacy::sensitive_path(path));
    }
}
#[test]
fn credential_patterns() {
    for data in [
        token(),
        [b"-----BEGIN ".as_slice(), b"PRIVATE KEY-----"].concat(),
        [
            b"https://".as_slice(),
            b"user:random-password@",
            b"host.example",
        ]
        .concat(),
    ] {
        assert!(!privacy::content_rules(&data).is_empty());
    }
}
#[test]
fn reserved_addresses() {
    assert!(
        privacy::content_rules(b"127.0.0.1 192.0.2.8 198.51.100.9 203.0.113.5 1.1.1.1").is_empty()
    );
}
#[test]
fn public_server_address() {
    assert!(has(
        privacy::content_rules(address().as_bytes()),
        "non-example-public-ipv4"
    ));
}
#[test]
fn machine_path() {
    let path = [
        b"C:".as_slice(),
        b"/",
        b"Users",
        b"/",
        b"synthetic-person/project",
    ]
    .concat();
    assert!(has(privacy::content_rules(&path), "machine-home-path"));
}
#[test]
fn svg_geometric_numbers_are_not_server_addresses() {
    assert!(!has(
        privacy::content_rules(
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0-{}"/></svg>"#,
                address()
            )
            .as_bytes()
        ),
        "non-example-public-ipv4"
    ));
}
#[test]
fn svg_text_and_url_addresses_are_still_scanned() {
    for text in [
        format!("<svg><text>{}</text></svg>", address()),
        format!(
            r#"<svg><image href="https://{}/asset.png"/></svg>"#,
            address()
        ),
    ] {
        assert!(has(
            privacy::content_rules(text.as_bytes()),
            "non-example-public-ipv4"
        ));
    }
}
#[test]
fn svg_key_is_never_exempted() {
    let secret = [b"-----BEGIN ".as_slice(), b"PRIVATE KEY-----"].concat();
    assert!(has(
        privacy::content_rules(
            &[
                b"<svg><path d=\"M0 0\"/><text>".as_slice(),
                &secret,
                b"</text></svg>"
            ]
            .concat()
        ),
        "private-key"
    ));
}
#[test]
fn invalid_svg_path_is_not_exempted() {
    assert!(has(
        privacy::content_rules(
            format!(r#"<svg><path d="https://{}"/></svg>"#, address()).as_bytes()
        ),
        "non-example-public-ipv4"
    ));
}
#[test]
fn index_is_scanned_even_when_worktree_file_is_removed() {
    let repo = repo();
    let fixture = repo.path().join("config.yaml");
    fs::write(&fixture, token()).unwrap();
    git(repo.path(), &["add", "config.yaml"]);
    fs::remove_file(fixture).unwrap();
    let output = privacy_cli(repo.path(), &[]);
    assert!(!output.status.success());
    let text = output_text(&output);
    assert!(text.contains("index:config.yaml") && text.contains("sensitive-file-path"));
    assert!(!text.contains(&String::from_utf8(token()).unwrap()));
}
#[test]
fn history_only_scans_a_bare_mirror_and_redacts_values() {
    let scratch = tempfile::tempdir().unwrap();
    let source = repo();
    fs::write(source.path().join("old.txt"), token()).unwrap();
    git(source.path(), &["add", "old.txt"]);
    git(
        source.path(),
        &["commit", "--quiet", "-m", "synthetic fixture"],
    );
    let mirror = scratch.path().join("mirror.git");
    git(
        source.path(),
        &[
            "clone",
            "--quiet",
            "--mirror",
            source.path().to_str().unwrap(),
            mirror.to_str().unwrap(),
        ],
    );
    let output = privacy_cli(&mirror, &["--history-only"]);
    assert!(!output.status.success());
    let text = output_text(&output);
    assert!(text.contains("history:old.txt"));
    assert!(!text.contains(&String::from_utf8(token()).unwrap()));
}

struct Assets {
    scratch: tempfile::TempDir,
    version: String,
}
impl Assets {
    fn new() -> Self {
        Self {
            scratch: tempfile::tempdir().unwrap(),
            version: "2.0.0".into(),
        }
    }
    fn directory(&self) -> &Path {
        self.scratch.path()
    }
    fn make(&self, extra: &[(&str, Vec<u8>)], omitted: Option<&str>, windows: Option<Vec<u8>>) {
        for platform in release::PLATFORMS {
            let suffix = if platform.starts_with("windows-") {
                ".exe"
            } else {
                ""
            };
            let mut files: BTreeMap<String, Vec<u8>> = release::COMMON
                .iter()
                .map(|path| ((*path).into(), b"public placeholder\n".to_vec()))
                .collect();
            let payload = if suffix.is_empty() {
                native_header(platform)
            } else {
                windows
                    .clone()
                    .unwrap_or_else(|| synthetic_pe("KERNEL32.dll", false))
            };
            for binary in release::BINARIES {
                files.insert(format!("bin/{binary}{suffix}"), payload.clone());
            }
            files.insert("version.txt".into(), self.version.as_bytes().to_vec());
            for (path, data) in extra {
                files.insert((*path).into(), data.clone());
            }
            if let Some(path) = omitted {
                files.remove(path);
            }
            let build = tempfile::tempdir().unwrap();
            let bundle = build.path().join(format!("vpn233-rust-{platform}"));
            for (path, data) in files {
                let path = bundle.join(path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, data).unwrap();
            }
            archives::create(&bundle, self.directory(), 1600000000).unwrap();
        }
        self.rehash();
    }
    fn rehash(&self) {
        let mut files: Vec<_> = fs::read_dir(self.directory())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.to_str()
                    .is_some_and(|path| path.ends_with(".zip") || path.ends_with(".tar.gz"))
            })
            .collect();
        files.sort();
        let text = files
            .iter()
            .map(|path| {
                format!(
                    "{}  {}\n",
                    support::sha256_file(path).unwrap(),
                    path.file_name().unwrap().to_str().unwrap()
                )
            })
            .collect::<String>();
        fs::write(self.directory().join("SHA256SUMS"), text).unwrap();
        fs::write(self.directory().join("version.txt"), &self.version).unwrap();
    }
    fn verify(&self) -> Vec<String> {
        release::verify(self.directory(), &self.version, false).unwrap()
    }
}
#[test]
fn valid_matrix() {
    let assets = Assets::new();
    assets.make(&[], None, None);
    assert_eq!(Vec::<String>::new(), assets.verify());
}
#[test]
fn windows_system_runtime_imports_are_allowed() {
    for dll in [
        "KERNEL32.dll",
        "ucrtbase.dll",
        "api-ms-win-crt-runtime-l1-1-0.dll",
        "msvcrt.dll",
    ] {
        for delay in [false, true] {
            let data = synthetic_pe(dll, delay);
            assert_eq!(
                release::pe_imports(&data).unwrap(),
                [dll.to_lowercase()].into_iter().collect()
            );
            assert!(release::windows_binary_rules(&data).is_empty());
        }
    }
}
#[test]
fn windows_extra_vc_runtime_imports_are_rejected() {
    for dll in [
        "VCRUNTIME140.dll",
        "vcruntime140_1.dll",
        "MSVCP140.dll",
        "msvcr120.dll",
        "MSVCRTD.dll",
        "concrt140.dll",
        "vcomp140.dll",
        "vccorlib140.dll",
        "mfc140u.dll",
        "mfcm140u.dll",
        "atl100.dll",
    ] {
        for delay in [false, true] {
            assert_eq!(
                release::windows_binary_rules(&synthetic_pe(dll, delay)),
                ["windows-extra-vc-runtime"]
            );
        }
    }
}
#[test]
fn windows_runtime_gate_applies_to_each_binary_in_both_archives() {
    let assets = Assets::new();
    assets.make(&[], None, Some(synthetic_pe("VCRUNTIME140.dll", false)));
    let findings = assets.verify();
    assert_eq!(6, findings.len());
    assert!(findings.iter().all(
        |item| item.contains("windows-extra-vc-runtime") && !item.contains("VCRUNTIME140.dll")
    ));
}
#[test]
fn windows_import_gate_uses_descriptors_not_unreferenced_strings() {
    assert!(release::windows_binary_rules(
        &[
            synthetic_pe("KERNEL32.dll", false),
            b"VCRUNTIME140.dll\0".to_vec()
        ]
        .concat()
    )
    .is_empty());
}
#[test]
fn windows_non_pe_archive_members_are_rejected() {
    let assets = Assets::new();
    assets.make(&[], None, Some(b"synthetic binary\0".to_vec()));
    assert_eq!(
        6,
        assets
            .verify()
            .iter()
            .filter(|item| item.contains("windows-invalid-amd64-pe"))
            .count()
    );
}
#[test]
fn windows_truncated_pe_is_rejected() {
    let valid = synthetic_pe("KERNEL32.dll", false);
    for length in [0, 2, 63, 0x83, 0x98, 0x187, 0x1ff, 0x2ff, valid.len() - 1] {
        assert_eq!(
            release::windows_binary_rules(&valid[..length]),
            ["windows-invalid-amd64-pe"]
        );
    }
}
#[test]
fn windows_malformed_headers_and_imports_are_rejected() {
    for (offset, width, value) in [
        (0x84, 2, 0x14c),
        (0x98, 2, 0x10b),
        (0x86, 2, 0),
        (0x96, 2, 0x2022),
        (0x94, 2, 112),
        (0x98 + 108, 4, 17),
        (0x98 + 120, 4, 0x9000),
        (0x98 + 120, 4, 0),
        (0x98 + 124, 4, 0),
        (0x98 + 124, 4, 19),
        (0x98 + 124, 4, 20),
        (0x220 + 12, 4, 0x9000),
        (0x220 + 12, 4, 0),
        (0x220 + 12, 4, 0x1200),
        (0x188 + 20, 4, 0x100),
        (0x188 + 12, 4, 0x100),
        (0x188 + 16, 4, 0x300),
        (0x98 + 16, 4, 0),
    ] {
        let mut data = synthetic_pe("KERNEL32.dll", false);
        if width == 2 {
            put16(&mut data, offset, value as u16);
        } else {
            put32(&mut data, offset, value);
        }
        assert_eq!(
            release::windows_binary_rules(&data),
            ["windows-invalid-amd64-pe"],
            "offset {offset}"
        );
    }
    for dll in [
        "../VCRUNTIME140.dll",
        "C:\\VCRUNTIME140.dll",
        "VCRUNTIME140",
        "VCRUNTIME140.dll.",
        "VCRUNTIME140.dll ",
    ] {
        assert_eq!(
            release::windows_binary_rules(&synthetic_pe(dll, false)),
            ["windows-invalid-amd64-pe"]
        );
    }
}
#[test]
fn windows_malformed_delay_imports_are_rejected() {
    for attributes in [0, 3] {
        let mut data = synthetic_pe("KERNEL32.dll", true);
        put32(&mut data, 0x220, attributes);
        assert_eq!(
            release::windows_binary_rules(&data),
            ["windows-invalid-amd64-pe"]
        );
    }
}
#[test]
fn windows_dll_name_cannot_read_beyond_its_section() {
    let mut data = synthetic_pe("KERNEL32.dll", false);
    data[0x280..].fill(b'A');
    data.extend_from_slice(b".dll\0");
    assert_eq!(
        release::windows_binary_rules(&data),
        ["windows-invalid-amd64-pe"]
    );
}
#[test]
fn checksum_tampering() {
    let assets = Assets::new();
    assets.make(&[], None, None);
    fs::write(
        assets.directory().join("vpn233-rust-linux-x86_64.zip"),
        b"tampered",
    )
    .unwrap();
    assert!(has(assets.verify(), "SHA256 mismatch"));
}
#[test]
fn missing_checksum() {
    let assets = Assets::new();
    assets.make(&[], None, None);
    fs::remove_file(assets.directory().join("SHA256SUMS")).unwrap();
    assert!(!assets.verify().is_empty());
}
#[test]
fn missing_member() {
    let assets = Assets::new();
    assets.make(&[], Some("scripts/lib/mirrors.sh"), None);
    assert!(has(assets.verify(), "missing scripts/lib/mirrors.sh"));
}
#[test]
fn private_file() {
    let assets = Assets::new();
    assets.make(&[("config/config.yaml", b"private".to_vec())], None, None);
    assert!(has(assets.verify(), "sensitive-file-path"));
}
#[test]
fn unexpected_member() {
    let assets = Assets::new();
    assets.make(
        &[("admin-web/dist/assets/source.map", b"{}".to_vec())],
        None,
        None,
    );
    assert!(has(assets.verify(), "non-allowlisted"));
}
fn hostile_tar(
    directory: &Path,
    name: &str,
    data: &[u8],
    kind: tar::EntryType,
) -> std::path::PathBuf {
    let path = directory.join("vpn233-rust-linux-x86_64.tar.gz");
    let gz = flate2::GzBuilder::new().write(
        fs::File::create(&path).unwrap(),
        flate2::Compression::fast(),
    );
    let mut tar = tar::Builder::new(gz);
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_entry_type(kind);
    assert!(name.len() < 100);
    header.as_mut_bytes()[..100].fill(0);
    header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
    header.set_cksum();
    tar.append(&header, Cursor::new(data)).unwrap();
    tar.into_inner().unwrap().finish().unwrap();
    path
}
#[test]
fn path_traversal() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "../vpn233-rust-linux-x86_64/version.txt",
        "/vpn233-rust-linux-x86_64/version.txt",
        "vpn233-rust-linux-x86_64/../version.txt",
        "vpn233-rust-linux-x86_64\\version.txt",
        "vpn233-rust-linux-x86_64/a:stream",
    ] {
        let path = hostile_tar(dir.path(), name, b"2.0.0", tar::EntryType::Regular);
        assert!(has(
            release::inspect_archive(&path, "2.0.0").unwrap(),
            "unsafe archive member"
        ));
    }
}
#[test]
fn secret_inside_archive() {
    let assets = Assets::new();
    assets.make(&[("admin-web/dist/assets/app.js", token())], None, None);
    assert!(has(assets.verify(), "github-token"));
}
#[test]
fn extra_release_asset() {
    let assets = Assets::new();
    assets.make(&[], None, None);
    fs::write(assets.directory().join("debug.txt"), b"local").unwrap();
    assert!(has(assets.verify(), "exact allowlist"));
}
#[test]
fn wrong_version() {
    let assets = Assets::new();
    assets.make(&[], None, None);
    fs::write(assets.directory().join("version.txt"), b"0.0.0").unwrap();
    assert!(!assets.verify().is_empty());
}
#[test]
fn broken_relative_documentation_link() {
    let assets = Assets::new();
    assets.make(
        &[(
            "README.md",
            b"[missing documentation](docs/missing.md)".to_vec(),
        )],
        None,
        None,
    );
    assert!(has(assets.verify(), "broken bundle link"));
}
#[test]
fn archives_are_reproducible_after_mtime_change() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("bundle/vpn233-rust-linux-x86_64");
    fs::create_dir_all(&bundle).unwrap();
    let asset = bundle.join("version.txt");
    fs::write(&asset, b"2.0.0").unwrap();
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    archives::create(&bundle, &first, 1600000000).unwrap();
    let file = fs::OpenOptions::new().write(true).open(&asset).unwrap();
    file.set_times(
        fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1700000000)),
    )
    .unwrap();
    archives::create(&bundle, &second, 1600000000).unwrap();
    for extension in ["tar.gz", "zip"] {
        let name = format!("vpn233-rust-linux-x86_64.{extension}");
        assert_eq!(
            fs::read(first.join(&name)).unwrap(),
            fs::read(second.join(name)).unwrap()
        );
    }
}

// Additional regressions cover migration boundaries and previously missing gates.
#[test]
fn decoded_svg_entities_cannot_hide_credentials() {
    let encoded = String::from_utf8(token())
        .unwrap()
        .bytes()
        .map(|byte| format!("&#{byte};"))
        .collect::<String>();
    for data in [
        format!(r#"<svg><text>{encoded}</text></svg>"#),
        format!(r#"<svg><path d="{encoded}"/></svg>"#),
    ] {
        assert!(has(privacy::content_rules(data.as_bytes()), "github-token"));
    }
}
#[test]
fn decoded_svg_entities_and_foreign_paths_cannot_hide_addresses() {
    let encoded = address()
        .bytes()
        .map(|byte| format!("&#{byte};"))
        .collect::<String>();
    for text in [
        format!(r#"<svg><path d="{encoded}"/></svg>"#),
        format!(
            r#"<svg><p:path xmlns:p="urn:foreign" d="M0-{}"/></svg>"#,
            address()
        ),
        format!(r#"<svg><text>{encoded}</text></svg>"#),
    ] {
        assert!(has(
            privacy::content_rules(text.as_bytes()),
            "non-example-public-ipv4"
        ));
    }
}
#[test]
fn ip_boundaries_and_iana_exceptions_match_previous_gate() {
    for text in [
        format!("prefix{}suffix", address()),
        format!("{}.4", address()),
        format!("_{}", address()),
    ] {
        assert!(!has(
            privacy::content_rules(text.as_bytes()),
            "non-example-public-ipv4"
        ));
    }
    for (octets, global) in [
        ([192, 0, 0, 8], false),
        ([192, 0, 0, 9], true),
        ([192, 0, 0, 10], true),
        ([100, 64, 0, 1], false),
        ([224, 0, 0, 1], true),
        ([240, 0, 0, 1], false),
    ] {
        assert_eq!(privacy::global_ipv4(octets.into()), global);
    }
    for (segments, global) in [
        ([0x2001, 1, 0, 0, 0, 0, 0, 1], true),
        ([0x2001, 1, 0, 0, 0, 0, 0, 4], false),
        ([0x2001, 3, 0, 0, 0, 0, 0, 1], true),
        ([0x3fff, 0, 0, 0, 0, 0, 0, 1], false),
    ] {
        assert_eq!(privacy::global_ipv6(segments.into()), global);
    }
}
#[test]
fn binary_tokens_are_scanned_without_ip_noise() {
    let bytes = [b"\0".as_slice(), &token(), address().as_bytes()].concat();
    let rules = privacy::content_rules(&bytes);
    assert!(has(rules.clone(), "github-token"));
    assert!(!has(rules, "non-example-public-ipv4"));
}
#[test]
fn ignored_local_configuration_is_not_read() {
    let repo = repo();
    fs::write(repo.path().join(".gitignore"), b"config.yaml\n").unwrap();
    fs::write(repo.path().join("config.yaml"), token()).unwrap();
    assert!(privacy_cli(repo.path(), &[]).status.success());
}
#[test]
fn malformed_or_empty_artifact_requests_fail_closed() {
    assert!(privacy::run(&["--artifacts-only".into()]).is_err());
    assert!(privacy::run(&["--repository".into()]).is_err());
}
#[test]
fn partial_manifest_must_cover_exact_present_archives() {
    let assets = Assets::new();
    assets.make(&[], None, None);
    for entry in fs::read_dir(assets.directory()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        if name.starts_with("vpn233-rust-") && !name.starts_with("vpn233-rust-windows-") {
            fs::remove_file(path).unwrap();
        }
    }
    assets.rehash();
    assert!(release::verify(assets.directory(), &assets.version, true)
        .unwrap()
        .is_empty());
    assert!(!assets.verify().is_empty());
    let manifest = assets.directory().join("SHA256SUMS");
    let data = fs::read(&manifest).unwrap();
    fs::write(&manifest, [data.as_slice(), data.as_slice()].concat()).unwrap();
    assert!(has(
        release::verify(assets.directory(), &assets.version, true).unwrap(),
        "duplicate SHA256SUMS"
    ));
}
#[test]
fn inner_version_and_percent_decoded_links_are_checked() {
    let assets = Assets::new();
    assets.make(
        &[
            ("README.md", b"[ok](docs/%69ndex.md#part)".to_vec()),
            ("version.txt", b"9.9.9".to_vec()),
        ],
        None,
        None,
    );
    let findings = assets.verify();
    assert!(has(findings.clone(), "version.txt does not match"));
    assert!(!has(findings, "broken bundle link"));
}
#[test]
fn release_directory_and_archive_links_and_devices_are_rejected() {
    let assets = Assets::new();
    assets.make(&[], None, None);
    fs::create_dir(assets.directory().join("unexpected")).unwrap();
    assert!(has(assets.verify(), "regular file"));
    let dir = tempfile::tempdir().unwrap();
    for kind in [
        tar::EntryType::Symlink,
        tar::EntryType::Link,
        tar::EntryType::Char,
        tar::EntryType::Block,
        tar::EntryType::Fifo,
    ] {
        let path = hostile_tar(dir.path(), "vpn233-rust-linux-x86_64/link", b"", kind);
        assert!(!release::inspect_archive(&path, "2.0.0").unwrap().is_empty());
    }
}
#[test]
fn zip_duplicates_and_case_collisions_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vpn233-rust-linux-x86_64.zip");
    let mut writer = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    for name in [
        "vpn233-rust-linux-x86_64/version.txt",
        "vpn233-rust-linux-x86_64/VERSION.TXT",
    ] {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"2.0.0").unwrap();
    }
    writer.finish().unwrap();
    assert!(has(
        release::inspect_archive(&path, "2.0.0").unwrap(),
        "duplicate archive member"
    ));
}
#[test]
fn malformed_archive_and_manifest_fail_closed() {
    let assets = Assets::new();
    assets.make(&[], None, None);
    let path = assets.directory().join("vpn233-rust-linux-x86_64.zip");
    fs::write(&path, b"not a zip").unwrap();
    assets.rehash();
    assert!(has(assets.verify(), "validation could not complete"));
    fs::write(assets.directory().join("SHA256SUMS"), b"malformed\n").unwrap();
    assert!(has(assets.verify(), "invalid SHA256SUMS"));
}
#[test]
fn compression_modes_timestamps_and_payloads_are_consistent() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("vpn233-rust-linux-x86_64");
    fs::create_dir_all(bundle.join("bin")).unwrap();
    for name in ["bin/tool", "install.sh", "install.ps1"] {
        fs::write(bundle.join(name), name).unwrap();
    }
    let out = dir.path().join("out");
    archives::create(&bundle, &out, 1600000000).unwrap();
    let gz = fs::read(out.join("vpn233-rust-linux-x86_64.tar.gz")).unwrap();
    assert_eq!(&gz[4..8], &1600000000u32.to_le_bytes());
    assert_eq!(gz[3] & 8, 0);
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(Cursor::new(gz)));
    let mut payloads = BTreeMap::new();
    for entry in tar.entries().unwrap() {
        let mut entry = entry.unwrap();
        let name = entry.path().unwrap().to_str().unwrap().to_owned();
        assert_eq!(entry.header().uid().unwrap(), 0);
        assert_eq!(entry.header().gid().unwrap(), 0);
        assert_eq!(entry.header().mtime().unwrap(), 1600000000);
        if entry.header().entry_type().is_file() {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            payloads.insert(name, (entry.header().mode().unwrap(), bytes));
        }
    }
    let mut zip =
        zip::ZipArchive::new(fs::File::open(out.join("vpn233-rust-linux-x86_64.zip")).unwrap())
            .unwrap();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).unwrap();
        assert_eq!(entry.system(), zip::System::Unix);
        if entry.is_dir() {
            assert_eq!(entry.unix_mode().unwrap() & 0o777, 0o755);
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        let name = entry.name().unwrap();
        let (mode, wanted) = payloads.get(name.as_ref()).unwrap();
        assert_eq!(&bytes, wanted);
        assert_eq!(entry.unix_mode().unwrap() & 0o777, *mode);
        assert_eq!(*mode, if name.ends_with(".ps1") { 0o644 } else { 0o755 });
    }
}
#[test]
fn output_inside_input_is_rejected_without_creating_output() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("bundle");
    fs::create_dir(&bundle).unwrap();
    fs::write(bundle.join("public.txt"), b"public").unwrap();
    assert!(archives::create(&bundle, &bundle.join("out"), 1600000000).is_err());
    assert!(!bundle.join("out").exists());
}
#[test]
fn pe_directory_overflow_zero_fill_and_unterminated_delay_fail_closed() {
    for delay in [false, true] {
        let directory = 0x98 + 112 + if delay { 13 } else { 1 } * 8;
        for (offset, value) in [
            (directory, 0xfffffff0),
            (directory + 4, 0x100001),
            (directory + 4, if delay { 32 } else { 20 }),
            (0x188 + 8, 0x300),
            (0x188 + 12, 0xfffffff0),
        ] {
            let mut data = synthetic_pe("KERNEL32.dll", delay);
            put32(&mut data, offset, value);
            if offset == 0x188 + 8 {
                put32(&mut data, if delay { 0x224 } else { 0x22c }, 0x1200);
            }
            assert!(!release::windows_binary_rules(&data).is_empty());
        }
    }
}
#[test]
fn all_platform_architecture_headers_are_enforced() {
    for platform in release::PLATFORMS {
        let data = if *platform == "windows-x86_64" {
            synthetic_pe("KERNEL32.dll", false)
        } else {
            native_header(platform)
        };
        assert!(release::architecture_rules(platform, &data).is_empty());
        for other in release::PLATFORMS {
            if other != platform {
                assert!(!release::architecture_rules(other, &data).is_empty());
            }
        }
        for length in [0, 2, 15, 31] {
            assert!(!release::architecture_rules(platform, &data[..length]).is_empty());
        }
    }
    let mut wrong = native_header("linux-x86_64");
    put16(&mut wrong, 18, 183);
    assert!(!release::architecture_rules("linux-x86_64", &wrong).is_empty());
}
#[test]
fn tar_zip_payload_must_match_even_with_valid_manifest() {
    let assets = Assets::new();
    assets.make(&[], None, None);
    let path = assets.directory().join("vpn233-rust-linux-x86_64.zip");
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let mut writer = zip::ZipWriter::new_append(file).unwrap();
    writer
        .start_file(
            "vpn233-rust-linux-x86_64/admin-web/dist/assets/extra.js",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    writer.write_all(b"public fixture").unwrap();
    writer.finish().unwrap();
    assets.rehash();
    assert!(has(
        assets.verify(),
        "tar/ZIP payload names or SHA256 differ"
    ));
}
#[test]
fn malformed_svg_and_dtd_are_rejected_by_named_gate() {
    for bytes in [
        b"<svg><broken>".as_slice(),
        b"<!DOCTYPE svg [<!ENTITY secret 'fixture'>]><svg>&secret;</svg>",
    ] {
        assert!(has(
            privacy::inspect_bytes("public.svg", bytes, true),
            "invalid-svg-document"
        ));
    }
}
#[test]
fn missing_output_parent_traversal_cannot_land_inside_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("bundle");
    fs::create_dir(&bundle).unwrap();
    fs::write(bundle.join("public.txt"), b"public").unwrap();
    let out = dir.path().join("missing/../bundle");
    assert!(archives::create(&bundle, &out, 1600000000).is_err());
    assert!(!dir.path().join("missing").exists());
    assert!(!bundle.join("bundle.zip").exists());
}
#[test]
fn dos_reserved_names_and_ads_are_rejected_in_both_formats() {
    let dir = tempfile::tempdir().unwrap();
    for part in [
        "CON.js",
        "aux.css",
        "NUL.txt",
        "COM1.js",
        "Lpt9.svg",
        "app.js:stream",
        "app.js.",
    ] {
        let name = format!("vpn233-rust-linux-x86_64/admin-web/dist/assets/{part}");
        let tar = hostile_tar(dir.path(), &name, b"public", tar::EntryType::Regular);
        assert!(has(
            release::inspect_archive(&tar, "2.0.0").unwrap(),
            "unsafe archive member"
        ));
        let path = dir.path().join("vpn233-rust-linux-x86_64.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"public").unwrap();
        zip.finish().unwrap();
        assert!(has(
            release::inspect_archive(&path, "2.0.0").unwrap(),
            "unsafe archive member"
        ));
    }
}
#[test]
fn extra_empty_directories_are_rejected_in_both_formats() {
    let dir = tempfile::tempdir().unwrap();
    for part in ["config", "unexpected"] {
        let name = format!("vpn233-rust-linux-x86_64/{part}");
        let tar = hostile_tar(dir.path(), &name, b"", tar::EntryType::Directory);
        assert!(has(
            release::inspect_archive(&tar, "2.0.0").unwrap(),
            "non-allowlisted directory"
        ));
        let path = dir.path().join("vpn233-rust-linux-x86_64.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        zip.add_directory(format!("{name}/"), zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.finish().unwrap();
        assert!(has(
            release::inspect_archive(&path, "2.0.0").unwrap(),
            "non-allowlisted directory"
        ));
    }
}
#[test]
fn manifest_and_compressed_input_limits_fail_closed() {
    let assets = Assets::new();
    assets.make(&[], None, None);
    let manifest = assets.directory().join("SHA256SUMS");
    let line = fs::read_to_string(&manifest)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned();
    fs::write(&manifest, format!("{line}\n").repeat(101)).unwrap();
    assert!(release::verify(assets.directory(), &assets.version, false).is_err());
    fs::write(&manifest, vec![b' '; 65537]).unwrap();
    assert!(release::verify(assets.directory(), &assets.version, false).is_err());
    let path = assets.directory().join("vpn233-rust-linux-x86_64.zip");
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(1024 * 1024 * 1024 + 1)
        .unwrap();
    assert!(release::inspect_archive(&path, &assets.version).is_err());
}
#[test]
fn oversized_zip_and_tar_members_are_rejected_before_reading_body() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vpn233-rust-linux-x86_64.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    zip.start_file(
        "vpn233-rust-linux-x86_64/version.txt",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"2.0.0").unwrap();
    zip.finish().unwrap();
    let mut data = fs::read(&path).unwrap();
    let central = data
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    put32(&mut data, central + 24, 256 * 1024 * 1024 + 1);
    fs::write(&path, data).unwrap();
    assert!(release::inspect_archive(&path, "2.0.0").is_err());
    let tarpath = dir.path().join("vpn233-rust-linux-x86_64.tar.gz");
    let gz = flate2::GzBuilder::new().write(
        fs::File::create(&tarpath).unwrap(),
        flate2::Compression::fast(),
    );
    let mut out = gz;
    let mut header = tar::Header::new_gnu();
    header
        .set_path("vpn233-rust-linux-x86_64/version.txt")
        .unwrap();
    header.set_size(256 * 1024 * 1024 + 1);
    header.set_mode(0o644);
    header.set_cksum();
    out.write_all(header.as_bytes()).unwrap();
    out.finish().unwrap();
    assert!(release::inspect_archive(&tarpath, "2.0.0").is_err());
}
#[test]
fn archive_comments_owner_names_and_filenames_cannot_hide_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vpn233-rust-linux-x86_64.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    let secret = String::from_utf8(token()).unwrap();
    zip.set_comment(secret.clone()).unwrap();
    zip.start_file(
        format!("vpn233-rust-linux-x86_64/admin-web/dist/assets/{secret}.js"),
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"public").unwrap();
    zip.finish().unwrap();
    let findings = release::inspect_archive(&path, "2.0.0").unwrap();
    assert!(has(findings.clone(), "ZIP comments are forbidden"));
    assert!(has(findings.clone(), "github-token"));
    assert!(findings.iter().all(|finding| !finding.contains(&secret)));
    let path = dir.path().join("vpn233-rust-linux-x86_64.tar.gz");
    let gz = flate2::GzBuilder::new().filename(secret.as_str()).write(
        fs::File::create(&path).unwrap(),
        flate2::Compression::fast(),
    );
    let mut out = tar::Builder::new(gz);
    let mut header = tar::Header::new_gnu();
    let owner = String::from_utf8([b"AK".as_slice(), b"IA", &[b'Q'; 16]].concat()).unwrap();
    header.set_username(&owner).unwrap();
    header.set_groupname("synthetic-owner").unwrap();
    header.set_size(5);
    header.set_mode(0o644);
    header.set_cksum();
    out.append_data(
        &mut header,
        "vpn233-rust-linux-x86_64/version.txt",
        Cursor::new(b"2.0.0"),
    )
    .unwrap();
    out.into_inner().unwrap().finish().unwrap();
    let findings = release::inspect_archive(&path, "2.0.0").unwrap();
    assert!(has(findings.clone(), "gzip names/comments"));
    assert!(has(findings.clone(), "tar owner names"));
    assert!(findings.iter().all(|finding| !finding.contains(&secret)));
}
#[test]
fn gnu_and_pax_long_paths_are_bounded_and_validated() {
    let dir = tempfile::tempdir().unwrap();
    let long = format!("admin-web/dist/assets/{}/asset.js", "x".repeat(120));
    let bundle = dir.path().join("vpn233-rust-linux-x86_64");
    let file = bundle.join(&long);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, b"public").unwrap();
    let out = dir.path().join("out");
    archives::create(&bundle, &out, 1600000000).unwrap();
    let findings =
        release::inspect_archive(&out.join("vpn233-rust-linux-x86_64.tar.gz"), "2.0.0").unwrap();
    assert!(!has(findings.clone(), "unsafe archive"));
    assert!(!has(findings, "non-allowlisted"));
    let path = dir.path().join("vpn233-rust-linux-x86_64.tar.gz");
    let gz = flate2::GzBuilder::new().write(
        fs::File::create(&path).unwrap(),
        flate2::Compression::fast(),
    );
    let mut writer = tar::Builder::new(gz);
    writer
        .append_pax_extensions([("path", b"../private".as_slice())])
        .unwrap();
    let mut header = tar::Header::new_gnu();
    header.set_size(0);
    header.set_mode(0o644);
    header.set_cksum();
    writer
        .append_data(
            &mut header,
            "vpn233-rust-linux-x86_64/version.txt",
            std::io::empty(),
        )
        .unwrap();
    writer.into_inner().unwrap().finish().unwrap();
    assert!(has(
        release::inspect_archive(&path, "2.0.0").unwrap(),
        "unsafe archive member"
    ));
}
#[test]
fn empty_asset_directory_names_are_scanned_and_redacted() {
    let dir = tempfile::tempdir().unwrap();
    let secret = String::from_utf8(token()).unwrap();
    for name in [secret.clone(), address()] {
        let name = format!("vpn233-rust-linux-x86_64/admin-web/dist/assets/{name}");
        let tar = hostile_tar(dir.path(), &name, b"", tar::EntryType::Directory);
        let findings = release::inspect_archive(&tar, "2.0.0").unwrap();
        assert!(has(
            findings.clone(),
            if name.contains(&secret) {
                "github-token"
            } else {
                "non-example-public-ipv4"
            }
        ));
        assert!(findings
            .iter()
            .all(|finding| !finding.contains(&secret) && !finding.contains(&address())));
        let path = dir.path().join("vpn233-rust-linux-x86_64.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        zip.add_directory(format!("{name}/"), zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.finish().unwrap();
        let findings = release::inspect_archive(&path, "2.0.0").unwrap();
        assert!(has(
            findings.clone(),
            if name.contains(&secret) {
                "github-token"
            } else {
                "non-example-public-ipv4"
            }
        ));
        assert!(findings
            .iter()
            .all(|finding| !finding.contains(&secret) && !finding.contains(&address())));
    }
}
#[test]
fn gzip_extra_comment_crc_concatenation_and_trailing_bytes_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = hostile_tar(
        dir.path(),
        "vpn233-rust-linux-x86_64/version.txt",
        b"2.0.0",
        tar::EntryType::Regular,
    );
    let valid = fs::read(&path).unwrap();
    let mut tarbytes = Vec::new();
    flate2::read::GzDecoder::new(Cursor::new(&valid))
        .read_to_end(&mut tarbytes)
        .unwrap();
    for field in ["extra", "comment"] {
        let builder = if field == "extra" {
            flate2::GzBuilder::new().extra(token())
        } else {
            flate2::GzBuilder::new().comment(token())
        };
        let mut encoder = builder.write(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&tarbytes).unwrap();
        fs::write(&path, encoder.finish().unwrap()).unwrap();
        let findings = release::inspect_archive(&path, "2.0.0").unwrap();
        assert!(has(
            findings.clone(),
            if field == "extra" {
                "gzip extra"
            } else {
                "gzip names/comments"
            }
        ));
        assert!(findings
            .iter()
            .all(|finding| !finding.contains(&String::from_utf8(token()).unwrap())));
    }
    let mut extra = flate2::GzBuilder::new()
        .filename(token())
        .write(Vec::new(), flate2::Compression::fast());
    extra.write_all(&[0; 8]).unwrap();
    for suffix in [extra.finish().unwrap(), b"trailing".to_vec()] {
        fs::write(&path, [valid.as_slice(), suffix.as_slice()].concat()).unwrap();
        assert!(release::inspect_archive(&path, "2.0.0").is_err());
    }
    let mut corrupt = valid.clone();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    fs::write(&path, corrupt).unwrap();
    assert!(release::inspect_archive(&path, "2.0.0").is_err());
    fs::write(&path, &valid[..valid.len() - 4]).unwrap();
    assert!(release::inspect_archive(&path, "2.0.0").is_err());
}
#[test]
fn zip_member_comments_are_rejected_and_redacted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vpn233-rust-linux-x86_64.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    zip.start_file(
        "vpn233-rust-linux-x86_64/version.txt",
        zip::write::FullFileOptions::default()
            .with_file_comment(String::from_utf8(token()).unwrap()),
    )
    .unwrap();
    zip.write_all(b"2.0.0").unwrap();
    zip.finish().unwrap();
    let findings = release::inspect_archive(&path, "2.0.0").unwrap();
    assert!(has(findings.clone(), "ZIP member comments"));
    assert!(findings
        .iter()
        .all(|finding| !finding.contains(&String::from_utf8(token()).unwrap())));
}

fn zip_record(data: &[u8], name: &str) -> (usize, usize) {
    let mut cursor =
        u32::from_le_bytes(data[data.len() - 6..data.len() - 2].try_into().unwrap()) as usize;
    loop {
        assert_eq!(&data[cursor..cursor + 4], b"PK\x01\x02");
        let name_len =
            u16::from_le_bytes(data[cursor + 28..cursor + 30].try_into().unwrap()) as usize;
        let extra = u16::from_le_bytes(data[cursor + 30..cursor + 32].try_into().unwrap()) as usize;
        let comment =
            u16::from_le_bytes(data[cursor + 32..cursor + 34].try_into().unwrap()) as usize;
        if &data[cursor + 46..cursor + 46 + name_len] == name.as_bytes() {
            return (
                cursor,
                u32::from_le_bytes(data[cursor + 42..cursor + 46].try_into().unwrap()) as usize,
            );
        }
        cursor += 46 + name_len + extra + comment;
    }
}
fn mutate_tar(path: &Path, mutate: impl FnOnce(&mut Vec<u8>)) {
    let mut data = Vec::new();
    flate2::read::GzDecoder::new(fs::File::open(path).unwrap())
        .read_to_end(&mut data)
        .unwrap();
    mutate(&mut data);
    let mut gzip = flate2::GzBuilder::new()
        .write(fs::File::create(path).unwrap(), flate2::Compression::fast());
    gzip.write_all(&data).unwrap();
    gzip.finish().unwrap();
}
fn redacted_rejection(assets: &Assets) {
    assets.rehash();
    let findings = assets.verify();
    assert!(!findings.is_empty());
    let secret = String::from_utf8(token()).unwrap();
    assert!(findings
        .iter()
        .all(|finding| !finding.contains(&secret) && !finding.contains(&address())));
}

#[test]
fn valid_matrix_rejects_disagreeing_zip_local_filename() {
    let assets = Assets::new();
    assets.make(&[], None, None);
    assert!(assets.verify().is_empty());
    let path = assets.directory().join("vpn233-rust-linux-x86_64.zip");
    let mut bytes = fs::read(&path).unwrap();
    let name = "vpn233-rust-linux-x86_64/docs/getting-started.md";
    let (_, local) = zip_record(&bytes, name);
    let mut hidden = token();
    hidden.resize(name.len(), b'X');
    bytes[local + 30..local + 30 + name.len()].copy_from_slice(&hidden);
    fs::write(&path, bytes).unwrap();
    redacted_rejection(&assets);
}

#[test]
fn valid_matrix_rejects_raw_local_and_central_unknown_zip_extras() {
    for local_extra in [false, true] {
        let assets = Assets::new();
        assets.make(&[], None, None);
        assert!(assets.verify().is_empty());
        let path = assets.directory().join("vpn233-rust-linux-x86_64.zip");
        let mut bytes = fs::read(&path).unwrap();
        let name = "vpn233-rust-linux-x86_64/docs/getting-started.md";
        let (central, local) = zip_record(&bytes, name);
        let old_end = bytes.len() - 22;
        let old_central =
            u32::from_le_bytes(bytes[old_end + 16..old_end + 20].try_into().unwrap()) as usize;
        let old_size = u32::from_le_bytes(bytes[old_end + 12..old_end + 16].try_into().unwrap());
        let mut extra = vec![0xfe, 0xca];
        extra.extend_from_slice(&(token().len() as u16).to_le_bytes());
        extra.extend_from_slice(&token());
        let inserted = extra.len();
        if local_extra {
            put16(&mut bytes, local + 28, inserted as u16);
            bytes.splice(local + 30 + name.len()..local + 30 + name.len(), extra);
            let new_end = old_end + inserted;
            put32(&mut bytes, new_end + 16, (old_central + inserted) as u32);
            let mut cursor = old_central + inserted;
            while cursor < new_end {
                let offset = u32::from_le_bytes(bytes[cursor + 42..cursor + 46].try_into().unwrap())
                    as usize;
                if offset > local {
                    put32(&mut bytes, cursor + 42, (offset + inserted) as u32);
                }
                let n = u16::from_le_bytes(bytes[cursor + 28..cursor + 30].try_into().unwrap())
                    as usize;
                let e = u16::from_le_bytes(bytes[cursor + 30..cursor + 32].try_into().unwrap())
                    as usize;
                let c = u16::from_le_bytes(bytes[cursor + 32..cursor + 34].try_into().unwrap())
                    as usize;
                cursor += 46 + n + e + c;
            }
        } else {
            put16(&mut bytes, central + 30, inserted as u16);
            bytes.splice(central + 46 + name.len()..central + 46 + name.len(), extra);
            put32(
                &mut bytes,
                old_end + inserted + 12,
                old_size + inserted as u32,
            );
        }
        fs::write(&path, bytes).unwrap();
        redacted_rejection(&assets);
    }
}

#[test]
fn valid_matrix_rejects_zip_prefix_tail_duplicate_and_unclaimed_regions() {
    for variant in ["prefix", "tail", "duplicate", "gap"] {
        let assets = Assets::new();
        assets.make(&[], None, None);
        assert!(assets.verify().is_empty());
        let path = assets.directory().join("vpn233-rust-linux-x86_64.zip");
        let mut bytes = fs::read(&path).unwrap();
        let old_end = bytes.len() - 22;
        let central =
            u32::from_le_bytes(bytes[old_end + 16..old_end + 20].try_into().unwrap()) as usize;
        match variant {
            "prefix" => bytes.splice(..0, token()).for_each(drop),
            "tail" => bytes.extend_from_slice(&token()),
            "duplicate" => {
                let n = u16::from_le_bytes(bytes[central + 28..central + 30].try_into().unwrap())
                    as usize;
                let record = bytes[central..central + 46 + n].to_vec();
                let size = record.len();
                bytes.splice(old_end..old_end, record);
                let count = u16::from_le_bytes(
                    bytes[old_end + size + 10..old_end + size + 12]
                        .try_into()
                        .unwrap(),
                ) + 1;
                let central_size = u32::from_le_bytes(
                    bytes[old_end + size + 12..old_end + size + 16]
                        .try_into()
                        .unwrap(),
                );
                put16(&mut bytes, old_end + size + 8, count);
                put16(&mut bytes, old_end + size + 10, count);
                put32(&mut bytes, old_end + size + 12, central_size + size as u32);
            }
            _ => {
                let added = token().len();
                bytes.splice(central..central, token());
                put32(&mut bytes, old_end + added + 16, (central + added) as u32);
            }
        }
        fs::write(&path, bytes).unwrap();
        redacted_rejection(&assets);
    }
}

#[test]
fn valid_matrix_rejects_tar_unused_header_fields_and_nul_padding() {
    for variant in ["name-padding", "linkname", "username-padding"] {
        let assets = Assets::new();
        assets.make(&[], None, None);
        assert!(assets.verify().is_empty());
        let path = assets.directory().join("vpn233-rust-linux-x86_64.tar.gz");
        mutate_tar(&path, |data| {
            let mut header = tar::Header::new_gnu();
            header.as_mut_bytes().copy_from_slice(&data[..512]);
            let offset = match variant {
                "name-padding" => header.path_bytes().len() + 1,
                "linkname" => 157,
                _ => 266,
            };
            let hidden = if variant == "username-padding" {
                [b"AK".as_slice(), b"IA", &[b'Q'; 16]].concat()
            } else {
                token()
            };
            header.as_mut_bytes()[offset..offset + hidden.len()].copy_from_slice(&hidden);
            header.set_cksum();
            data[..512].copy_from_slice(header.as_bytes());
        });
        redacted_rejection(&assets);
    }
}

#[test]
fn valid_matrix_rejects_nonzero_tar_payload_alignment_padding() {
    let assets = Assets::new();
    assets.make(&[], None, None);
    assert!(assets.verify().is_empty());
    let path = assets.directory().join("vpn233-rust-linux-x86_64.tar.gz");
    mutate_tar(&path, |data| {
        let mut cursor = 0;
        loop {
            let header = tar::Header::from_byte_slice(&data[cursor..cursor + 512]);
            let size = header.size().unwrap() as usize;
            let padding = (512 - size % 512) % 512;
            if header.entry_type().is_file() && padding >= token().len() {
                let start = cursor + 512 + size;
                data[start..start + token().len()].copy_from_slice(&token());
                break;
            }
            cursor += 512 + size + padding;
        }
    });
    redacted_rejection(&assets);
}

#[test]
fn valid_matrix_rejects_pax_metadata_and_missing_second_eoa_block() {
    for variant in ["pax", "one-eoa"] {
        let assets = Assets::new();
        assets.make(&[], None, None);
        assert!(assets.verify().is_empty());
        let path = assets.directory().join("vpn233-rust-linux-x86_64.tar.gz");
        mutate_tar(&path, |data| {
            if variant == "one-eoa" {
                assert!(data[data.len() - 1024..].iter().all(|byte| *byte == 0));
                data.truncate(data.len() - 512);
            } else {
                let body = format!("comment=\0{}\n", address());
                let mut length = body.len() + 2;
                let record = loop {
                    let value = format!("{length} {body}");
                    if value.len() == length {
                        break value.into_bytes();
                    }
                    length = value.len();
                };
                let mut header = tar::Header::new_gnu();
                header.set_path("PaxHeaders/public").unwrap();
                header.set_entry_type(tar::EntryType::XHeader);
                header.set_size(record.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                let mut extra = header.as_bytes().to_vec();
                extra.extend_from_slice(&record);
                extra.resize(512 + record.len().div_ceil(512) * 512, 0);
                data.splice(..0, extra);
            }
        });
        redacted_rejection(&assets);
    }
}
#[cfg(unix)]
#[test]
fn tracked_parent_symlink_is_rejected_without_reading_external_file() {
    use std::os::unix::fs::symlink;
    let repo = repo();
    fs::create_dir(repo.path().join("nested")).unwrap();
    fs::write(repo.path().join("nested/public.txt"), b"public fixture").unwrap();
    git(repo.path(), &["add", "nested/public.txt"]);
    fs::rename(repo.path().join("nested"), repo.path().join("original")).unwrap();
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("public.txt"), token()).unwrap();
    symlink(external.path(), repo.path().join("nested")).unwrap();
    let output = privacy_cli(repo.path(), &[]);
    assert!(!output.status.success());
    let text = output_text(&output);
    assert!(text.contains("repository-symlink"));
    assert!(!text.contains("github-token"));
}
#[cfg(unix)]
#[test]
fn fifo_release_asset_is_rejected_without_opening_it() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let assets = Assets::new();
    assets.make(&[], None, None);
    let path = assets.directory().join("vpn233-rust-linux-x86_64.zip");
    fs::remove_file(&path).unwrap();
    let value = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(value.as_ptr(), 0o600) }, 0);
    let output = support::process_output(
        Command::new(env!("CARGO_BIN_EXE_vpn233-xtask"))
            .arg("verify-release")
            .arg(assets.directory())
            .arg(&assets.version),
        None,
        Duration::from_secs(10),
    )
    .unwrap();
    assert!(!output.status.success());
    assert!(output_text(&output).contains("regular file"));
}
#[cfg(unix)]
#[test]
fn filesystem_symlinks_never_follow_private_targets() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("private.txt"), token()).unwrap();
    let bundle = dir.path().join("bundle");
    fs::create_dir(&bundle).unwrap();
    symlink(
        external.path().join("private.txt"),
        bundle.join("public.txt"),
    )
    .unwrap();
    assert!(archives::create(&bundle, &dir.path().join("out"), 1600000000).is_err());
    assert!(privacy::run(&[
        "--artifacts-only".into(),
        "--artifacts".into(),
        bundle.to_str().unwrap().into()
    ])
    .is_err());
}
