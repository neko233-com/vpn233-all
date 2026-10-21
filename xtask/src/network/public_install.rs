//! Post-publication customer-path gate. All private state stays in a temporary
//! user-owned directory; local bundles can never replace an online installation.
use super::{option, path_arg, private_write, program};
use anyhow::{anyhow, bail, ensure, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand_core::{OsRng, RngCore};
use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fs,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

const REPO: &str = "neko233-com/vpn233-all";
const PROGRAMS: [&str; 3] = [
    "vpn233-provider-server",
    "vpn233-subscribe-server",
    "vpn233-client",
];
const CONFIGS: [&str; 3] = [
    "provider-server.yaml",
    "subscribe-server.yaml",
    "vpn233-client.yaml",
];

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn random() -> String {
    let mut value = [0; 16];
    OsRng.fill_bytes(&mut value);
    value.iter().map(|b| format!("{b:02x}")).collect()
}
fn stage<T>(label: &str, work: impl FnOnce() -> Result<T>) -> Result<T> {
    let start = Instant::now();
    println!("public-install: {label} running");
    let result = work().map_err(|_| anyhow!("public customer acceptance failed at controlled stage {label}; private output was not printed"))?;
    println!(
        "public-install: {label} passed ({:.3}s)",
        start.elapsed().as_secs_f64()
    );
    Ok(result)
}

pub(super) fn run(args: &[String]) -> Result<()> {
    let tag =
        option(args, "--tag")?.ok_or_else(|| anyhow!("public-install requires a fixed tag"))?;
    let commit = option(args, "--commit")?
        .ok_or_else(|| anyhow!("public-install requires a full commit"))?;
    ensure!(
        Regex::new(r"^v[0-9]+\.[0-9]+\.[0-9]+$")?.is_match(&tag),
        "invalid fixed publication tag"
    );
    ensure!(
        Regex::new(r"^[a-f0-9]{40}$")?.is_match(&commit),
        "invalid full publication commit"
    );
    let mirror = option(args, "--mirror")?.unwrap_or_else(|| "auto".into());
    let prefixes = mirrors(&mirror)?;
    ensure!(
        cfg!(windows) || cfg!(target_os = "linux"),
        "public customer gate supports native Windows and Linux"
    );
    #[cfg(unix)]
    ensure!(
        unsafe { libc::geteuid() } != 0,
        "public customer acceptance must run without root privileges"
    );
    let root = crate::support::root()?;
    let start = Instant::now();
    let git =
        program(&["git"]).ok_or_else(|| anyhow!("Git is required for publication source proof"))?;
    stage("source-commit", || {
        let mut cmd = local_command(&git);
        cmd.current_dir(&root).args(["rev-parse", "HEAD"]);
        let head = crate::support::process_output(&mut cmd, None, Duration::from_secs(15))?;
        ensure!(
            head.status.success() && String::from_utf8(head.stdout)?.trim() == commit,
            "checkout differs from the published commit"
        );
        let mut cmd = local_command(&git);
        cmd.current_dir(&root)
            .args(["status", "--porcelain", "--untracked-files=no"]);
        let state = crate::support::process_output(&mut cmd, None, Duration::from_secs(15))?;
        ensure!(
            state.status.success() && state.stdout.is_empty(),
            "publication checkout has tracked modifications"
        );
        Ok(())
    })?;
    let metadata = stage("public-repository-release-tag-api", || {
        metadata(
            &tag,
            &commit,
            option(args, "--public-api-proof")?.as_deref(),
        )
    })?;
    let assets = metadata["release"]["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("public asset metadata missing"))?;
    let asset_meta = |name: &str| -> Result<(String, u64)> {
        let items: Vec<_> = assets
            .iter()
            .filter(|asset| asset["name"].as_str() == Some(name))
            .collect();
        ensure!(items.len() == 1, "public asset missing or duplicate");
        let digest = items[0]["digest"]
            .as_str()
            .and_then(|s| s.strip_prefix("sha256:"))
            .ok_or_else(|| anyhow!("public API SHA-256 absent"))?;
        ensure!(
            Regex::new(r"^[a-f0-9]{64}$")?.is_match(digest),
            "public API SHA-256 invalid"
        );
        let size = items[0]["size"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or_else(|| anyhow!("public API asset size missing"))?;
        Ok((digest.into(), size))
    };
    let work = tempfile::Builder::new()
        .prefix("vpn233-public-customer-")
        .tempdir()?;
    let windows = cfg!(windows);
    let shell = program(if windows {
        &["powershell", "pwsh"]
    } else {
        &["bash"]
    })
    .ok_or_else(|| anyhow!("native installer shell unavailable"))?;
    let mut environment = environment(work.path());
    protect_temporary_directory(work.path(), &shell, &environment)?;
    let bootstrap = work.path().join("bootstrap");
    let prefix = work.path().join("installed");
    let logs = work.path().join("diagnostics");
    fs::create_dir(&bootstrap)?;
    fs::create_dir(&logs)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bootstrap, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&logs, fs::Permissions::from_mode(0o700))?;
    }
    let suffix = if windows { ".ps1" } else { ".sh" };
    let sources = if windows {
        vec![
            "install.ps1",
            "scripts/lib/mirrors.ps1",
            "scripts/lib/layout.ps1",
        ]
    } else {
        vec![
            "install.sh",
            "scripts/lib/mirrors.sh",
            "scripts/lib/layout.sh",
            "scripts/lib/services.sh",
        ]
    };
    stage("fixed-tag-and-commit-public-bootstrap", || {
        for source in sources {
            let mut cmd = local_command(&git);
            cmd.current_dir(&root)
                .args(["show", &format!("{commit}:{source}")]);
            let blob = crate::support::process_output(&mut cmd, None, Duration::from_secs(15))?;
            ensure!(blob.status.success(), "source Git object unavailable");
            let digest = hash(&blob.stdout);
            let length = blob.stdout.len() as u64;
            let path = bootstrap.join(source);
            fs::create_dir_all(path.parent().unwrap())?;
            download(
                &format!("https://raw.githubusercontent.com/{REPO}/{tag}/{source}"),
                &path,
                &digest,
                length,
                &prefixes,
            )?;
            let commit_copy = work.path().join("commit-source-proof");
            download(
                &format!("https://raw.githubusercontent.com/{REPO}/{commit}/{source}"),
                &commit_copy,
                &digest,
                length,
                &prefixes,
            )?;
            fs::remove_file(commit_copy)?;
        }
        Ok(())
    })?;
    let sums = work.path().join("SHA256SUMS");
    let asset = if windows {
        "vpn233-rust-windows-x86_64.zip"
    } else {
        "vpn233-rust-linux-x86_64.tar.gz"
    };
    let (sums_digest, sums_size) = asset_meta("SHA256SUMS")?;
    let (archive_digest, archive_size) = asset_meta(asset)?;
    let reference = work.path().join(asset);
    stage("api-manifest-and-reference-archive-digests", || {
        download(
            &format!("https://github.com/{REPO}/releases/download/{tag}/SHA256SUMS"),
            &sums,
            &sums_digest,
            sums_size,
            &prefixes,
        )?;
        let lines: Vec<_> = fs::read_to_string(&sums)?
            .lines()
            .filter_map(|line| {
                let mut words = line.split_whitespace();
                let digest = words.next()?;
                let name = words.next()?.trim_start_matches('*');
                (name == asset).then_some(digest.to_owned())
            })
            .collect();
        ensure!(
            lines == [archive_digest.clone()],
            "manifest archive digest differs from official API"
        );
        download(
            &format!("https://github.com/{REPO}/releases/download/{tag}/{asset}"),
            &reference,
            &archive_digest,
            archive_size,
            &prefixes,
        )
    })?;
    let reference_files = archive_contents(&reference, windows)?;
    let admin_password = format!("synthetic-admin-{}", random());
    environment.extend([
        ("VPN233_ADMIN_USER".into(), "synthetic-admin".into()),
        (
            "VPN233_ADMIN_PASSWORD".into(),
            admin_password.clone().into(),
        ),
        (
            "VPN233_PROVIDER_FEED_TOKEN".into(),
            format!("synthetic-feed-{}", random()).into(),
        ),
        (
            "VPN233_REGISTRY_TOKEN".into(),
            format!("synthetic-registry-{}", random()).into(),
        ),
    ]);
    let listeners = (0..5)
        .map(|_| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)))
        .collect::<std::io::Result<Vec<_>>>()?;
    let ports = listeners
        .iter()
        .map(|l| l.local_addr().map(|a| a.port()))
        .collect::<std::io::Result<Vec<_>>>()?;
    drop(listeners);
    for (key, port) in [
        "VPN233_PROVIDER_PORT",
        "VPN233_SUBSCRIBE_PORT",
        "VPN233_CLIENT_SOCKS_PORT",
        "VPN233_CLIENT_HTTP_PORT",
        "VPN233_CLIENT_WEB_PORT",
    ]
    .iter()
    .zip(&ports)
    {
        environment.push(((*key).into(), port.to_string().into()));
    }
    let run_script = |label: &str, path: &Path, args: &[String], limit: Duration| -> Result<()> {
        let mut cmd = script_command(&shell, path, &environment);
        cmd.args(args).current_dir(&root);
        let output = crate::support::process_output(&mut cmd, None, limit)
            .map_err(|_| anyhow!("customer script process failed or timed out"))?;
        private_write(&logs.join(format!("{label}.stdout")), &output.stdout)?;
        private_write(&logs.join(format!("{label}.stderr")), &output.stderr)?;
        ensure!(output.status.success(), "customer script returned failure");
        if !windows && (label == "install" || label == "update") {
            ensure!(
                String::from_utf8_lossy(&output.stdout).contains("SHA-256 verified"),
                "customer script did not attest strict archive SHA-256"
            );
        }
        Ok(())
    };
    let install_args: Vec<String> = if windows {
        vec![
            "-Release".into(),
            "-Version".into(),
            tag.clone(),
            "-Mirror".into(),
            mirror.clone(),
            "-Prefix".into(),
            path_arg(&prefix),
            "-NoStart".into(),
        ]
    } else {
        vec![
            "--release".into(),
            "--version".into(),
            tag.clone(),
            "--mirror".into(),
            mirror.clone(),
            "--prefix".into(),
            path_arg(&prefix),
            "--no-start".into(),
            "--yes".into(),
        ]
    };
    stage("actual-public-online-install", || {
        run_script(
            "install",
            &bootstrap.join(format!("install{suffix}")),
            &install_args,
            Duration::from_secs(3600),
        )
    })?;
    let mut cleanup = Installation {
        prefix: prefix.clone(),
        shell: shell.clone(),
        environment: environment.clone(),
    };
    stage("installed-payload-and-versions-private-permissions", || {
        compare_payload(&prefix, &reference_files)?;
        versions(&prefix, &tag, &environment)?;
        permissions(&prefix, &shell, &environment)
    })?;
    let default_node_ip = Regex::new(r"(?m)^default_node_ip:.*$")?;
    let auto_detect_public_ip = Regex::new(r"(?m)^auto_detect_public_ip:.*$")?;
    for name in CONFIGS {
        let path = prefix.join("config").join(name);
        let mut text = fs::read_to_string(&path)?;
        if name == "provider-server.yaml" {
            text = default_node_ip
                .replace(&text, "default_node_ip: '127.0.0.1'")
                .into_owned();
            text = auto_detect_public_ip
                .replace(&text, "auto_detect_public_ip: false")
                .into_owned();
        }
        text.push_str("\n# synthetic public upgrade preservation sentinel\n");
        fs::write(path, text)?;
    }
    let sentinel = prefix.join("data/synthetic-preservation.txt");
    private_write(
        &sentinel,
        b"synthetic unrelated data must survive actual online update\n",
    )?;
    let before = snapshot(&prefix)?;
    let update_args: Vec<String> = if windows {
        vec![
            "-Version".into(),
            tag.clone(),
            "-Mirror".into(),
            mirror.clone(),
            "-NoStart".into(),
        ]
    } else {
        vec![
            "--version".into(),
            tag.clone(),
            "--mirror".into(),
            mirror.clone(),
            "--no-start".into(),
            "--yes".into(),
        ]
    };
    stage(
        "actual-installed-online-update-preserves-config-data-layout",
        || {
            run_script(
                "update",
                &prefix.join(format!("update{suffix}")),
                &update_args,
                Duration::from_secs(3600),
            )?;
            ensure!(
                snapshot(&prefix)? == before,
                "online update changed configuration or data bytes"
            );
            compare_payload(&prefix, &reference_files)?;
            versions(&prefix, &tag, &environment)?;
            permissions(&prefix, &shell, &environment)
        },
    )?;
    stage("start-health-ui-provider-and-subscription", || {
        run_script(
            "start",
            &prefix.join(format!("start{suffix}")),
            &[],
            Duration::from_secs(60),
        )?;
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()?;
        wait_http(&client, ports[0], "/api/v1/health", None)?;
        wait_http(
            &client,
            ports[1],
            "/api/status",
            Some((&admin_password, "synthetic-admin")),
        )?;
        wait_http(&client, ports[4], "/api/v1/health", None)?;
        let base = format!("http://127.0.0.1:{}", ports[1]);
        let page = client
            .get(format!("{base}/"))
            .send()?
            .error_for_status()?
            .text()?;
        ensure!(
            page.contains("id=\"root\""),
            "installed UI root unavailable"
        );
        for resource in Regex::new(r#"(?:src|href)="([^" ]+\.(?:js|css))"#)?.captures_iter(&page) {
            let path = &resource[1];
            ensure!(
                path.starts_with('/') && !path.starts_with("//"),
                "installed UI attempted external resource"
            );
            client
                .get(format!("{base}{path}"))
                .send()?
                .error_for_status()?;
        }
        let begin = Instant::now();
        loop {
            let inventory: Value = client
                .get(format!("{base}/api/admin/providers"))
                .basic_auth("synthetic-admin", Some(&admin_password))
                .send()?
                .error_for_status()?
                .json()?;
            if inventory["providers"].as_array().is_some_and(|list| {
                list.iter().any(|p| {
                    p["online"].as_bool() == Some(true) && p["node_count"].as_u64().unwrap_or(0) > 0
                })
            }) {
                break;
            }
            ensure!(
                begin.elapsed() < Duration::from_secs(45),
                "installed provider registry readiness failed"
            );
            thread::sleep(Duration::from_millis(300));
        }
        let token = format!("synthetic-token-{}", random());
        let password = format!("synthetic-subscription-{}", random());
        client
            .post(format!("{base}/api/admin/tokens"))
            .basic_auth("synthetic-admin", Some(&admin_password))
            .json(&json!({"token":token,"password":password,"limit_bytes":1000000}))
            .send()?
            .error_for_status()?;
        let response = client
            .get(format!("{base}/sub/clash"))
            .basic_auth(&token, Some(&password))
            .send()?
            .error_for_status()?;
        ensure!(
            response
                .headers()
                .get("X-VPN233-Rendered")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0)
                > 0,
            "synthetic subscription rendered no nodes"
        );
        Ok(())
    })?;
    let pids: Vec<u32> = fs::read_dir(prefix.join("runtime"))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|s| s == "pid"))
        .map(|p| {
            fs::read_to_string(p)?
                .trim()
                .parse::<u32>()
                .map_err(anyhow::Error::from)
        })
        .collect::<Result<_>>()?;
    ensure!(
        pids.len() == 3
            && pids.iter().collect::<std::collections::BTreeSet<_>>().len() == 3
            && pids.iter().all(|id| *id > 0),
        "installation did not track exactly three process IDs"
    );
    #[cfg(target_os = "linux")]
    for pid in &pids {
        let executable = fs::read_link(format!("/proc/{pid}/exe"))?;
        ensure!(
            executable.starts_with(prefix.join("bin")),
            "recorded process is not owned by the temporary installation"
        );
    }
    #[cfg(windows)]
    {
        let script = "$ErrorActionPreference='Stop';foreach($name in @('provider-server','subscribe-server','vpn233-client')){$file=Join-Path $env:VPN233_TEST_PREFIX ('runtime/'+$name+'.pid');$id=[int]([IO.File]::ReadAllText($file).Trim());$running=Get-Process -Id $id -ErrorAction Stop;$binary=if($name -eq 'vpn233-client'){$name}else{'vpn233-'+$name};$expected=[IO.Path]::GetFullPath((Join-Path $env:VPN233_TEST_PREFIX ('bin/'+$binary+'.exe')));if($running.Path -ne $expected){exit 1};$identity=[IO.File]::ReadAllText($file+'.identity').Trim();if($identity -ne [string]$running.StartTime.ToUniversalTime().Ticks){exit 1}}";
        let mut cmd = script_command_inline(&shell, script, &environment);
        cmd.env("VPN233_TEST_PREFIX", &prefix);
        ensure!(
            crate::support::process_output(&mut cmd, None, Duration::from_secs(30))?
                .status
                .success(),
            "recorded Windows process is not owned by the temporary installation"
        );
    }
    stage("stop-exact-three-owned-processes", || {
        run_script(
            "stop",
            &prefix.join(format!("stop{suffix}")),
            &[],
            Duration::from_secs(60),
        )?;
        ensure!(
            !fs::read_dir(prefix.join("runtime"))?.any(|entry| entry
                .ok()
                .is_some_and(|e| e.path().extension().is_some_and(|s| s == "pid"))),
            "stop retained PID metadata"
        );
        for port in &ports {
            ensure!(
                TcpStream::connect_timeout(
                    &SocketAddr::from((Ipv4Addr::LOCALHOST, *port)),
                    Duration::from_millis(300)
                )
                .is_err(),
                "installed listener survived stop"
            );
        }
        #[cfg(target_os = "linux")]
        for pid in &pids {
            ensure!(
                !PathBuf::from(format!("/proc/{pid}/exe")).exists(),
                "owned process survived stop"
            );
        }
        #[cfg(windows)]
        {
            let mut cmd = script_command_inline(&shell, "$ids=$env:VPN233_TEST_PIDS -split ','; foreach($id in $ids){if(Get-Process -Id ([int]$id) -ErrorAction SilentlyContinue){exit 1}}", &environment);
            cmd.env(
                "VPN233_TEST_PIDS",
                pids.iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            );
            ensure!(
                crate::support::process_output(&mut cmd, None, Duration::from_secs(15))?
                    .status
                    .success(),
                "owned Windows process survived stop"
            );
        }
        for (path, expected) in &before {
            ensure!(
                crate::support::sha256_file(&prefix.join(path))? == *expected,
                "runtime changed original configuration or unrelated data"
            );
        }
        Ok(())
    })?;
    stage("installed-binaries-and-installed-ui-full-14-e2e", || {
        let script = root.join(if windows {
            "scripts/e2e/e2e.ps1"
        } else {
            "scripts/e2e/e2e.sh"
        });
        let mut cmd = script_command(&shell, &script, &environment);
        cmd.current_dir(&root)
            .env("VPN233_XTASK_BIN", env::current_exe()?)
            .env("BIN_DIR", prefix.join("bin"))
            .env("E2E_WEB_DIR", prefix.join("admin-web/dist"));
        if windows {
            cmd.arg("-BinDir")
                .arg(prefix.join("bin"))
                .arg("-WebDir")
                .arg(prefix.join("admin-web/dist"));
        }
        let output = crate::support::process_output(&mut cmd, None, Duration::from_secs(600))?;
        private_write(&logs.join("e2e.stdout"), &output.stdout)?;
        private_write(&logs.join("e2e.stderr"), &output.stderr)?;
        ensure!(
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("E2E PASSED"),
            "installed application full E2E failed"
        );
        Ok(())
    })?;
    cleanup.stop();
    println!(
        "{}",
        json!({"mode":"real-anonymous-public-install-and-installed-online-update","tag":tag,"commit":commit,"mirror_mode":mirror,"strict_api_manifest_archive_sha256":true,"binary_versions":6,"config_data_bytes_preserved":true,"health_endpoints":3,"ui_provider_subscription":true,"owned_processes_stopped":3,"installed_application_e2e_steps":14,"private_logs":"temporary user-owned only; never uploaded or printed","seconds":start.elapsed().as_secs_f64()})
    );
    Ok(())
}

fn metadata(tag: &str, commit: &str, proof: Option<&str>) -> Result<Value> {
    let data = if let Some(path) = proof {
        serde_json::from_slice(&fs::read(path)?)?
    } else {
        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;
        let token = env::var("GH_TOKEN")
            .or_else(|_| env::var("GITHUB_TOKEN"))
            .ok();
        let api = |path: &str| -> Result<Value> {
            let mut request = client
                .get(format!("https://api.github.com/repos/{REPO}{path}"))
                .header("User-Agent", "vpn233-public-customer-gate")
                .header("Accept", "application/vnd.github+json");
            if let Some(token) = &token {
                request = request.bearer_auth(token);
            }
            Ok(request.send()?.error_for_status()?.json()?)
        };
        let repository = api("")?;
        let release = api(&format!("/releases/tags/{tag}"))?;
        let mut object = api(&format!("/git/ref/tags/{tag}"))?["object"].clone();
        let sha_pattern = Regex::new(r"^[a-f0-9]{40}$")?;
        for _ in 0..4 {
            if object["type"] == "commit" {
                break;
            }
            ensure!(
                object["type"] == "tag",
                "publication ref has unexpected object type"
            );
            let sha = object["sha"]
                .as_str()
                .ok_or_else(|| anyhow!("tag object SHA missing"))?;
            ensure!(sha_pattern.is_match(sha), "invalid tag object SHA");
            object = api(&format!("/git/tags/{sha}"))?["object"].clone();
        }
        ensure!(
            object["type"] == "commit",
            "publication ref did not resolve to a commit"
        );
        json!({"repository":repository,"release":release,"tag_commit":object["sha"]})
    };
    ensure!(
        data["repository"]["full_name"] == REPO
            && data["repository"]["private"] == false
            && data["repository"]["fork"] == false,
        "repository must be independently public"
    );
    ensure!(
        data["release"]["tag_name"] == tag
            && data["release"]["draft"] == false
            && data["release"]["prerelease"] == false
            && data["release"]["published_at"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
            && data["tag_commit"] == commit,
        "release is not formally published at the expected commit"
    );
    let assets = data["release"]["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("published assets missing"))?;
    let mut names = vec!["SHA256SUMS".to_owned(), "version.txt".to_owned()];
    for platform in [
        "windows-x86_64",
        "linux-x86_64",
        "macos-x86_64",
        "macos-aarch64",
    ] {
        for extension in ["tar.gz", "zip"] {
            names.push(format!("vpn233-rust-{platform}.{extension}"));
        }
    }
    ensure!(
        names.iter().all(|name| assets
            .iter()
            .filter(|asset| asset["name"].as_str() == Some(name))
            .count()
            == 1),
        "formal release lacks its complete asset matrix"
    );
    let digest_pattern = Regex::new(r"^sha256:[a-f0-9]{64}$")?;
    ensure!(
        names.iter().all(|name| assets
            .iter()
            .find(|asset| asset["name"].as_str() == Some(name))
            .is_some_and(|asset| asset["size"].as_u64().unwrap_or(0) > 0
                && asset["digest"]
                    .as_str()
                    .is_some_and(|s| digest_pattern.is_match(s)))),
        "formal release has an asset size or digest metadata gap"
    );
    Ok(data)
}

fn mirrors(mode: &str) -> Result<Vec<String>> {
    if mode == "off" {
        return Ok(vec![String::new()]);
    }
    if mode == "auto" {
        return Ok([
            "https://ghproxy.net/",
            "https://gh.llkk.cc/",
            "https://ghfast.top/",
            "https://gh-proxy.com/",
            "",
        ]
        .iter()
        .map(|s| (*s).into())
        .collect());
    }
    let url = reqwest::Url::parse(mode)?;
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url
                .host_str()
                .is_some_and(|host| !host.eq_ignore_ascii_case("localhost")
                    && host.parse::<std::net::IpAddr>().is_err()),
        "mirror must be a public HTTPS prefix without credentials"
    );
    Ok(vec![format!("{}/", mode.trim_end_matches('/'))])
}

fn download(
    url: &str,
    destination: &Path,
    expected: &str,
    size: u64,
    prefixes: &[String],
) -> Result<()> {
    ensure!(
        Regex::new(r"^[a-f0-9]{64}$")?.is_match(expected) && size > 0,
        "download requires trusted size and SHA-256"
    );
    for prefix in prefixes {
        let mut partial = tempfile::NamedTempFile::new_in(destination.parent().unwrap())?;
        let begin = Instant::now();
        for _ in 0..3 {
            let Some(remaining) = Duration::from_secs(300).checked_sub(begin.elapsed()) else {
                break;
            };
            if remaining < Duration::from_millis(1) {
                break;
            }
            let offset = partial.as_file().metadata()?.len();
            let client = reqwest::blocking::Client::builder()
                .https_only(true)
                .connect_timeout(remaining.min(Duration::from_secs(10)))
                .timeout(remaining)
                .build()?;
            let mut request = client
                .get(format!("{prefix}{url}"))
                .header("Accept-Encoding", "identity")
                .header("User-Agent", "vpn233-public-customer-gate");
            if offset > 0 {
                request = request.header("Range", format!("bytes={offset}-"));
            }
            let Ok(mut response) = request.send() else {
                break;
            };
            if !response.status().is_success() {
                break;
            }
            if response
                .headers()
                .get("content-encoding")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|s| s != "identity")
            {
                break;
            }
            if offset > 0 {
                if response.status() == reqwest::StatusCode::OK {
                    partial.as_file_mut().set_len(0)?;
                    use std::io::Seek;
                    partial.as_file_mut().rewind()?;
                } else if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
                    || !response
                        .headers()
                        .get("content-range")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .starts_with(&format!("bytes {offset}-"))
                {
                    break;
                }
            }
            let mut complete = true;
            loop {
                let mut buffer = [0; 65536];
                match response.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if partial.as_file().metadata()?.len() + count as u64 > size {
                            complete = false;
                            break;
                        }
                        partial.as_file_mut().write_all(&buffer[..count])?;
                    }
                    Err(_) => {
                        complete = false;
                        break;
                    }
                }
            }
            if partial.as_file().metadata()?.len() == size
                && crate::support::sha256_file(partial.path())? == expected
            {
                partial
                    .persist(destination)
                    .map_err(|_| anyhow!("verified public download move failed"))?;
                return Ok(());
            }
            if complete || partial.as_file().metadata()?.len() == 0 {
                break;
            }
        }
    }
    bail!("all public download sources failed transport or digest verification")
}

fn archive_contents(path: &Path, windows: bool) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut result = BTreeMap::new();
    let mut insert = |name: String, bytes: Vec<u8>| -> Result<()> {
        ensure!(
            !name.contains('\\') && !name.split('/').any(|p| p == "..") && !name.starts_with('/'),
            "unsafe reference archive entry"
        );
        let (_, relative) = name
            .split_once('/')
            .ok_or_else(|| anyhow!("reference archive root missing"))?;
        ensure!(
            result.insert(relative.to_owned(), bytes).is_none(),
            "duplicate reference archive path"
        );
        Ok(())
    };
    if windows {
        let mut zip = zip::ZipArchive::new(fs::File::open(path)?)?;
        for index in 0..zip.len() {
            let mut item = zip.by_index(index)?;
            if item.is_dir() {
                continue;
            }
            ensure!(
                item.size() < 512 * 1024 * 1024,
                "reference archive entry exceeds bound"
            );
            let name = item.name()?.into_owned();
            let mut bytes = Vec::new();
            item.read_to_end(&mut bytes)?;
            insert(name, bytes)?;
        }
    } else {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(path)?));
        for item in tar.entries()? {
            let mut item = item?;
            if item.header().entry_type().is_dir() {
                continue;
            }
            ensure!(
                item.header().entry_type().is_file() && item.size() < 512 * 1024 * 1024,
                "nonregular reference archive entry"
            );
            let name = item.path()?.to_string_lossy().into_owned();
            let mut bytes = Vec::new();
            item.read_to_end(&mut bytes)?;
            insert(name, bytes)?;
        }
    }
    Ok(result)
}

fn compare_payload(prefix: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    for (name, bytes) in files {
        if name.starts_with("bin/")
            || name.starts_with("admin-web/dist/")
            || name.starts_with("scripts/lib/")
            || [
                "install.sh",
                "install.ps1",
                "start.sh",
                "start.ps1",
                "stop.sh",
                "stop.ps1",
                "update.sh",
                "update.ps1",
                "version.txt",
            ]
            .contains(&name.as_str())
        {
            let path = prefix.join(name);
            if path.is_file() {
                ensure!(
                    payload_matches(name, &fs::read(path)?, bytes),
                    "installed application differs from API-verified archive"
                );
            } else {
                ensure!(
                    !name.starts_with("bin/") && !name.starts_with("admin-web/dist/"),
                    "installed application file missing"
                );
            }
        }
    }
    Ok(())
}
fn payload_matches(name: &str, installed: &[u8], reference: &[u8]) -> bool {
    // Raw fixed-commit helpers are LF; release checkouts use .gitattributes CRLF.
    // Their source bytes were independently verified before execution. Accept
    // only that known transport distinction; binaries, UI and other files stay
    // byte-exact against the API-verified archive.
    if matches!(name, "scripts/lib/mirrors.ps1" | "scripts/lib/layout.ps1") {
        let normalize = |bytes: &[u8]| {
            bytes
                .iter()
                .enumerate()
                .filter(|(index, byte)| **byte != b'\r' || bytes.get(index + 1) != Some(&b'\n'))
                .map(|(_, byte)| *byte)
                .collect::<Vec<_>>()
        };
        normalize(installed) == normalize(reference)
    } else {
        installed == reference
    }
}
fn environment(work: &Path) -> Vec<(OsString, OsString)> {
    let allowed = [
        "PATH",
        "HOME",
        "USERPROFILE",
        "LANG",
        "LC_ALL",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "LOCALAPPDATA",
        "APPDATA",
        "PATHEXT",
        "OS",
        "PROCESSOR_ARCHITECTURE",
        "PROCESSOR_ARCHITEW6432",
    ];
    let mut values: Vec<_> = env::vars_os()
        .filter(|(key, _)| allowed.contains(&key.to_string_lossy().to_ascii_uppercase().as_str()))
        .collect();
    for (key, name) in [
        ("XDG_CONFIG_HOME", "xdg-config"),
        ("XDG_CACHE_HOME", "xdg-cache"),
        ("XDG_DATA_HOME", "xdg-data"),
        ("VPN233_HOME", "mirror-cache"),
    ] {
        values.push((key.into(), work.join(name).into_os_string()));
    }
    values
}
fn protect_temporary_directory(
    path: &Path,
    shell: &Path,
    environment: &[(OsString, OsString)],
) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        let _ = (shell, environment);
    }
    #[cfg(windows)]
    {
        let script = "$ErrorActionPreference='Stop';$path=$env:VPN233_TEST_PRIVATE_DIR;$acl=Get-Acl -LiteralPath $path;$acl.SetAccessRuleProtection($true,$false);foreach($rule in @($acl.Access)){[void]$acl.RemoveAccessRuleSpecific($rule)};$current=[Security.Principal.WindowsIdentity]::GetCurrent().User;$acl.SetOwner($current);foreach($sid in @($current,(New-Object Security.Principal.SecurityIdentifier 'S-1-5-18'),(New-Object Security.Principal.SecurityIdentifier 'S-1-5-32-544'))){$acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow')))};Set-Acl -LiteralPath $path -AclObject $acl";
        let mut cmd = script_command_inline(shell, script, environment);
        cmd.env("VPN233_TEST_PRIVATE_DIR", path);
        ensure!(
            crate::support::process_output(&mut cmd, None, Duration::from_secs(30))?
                .status
                .success(),
            "private temporary directory ACL protection failed"
        );
    }
    Ok(())
}
fn local_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    for name in [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
    ] {
        command.env_remove(name);
    }
    command
}
fn script_command(shell: &Path, path: &Path, environment: &[(OsString, OsString)]) -> Command {
    let mut command = Command::new(shell);
    command.env_clear().envs(environment.iter().cloned());
    if cfg!(windows) {
        command
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(path);
    } else {
        command.arg(path_arg(path));
    }
    command
}
#[cfg(windows)]
fn script_command_inline(
    shell: &Path,
    script: &str,
    environment: &[(OsString, OsString)],
) -> Command {
    let mut command = Command::new(shell);
    command
        .env_clear()
        .envs(environment.iter().cloned())
        .args(["-NoProfile", "-Command", script]);
    command
}
fn versions(prefix: &Path, tag: &str, environment: &[(OsString, OsString)]) -> Result<()> {
    #[cfg(target_os = "linux")]
    let glibc_symbols = Regex::new(r"GLIBC_([0-9]+)\.([0-9]+)")?;
    for name in PROGRAMS {
        let binary = prefix
            .join("bin")
            .join(format!("{name}{}", if cfg!(windows) { ".exe" } else { "" }));
        let mut cmd = Command::new(binary);
        cmd.env_clear()
            .envs(environment.iter().cloned())
            .arg("--version");
        let output = crate::support::process_output(&mut cmd, None, Duration::from_secs(30))?;
        ensure!(
            output.status.success()
                && output.stderr.is_empty()
                && String::from_utf8_lossy(&output.stdout).contains(tag.trim_start_matches('v')),
            "installed version or loader warning check failed"
        );
        #[cfg(target_os = "linux")]
        {
            let mut cmd = Command::new("readelf");
            cmd.env_clear()
                .envs(environment.iter().cloned())
                .args(["--version-info"])
                .arg(prefix.join("bin").join(name));
            let symbols = crate::support::process_output(&mut cmd, None, Duration::from_secs(30))?;
            ensure!(symbols.status.success(), "ELF version inspection failed");
            let text = String::from_utf8(symbols.stdout)?;
            ensure!(
                !text.contains("GLIBC_PRIVATE") && !text.contains("GLIBC_ABI_DT_RELR"),
                "unsupported glibc ABI"
            );
            for capture in glibc_symbols.captures_iter(&text) {
                ensure!(
                    (capture[1].parse::<u32>()?, capture[2].parse::<u32>()?) <= (2, 35),
                    "public Linux binary exceeds glibc 2.35"
                );
            }
        }
    }
    Ok(())
}
fn permissions(prefix: &Path, shell: &Path, environment: &[(OsString, OsString)]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let uid = unsafe { libc::geteuid() };
        for name in ["config", "data", "logs", "backups"] {
            let path = prefix.join(name);
            if !path.exists() && name == "backups" {
                continue;
            }
            let meta = fs::metadata(path)?;
            ensure!(
                meta.uid() == uid && meta.permissions().mode() & 0o777 == 0o700,
                "private directory ownership or mode failed"
            );
        }
        for name in CONFIGS {
            let meta = fs::metadata(prefix.join("config").join(name))?;
            ensure!(
                meta.uid() == uid && meta.permissions().mode() & 0o777 == 0o600,
                "configuration ownership or mode failed"
            );
        }
        let _ = (shell, environment);
    }
    #[cfg(windows)]
    {
        let script = "$ErrorActionPreference='Stop';$user=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value;$items=@();foreach($p in @('config','data','logs','backups','config/provider-server.yaml','config/subscribe-server.yaml','config/vpn233-client.yaml')){$path=Join-Path $env:VPN233_TEST_PREFIX $p;if(!(Test-Path -LiteralPath $path)){if($p -eq 'backups'){continue};throw 'missing private path'};$acl=Get-Acl -LiteralPath $path;$allow=@($acl.Access|Where-Object {$_.AccessControlType -eq 'Allow'}|ForEach-Object {@{sid=$_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value;rights=[long]$_.FileSystemRights}});$items+=@{rules=$allow}};@{user=$user;items=$items}|ConvertTo-Json -Depth 8 -Compress";
        let mut cmd = script_command_inline(shell, script, environment);
        cmd.env("VPN233_TEST_PREFIX", prefix);
        let output = crate::support::process_output(&mut cmd, None, Duration::from_secs(30))?;
        ensure!(
            output.status.success(),
            "private Windows ACL inspection failed"
        );
        let data: Value = serde_json::from_slice(&output.stdout)?;
        let user = data["user"]
            .as_str()
            .ok_or_else(|| anyhow!("ACL identity missing"))?;
        for item in data["items"]
            .as_array()
            .ok_or_else(|| anyhow!("ACL items missing"))?
        {
            let rules = item["rules"]
                .as_array()
                .ok_or_else(|| anyhow!("ACL rules missing"))?;
            ensure!(
                rules.iter().all(|r| r["sid"] == user
                    || r["sid"] == "S-1-5-18"
                    || r["sid"] == "S-1-5-32-544")
                    && rules
                        .iter()
                        .any(|r| r["sid"] == user && r["rights"].as_i64().unwrap_or(0) & 3 == 3),
                "Windows private ACL grants unintended access"
            );
        }
    }
    Ok(())
}
fn snapshot(prefix: &Path) -> Result<BTreeMap<String, String>> {
    fn walk(prefix: &Path, path: &Path, result: &mut BTreeMap<String, String>) -> Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            ensure!(
                !entry.file_type()?.is_symlink(),
                "private runtime snapshot contains a symlink"
            );
            if entry.file_type()?.is_dir() {
                walk(prefix, &entry.path(), result)?;
            } else {
                result.insert(
                    path_arg(entry.path().strip_prefix(prefix)?),
                    crate::support::sha256_file(&entry.path())?,
                );
            }
        }
        Ok(())
    }
    let mut result = BTreeMap::new();
    walk(prefix, &prefix.join("config"), &mut result)?;
    walk(prefix, &prefix.join("data"), &mut result)?;
    Ok(result)
}
fn wait_http(
    client: &reqwest::blocking::Client,
    port: u16,
    path: &str,
    credentials: Option<(&str, &str)>,
) -> Result<()> {
    let start = Instant::now();
    loop {
        let mut request = client.get(format!("http://127.0.0.1:{port}{path}"));
        if let Some((password, user)) = credentials {
            request = request.header(
                "Authorization",
                format!("Basic {}", STANDARD.encode(format!("{user}:{password}"))),
            );
        }
        if request.send().is_ok_and(|r| r.status().is_success()) {
            return Ok(());
        }
        ensure!(
            start.elapsed() < Duration::from_secs(45),
            "installed health endpoint did not become ready"
        );
        thread::sleep(Duration::from_millis(200));
    }
}
struct Installation {
    prefix: PathBuf,
    shell: PathBuf,
    environment: Vec<(OsString, OsString)>,
}
impl Installation {
    fn stop(&mut self) {
        let path = self
            .prefix
            .join(if cfg!(windows) { "stop.ps1" } else { "stop.sh" });
        if path.is_file() {
            let mut cmd = script_command(&self.shell, &path, &self.environment);
            let _ = crate::support::process_output(&mut cmd, None, Duration::from_secs(60));
        }
    }
}
impl Drop for Installation {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_public_gate_arguments_fail_before_network() {
        assert!(run(&[
            "--tag".into(),
            "invalid".into(),
            "--commit".into(),
            "a".repeat(40)
        ])
        .is_err());
        assert!(mirrors("https://user:private@example.invalid/").is_err());
        assert!(mirrors("http://127.0.0.1/").is_err());
        assert!(mirrors("https://example.invalid/?private=value").is_err());
    }
    #[test]
    fn public_payload_only_allows_verified_helper_crlf_variation() {
        for name in ["scripts/lib/mirrors.ps1", "scripts/lib/layout.ps1"] {
            assert!(payload_matches(
                name,
                b"fixed\nsource\n",
                b"fixed\r\nsource\r\n"
            ));
            assert!(!payload_matches(
                name,
                b"changed\nsource\n",
                b"fixed\r\nsource\r\n"
            ));
            assert!(!payload_matches(
                name,
                b"fixed\nsource",
                b"fixed\r\nsource\r\n"
            ));
            assert!(!payload_matches(name, b"fixed\r", b"fixed"));
        }
        for name in [
            "bin/vpn233-client.exe",
            "admin-web/dist/index.html",
            "start.ps1",
        ] {
            assert!(!payload_matches(name, b"fixed\n", b"fixed\r\n"));
            assert!(payload_matches(name, b"fixed\n", b"fixed\n"));
        }
    }
    #[test]
    fn customer_environment_keeps_native_runtime_and_excludes_inherited_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let values = environment(temp.path());
        for (key, _) in &values {
            let key = key.to_string_lossy().to_ascii_uppercase();
            assert!(![
                "GH_TOKEN",
                "GITHUB_TOKEN",
                "GH_ENTERPRISE_TOKEN",
                "GITHUB_ENTERPRISE_TOKEN",
                "BASH_ENV",
                "ENV",
                "HTTP_PROXY",
                "HTTPS_PROXY",
                "ALL_PROXY"
            ]
            .contains(&key.as_str()));
            assert!(!key.starts_with("VPN233_") || key == "VPN233_HOME");
        }
        #[cfg(windows)]
        for key in ["OS", "PROCESSOR_ARCHITECTURE", "SYSTEMROOT"] {
            assert!(values.iter().any(|(name, value)| name
                .to_string_lossy()
                .eq_ignore_ascii_case(key)
                && !value.is_empty()));
        }
    }
    #[test]
    fn private_customer_workspace_protects_future_logs() {
        let temp = tempfile::tempdir().unwrap();
        let shell = program(if cfg!(windows) {
            &["powershell", "pwsh"]
        } else {
            &["bash"]
        })
        .unwrap();
        let values = environment(temp.path());
        protect_temporary_directory(temp.path(), &shell, &values).unwrap();
        let logs = temp.path().join("logs");
        fs::create_dir(&logs).unwrap();
        let file = logs.join("fixture.log");
        private_write(&file, b"synthetic fixture only").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(temp.path()).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        #[cfg(windows)]
        {
            let script = "$ErrorActionPreference='Stop';$allowed=@([Security.Principal.WindowsIdentity]::GetCurrent().User.Value,'S-1-5-18','S-1-5-32-544');foreach($item in @($env:VPN233_TEST_PRIVATE_DIR,(Join-Path $env:VPN233_TEST_PRIVATE_DIR 'logs/fixture.log'))){$acl=Get-Acl -LiteralPath $item;if(@($acl.Access|Where-Object {$_.AccessControlType -eq 'Allow' -and $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value -notin $allowed}).Count){exit 1}};if($env:OS -ne 'Windows_NT'){exit 1}";
            let mut command = script_command_inline(&shell, script, &values);
            command.env("VPN233_TEST_PRIVATE_DIR", temp.path());
            assert!(
                crate::support::process_output(&mut command, None, Duration::from_secs(30))
                    .unwrap()
                    .status
                    .success()
            );
        }
    }
}
