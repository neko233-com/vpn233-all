use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn tool() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vpn233-xtask"))
}
fn command(args: &[&str], input: Option<&[u8]>) -> std::process::Output {
    let mut cmd = Command::new(tool());
    cmd.args(args);
    vpn233_xtask::support::process_output(&mut cmd, input, Duration::from_secs(10)).unwrap()
}

#[test]
fn actual_sh_http_matrix_preserves_all_23_cases() {
    vpn233_xtask::network::run(&["mirror-tests".into(), "--shell".into(), "sh".into()]).unwrap();
}
#[test]
fn actual_powershell_http_matrix_preserves_all_10_groups() {
    vpn233_xtask::network::run(&["mirror-tests".into(), "--shell".into(), "ps".into()]).unwrap();
}

#[test]
fn typed_e2e_data_helpers_replace_expression_evaluation() {
    for (args, data, expected) in [
        (
            vec!["json-get", "/token"],
            r#"{"token":"synthetic-test"}"#,
            "synthetic-test",
        ),
        (vec!["json-length", "/domains"], r#"{"domains":{}}"#, "0"),
        (
            vec!["json-length", "/formats"],
            r#"{"formats":["a","b"]}"#,
            "2",
        ),
        (
            vec!["json-contains", "/protocols", "vless"],
            r#"{"protocols":["vless"]}"#,
            "true",
        ),
        (
            vec!["provider-online", "fixture"],
            r#"{"providers":[{"provider_id":"fixture","online":true,"node_count":2}]}"#,
            "true",
        ),
        (
            vec!["provider-online", "fixture"],
            r#"{"providers":[{"provider_id":"fixture","online":false,"node_count":2}]}"#,
            "false",
        ),
    ] {
        let mut arguments = vec!["network", "e2e"];
        arguments.extend(args);
        let result = command(&arguments, Some(data.as_bytes()));
        assert!(result.status.success());
        assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), expected);
    }
    for subcommand in ["validate-json", "validate-yaml"] {
        let result = command(
            &["network", "e2e", subcommand],
            Some(b"{synthetic-private-invalid"),
        );
        assert!(!result.status.success());
        assert!(!String::from_utf8_lossy(&result.stderr).contains("synthetic-private-invalid"));
    }
    let result = command(
        &["network", "e2e", "validate-links", "2"],
        Some(b"vless://example.invalid\nss://example.invalid\n"),
    );
    assert!(result.status.success());
    assert!(!command(
        &["network", "e2e", "validate-links", "1"],
        Some(b"unknown://example.invalid")
    )
    .status
    .success());
}

#[test]
fn cryptographic_random_ports_and_base64_are_rust_owned() {
    let a = command(&["network", "e2e", "random-hex", "16"], None);
    let b = command(&["network", "e2e", "random-hex", "16"], None);
    assert!(a.status.success() && b.status.success());
    assert_eq!(String::from_utf8_lossy(&a.stdout).trim().len(), 32);
    assert_ne!(a.stdout, b.stdout);
    let ports = command(&["network", "e2e", "free-ports", "5"], None);
    assert!(ports.status.success());
    let values: std::collections::BTreeSet<u16> = String::from_utf8(ports.stdout)
        .unwrap()
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(values.len(), 5);
    assert!(!values.contains(&0));
    let encoded = command(
        &["network", "e2e", "base64-encode"],
        Some(b"synthetic\0payload"),
    );
    assert!(encoded.status.success());
    let decoded = command(&["network", "e2e", "base64-decode"], Some(&encoded.stdout));
    assert!(decoded.status.success());
    assert_eq!(decoded.stdout, b"synthetic\0payload");
    assert!(!command(
        &["network", "e2e", "base64-decode"],
        Some(b"invalid!base64")
    )
    .status
    .success());
}

struct Echo(std::process::Child);
impl Drop for Echo {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn echo_port_zero_private_readiness_get_head_and_shutdown() {
    let temp = tempfile::tempdir().unwrap();
    let ready = temp.path().join("ready");
    let child = Command::new(tool())
        .args([
            "network",
            "echo",
            "--port",
            "0",
            "--marker",
            "change-me-private-marker",
            "--ready-file",
        ])
        .arg(&ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = Echo(child);
    let begin = Instant::now();
    while !ready.is_file() || fs::metadata(&ready).unwrap().len() == 0 {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "echo exited before private readiness"
        );
        assert!(begin.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(20));
    }
    let port: u16 = fs::read_to_string(&ready).unwrap().trim().parse().unwrap();
    assert!(port > 0);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&ready).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let response: serde_json::Value = client
        .get(format!("{base}/fixture/test"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(response["marker"], "change-me-private-marker");
    assert_eq!(response["path"], "/fixture/test");
    let head = client.head(format!("{base}/fixture/head")).send().unwrap();
    assert_eq!(head.status(), 200);
    assert!(head.bytes().unwrap().is_empty());
    #[cfg(unix)]
    {
        assert_eq!(unsafe { libc::kill(child.0.id() as i32, libc::SIGTERM) }, 0);
    }
    #[cfg(windows)]
    child.0.kill().unwrap();
    let begin = Instant::now();
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            #[cfg(unix)]
            {
                assert!(status.success());
                assert!(!ready.exists());
            }
            #[cfg(windows)]
            let _ = status;
            break;
        }
        assert!(
            begin.elapsed() < Duration::from_secs(3),
            "echo shutdown hung"
        );
        thread::sleep(Duration::from_millis(20));
    }
    use std::io::Read;
    let mut stdout = String::new();
    let mut stderr = String::new();
    child
        .0
        .stdout
        .as_mut()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    child
        .0
        .stderr
        .as_mut()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(!format!("{stdout}{stderr}").contains("change-me-private-marker"));
}

#[test]
fn invalid_echo_readiness_and_ports_never_overwrite_or_disclose() {
    let temp = tempfile::tempdir().unwrap();
    let existing = temp.path().join("existing");
    fs::write(&existing, b"retained synthetic content").unwrap();
    let paths = vec![
        temp.path().to_owned(),
        temp.path().join("missing/ready"),
        existing.clone(),
    ];
    #[cfg(unix)]
    let paths = {
        let mut paths = paths;
        let symlink = temp.path().join("symlink");
        std::os::unix::fs::symlink(&existing, &symlink).unwrap();
        paths.push(symlink);
        paths
    };
    for path in paths {
        let mut cmd = Command::new(tool());
        cmd.args([
            "network",
            "echo",
            "--port",
            "0",
            "--marker",
            "change-me-private-marker",
            "--ready-file",
        ])
        .arg(path);
        let output =
            vpn233_xtask::support::process_output(&mut cmd, None, Duration::from_secs(5)).unwrap();
        assert!(!output.status.success());
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        assert!(!diagnostics.contains("change-me-private-marker"));
        assert!(!diagnostics.contains(&temp.path().to_string_lossy().into_owned()));
    }
    assert_eq!(fs::read(existing).unwrap(), b"retained synthetic content");
    assert!(!command(&["network", "echo", "--port", "65536"], None)
        .status
        .success());
    assert!(
        !command(&["network", "echo", "--bind", "example.invalid"], None)
            .status
            .success()
    );
}

#[test]
fn official_parser_rejects_tampered_archive_before_spawning_provider() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("private-archive");
    fs::write(&archive, b"wrong official archive").unwrap();
    let args = vec![
        "mihomo".into(),
        "--provider-bin".into(),
        tool().to_string_lossy().into_owned(),
        "--mihomo-archive".into(),
        archive.to_string_lossy().into_owned(),
    ];
    let error = vpn233_xtask::network::run(&args).unwrap_err().to_string();
    assert!(error.contains("SHA-256 mismatch"));
    assert!(!error.contains(&temp.path().to_string_lossy().into_owned()));
}
