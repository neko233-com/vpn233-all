//! Shared installed-customer gates. All private state stays in a temporary
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
#[cfg(windows)]
#[path = "windows_acl.rs"]
mod windows_acl;

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
fn stage<T>(label: &'static str, work: impl FnOnce() -> Result<T>) -> Result<T> {
    let start = Instant::now();
    println!("public-install: {label} running");
    let result = work().map_err(|_| {
        eprintln!(
            "public-install: {label} failed ({:.3}s; private details redacted)",
            start.elapsed().as_secs_f64()
        );
        anyhow!("public customer acceptance failed at controlled stage {label}")
    })?;
    println!(
        "public-install: {label} passed ({:.3}s)",
        start.elapsed().as_secs_f64()
    );
    Ok(result)
}

pub(super) fn run(args: &[String]) -> Result<()> {
    let candidate = args
        .iter()
        .filter(|value| value.as_str() == "--candidate")
        .count();
    ensure!(candidate <= 1, "duplicate candidate mode");
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
            candidate == 1,
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
    protect_temporary_directory(work.path())?;
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
    let (admin_password, ports) = customer_parameters(&mut environment)?;
    let scripts = CustomerScripts {
        root: &root,
        prefix: &prefix,
        shell: &shell,
        environment: &environment,
        logs: &logs,
    };
    let run_script = |label: &'static str, path: &Path, args: &[String], limit: Duration| {
        scripts.run(label, path, args, limit)
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
        processes: None,
    };
    stage("installed-payload-and-versions-private-permissions", || {
        compare_payload(&prefix, &reference_files)?;
        versions(&prefix, &tag, &environment)?;
        permissions(&prefix)
    })?;
    let before = configure_synthetic_state(&prefix)?;
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
            permissions(&prefix)
        },
    )?;
    exercise_installed(
        &scripts,
        &mut cleanup,
        &ports,
        &admin_password,
        &before,
        true,
    )?;
    cleanup.stop();
    println!(
        "{}",
        json!({"mode":"real-anonymous-public-install-and-installed-online-update","tag":tag,"commit":commit,"mirror_mode":mirror,"candidate_mode":candidate == 1,"strict_api_manifest_archive_sha256":true,"binary_versions":6,"config_data_bytes_preserved":true,"health_endpoints":3,"ui_provider_subscription":true,"owned_processes_stopped":3,"installed_application_e2e_steps":14,"private_logs":"temporary user-owned only; never uploaded or printed","seconds":start.elapsed().as_secs_f64()})
    );
    Ok(())
}

pub(super) fn installed_lifecycle(args: &[String]) -> Result<()> {
    let tag = option(args, "--tag")?
        .ok_or_else(|| anyhow!("installed lifecycle requires a fixed tag"))?;
    ensure!(
        Regex::new(r"^v[0-9]+\.[0-9]+\.[0-9]+$")?.is_match(&tag),
        "invalid local validation tag"
    );
    #[cfg(unix)]
    ensure!(
        unsafe { libc::geteuid() } != 0,
        "customer lifecycle must run without root privileges"
    );
    let assets = crate::support::canonical_path(Path::new(
        &option(args, "--assets")?.ok_or_else(|| anyhow!("verified assets directory required"))?,
    ))?;
    let root = crate::support::root()?;
    stage("offline-archive-checksums-and-allowlist", || {
        crate::verify_release::run(&[
            path_arg(&assets),
            tag.trim_start_matches('v').into(),
            "--partial".into(),
        ])
    })?;
    let windows = cfg!(windows);
    let platform = if windows {
        "windows-x86_64"
    } else if cfg!(target_os = "linux") {
        "linux-x86_64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "macos-aarch64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        "macos-x86_64"
    } else {
        bail!("unsupported installed lifecycle platform");
    };
    let archive = assets.join(format!(
        "vpn233-rust-{platform}.{}",
        if windows { "zip" } else { "tar.gz" }
    ));
    let expected = crate::support::sha256_file(&archive)?;
    let reference = archive_contents(&archive, windows)?;
    let work = tempfile::Builder::new()
        .prefix("vpn233-offline-customer-")
        .tempdir()?;
    protect_temporary_directory(work.path())?;
    let shell = program(if windows {
        &["powershell", "pwsh"]
    } else {
        &["bash"]
    })
    .ok_or_else(|| anyhow!("native installer shell unavailable"))?;
    let prefix = work.path().join("installed with spaces");
    let logs = work.path().join("diagnostics");
    fs::create_dir(&logs)?;
    let mut environment = environment(work.path());
    let (admin_password, ports) = customer_parameters(&mut environment)?;
    let scripts = CustomerScripts {
        root: &root,
        prefix: &prefix,
        shell: &shell,
        environment: &environment,
        logs: &logs,
    };
    let mut cleanup = Installation {
        prefix: prefix.clone(),
        shell: shell.clone(),
        environment: environment.clone(),
        processes: None,
    };
    let install = root.join(if windows { "install.ps1" } else { "install.sh" });
    let parameters = if windows {
        vec![
            "-Release".into(),
            "-Version".into(),
            tag.clone(),
            "-Bundle".into(),
            path_arg(&archive),
            "-Sha256".into(),
            expected,
            "-Prefix".into(),
            path_arg(&prefix),
            "-Mirror".into(),
            "off".into(),
            "-NoStart".into(),
        ]
    } else {
        vec![
            "--release".into(),
            "--version".into(),
            tag.clone(),
            "--bundle".into(),
            path_arg(&archive),
            "--sha256".into(),
            expected,
            "--prefix".into(),
            path_arg(&prefix),
            "--mirror".into(),
            "off".into(),
            "--no-start".into(),
            "--yes".into(),
        ]
    };
    stage("offline-archive-actual-install", || {
        scripts.run(
            "offline-install",
            &install,
            &parameters,
            Duration::from_secs(120),
        )
    })?;
    stage("offline-installed-payload-version-and-permissions", || {
        compare_payload(&prefix, &reference)?;
        versions(&prefix, &tag, &environment)?;
        permissions(&prefix)
    })?;
    let before = configure_synthetic_state(&prefix)?;
    exercise_installed(
        &scripts,
        &mut cleanup,
        &ports,
        &admin_password,
        &before,
        false,
    )?;
    cleanup.stop();
    println!(
        "{}",
        json!({"mode":"offline-built-archive-installed-lifecycle","tag":tag,
        "health_endpoints":3,"ui_provider_subscription":true,"owned_processes_stopped":3,
        "online_installation_verified":false,"full_e2e_executed":false})
    );
    Ok(())
}

struct CustomerScripts<'a> {
    root: &'a Path,
    prefix: &'a Path,
    shell: &'a Path,
    environment: &'a [(OsString, OsString)],
    logs: &'a Path,
}
impl CustomerScripts<'_> {
    fn run(
        &self,
        label: &'static str,
        path: &Path,
        args: &[String],
        limit: Duration,
    ) -> Result<()> {
        let mut command = script_command(self.shell, path, self.environment);
        command.args(args).current_dir(self.root);
        let output =
            crate::support::process_output(&mut command, None, limit).map_err(|error| {
                eprintln!(
                    "public-install: {label}-launcher failed (kind={})",
                    crate::support::process_failure_kind(&error)
                );
                anyhow!("customer script execution failed")
            })?;
        private_write(&self.logs.join(format!("{label}.stdout")), &output.stdout)?;
        private_write(&self.logs.join(format!("{label}.stderr")), &output.stderr)?;
        if !output.status.success() {
            eprintln!(
                "public-install: {label}-launcher failed (kind=nonzero; exit={})",
                output.status.code().unwrap_or(-1)
            );
            bail!("customer script returned failure");
        }
        if !cfg!(windows) && (label == "install" || label == "update") {
            ensure!(
                String::from_utf8_lossy(&output.stdout).contains("SHA-256 verified"),
                "customer script did not attest strict archive SHA-256"
            );
        }
        Ok(())
    }
    fn start(&self) -> Result<crate::support::OwnedProcessGroup> {
        let path = self.prefix.join(if cfg!(windows) {
            "start.ps1"
        } else {
            "start.sh"
        });
        let mut command = script_command(self.shell, &path, self.environment);
        command.current_dir(self.root);
        let (_output, processes) = crate::support::process_launcher(
            &mut command,
            &self.logs.join("start.stdout"),
            &self.logs.join("start.stderr"),
            Duration::from_secs(60),
        )
        .map_err(|error| {
            eprintln!(
                "public-install: start-launcher failed (kind={}; exit={})",
                crate::support::process_failure_kind(&error),
                crate::support::process_failure_exit_code(&error).unwrap_or(-1)
            );
            anyhow!("customer launcher failed")
        })?;
        Ok(processes)
    }
}

fn customer_parameters(environment: &mut Vec<(OsString, OsString)>) -> Result<(String, Vec<u16>)> {
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
    Ok((admin_password, ports))
}

fn configure_synthetic_state(prefix: &Path) -> Result<BTreeMap<String, String>> {
    let default_node_ip = Regex::new(r"(?m)^default_node_ip:.*$")?;
    let auto_detect_public_ip = Regex::new(r"(?m)^auto_detect_public_ip:.*$")?;
    let heartbeat = Regex::new(r"(?m)^heartbeat_interval_secs:.*$")?;
    for name in CONFIGS {
        let path = prefix.join("config").join(name);
        let mut text = fs::read_to_string(&path)?;
        if name == "provider-server.yaml" {
            ensure!(
                default_node_ip.find_iter(&text).count() == 1
                    && auto_detect_public_ip.find_iter(&text).count() == 1
                    && heartbeat.find_iter(&text).count() == 1,
                "synthetic provider fields missing or duplicated"
            );
            text = default_node_ip
                .replace(&text, "default_node_ip: '127.0.0.1'")
                .into_owned();
            text = auto_detect_public_ip
                .replace(&text, "auto_detect_public_ip: false")
                .into_owned();
            // The synthetic customer explicitly configures a short registration
            // cycle; production's documented 30-second default stays unchanged.
            text = heartbeat
                .replace(&text, "heartbeat_interval_secs: 1")
                .into_owned();
        }
        text.push_str("\n# synthetic public upgrade preservation sentinel\n");
        fs::write(path, text)?;
    }
    private_write(
        &prefix.join("data/synthetic-preservation.txt"),
        b"synthetic unrelated data must survive actual online update\n",
    )?;
    snapshot(prefix)
}
fn exercise_installed(
    scripts: &CustomerScripts<'_>,
    cleanup: &mut Installation,
    ports: &[u16],
    admin_password: &str,
    before: &BTreeMap<String, String>,
    full_e2e: bool,
) -> Result<()> {
    let root = scripts.root;
    let prefix = scripts.prefix;
    let shell = scripts.shell;
    let environment = scripts.environment;
    let logs = scripts.logs;
    let windows = cfg!(windows);
    let suffix = if windows { ".ps1" } else { ".sh" };
    let run_script = |label: &'static str, path: &Path, args: &[String], limit: Duration| {
        scripts.run(label, path, args, limit)
    };
    cleanup.processes = Some(stage("start-launcher", || scripts.start())?);
    stage("start-health-ui-provider-and-subscription", || {
        let client = stage("loopback-http-client", || {
            Ok(reqwest::blocking::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3))
                .build()?)
        })?;
        stage("provider-health", || {
            wait_http(&client, ports[0], "/api/v1/health", None)
        })?;
        stage("subscribe-health", || {
            wait_http(
                &client,
                ports[1],
                "/api/status",
                Some((admin_password, "synthetic-admin")),
            )
        })?;
        stage("client-health", || {
            wait_http(&client, ports[4], "/api/v1/health", None)
        })?;
        let base = format!("http://127.0.0.1:{}", ports[1]);
        let page = stage("ui-root", || {
            let page = client
                .get(format!("{base}/"))
                .send()?
                .error_for_status()?
                .text()?;
            ensure!(
                page.contains("id=\"root\""),
                "installed UI root unavailable"
            );
            Ok(page)
        })?;
        stage("ui-local-assets", || {
            for resource in
                Regex::new(r#"(?:src|href)="([^" ]+\.(?:js|css))""#)?.captures_iter(&page)
            {
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
            Ok(())
        })?;
        stage("provider-registry", || {
            let begin = Instant::now();
            loop {
                let inventory: Value = client
                    .get(format!("{base}/api/admin/providers"))
                    .basic_auth("synthetic-admin", Some(admin_password))
                    .send()?
                    .error_for_status()?
                    .json()?;
                if inventory["providers"].as_array().is_some_and(|list| {
                    list.iter().any(|p| {
                        p["online"].as_bool() == Some(true)
                            && p["node_count"].as_u64().unwrap_or(0) > 0
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
            Ok(())
        })?;
        let token = format!("synthetic-token-{}", random());
        let password = format!("synthetic-subscription-{}", random());
        stage("token-create", || {
            client
                .post(format!("{base}/api/admin/tokens"))
                .basic_auth("synthetic-admin", Some(admin_password))
                .json(&json!({"token":token,"password":password,"limit_bytes":1000000}))
                .send()?
                .error_for_status()?;
            Ok(())
        })?;
        stage("subscription-render", || {
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
        Ok(())
    })?;
    stage("captured-output-budget", || cleanup.check_capture())?;
    let pids = stage("owned-process-identity", || {
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
            let mut cmd = script_command_inline(shell, script, environment);
            cmd.env("VPN233_TEST_PREFIX", prefix);
            ensure!(
                crate::support::process_output(&mut cmd, None, Duration::from_secs(30))?
                    .status
                    .success(),
                "recorded Windows process is not owned by the temporary installation"
            );
        }
        Ok(pids)
    })?;
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
        for port in ports {
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
            let mut cmd = script_command_inline(shell, "$ids=$env:VPN233_TEST_PIDS -split ','; foreach($id in $ids){if(Get-Process -Id ([int]$id) -ErrorAction SilentlyContinue){exit 1}}", environment);
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
        for (path, expected) in before {
            ensure!(
                crate::support::sha256_file(&prefix.join(path))? == *expected,
                "runtime changed original configuration or unrelated data"
            );
        }
        Ok(())
    })?;
    stage("capture-completion", || cleanup.finish_capture())?;
    if full_e2e {
        stage("installed-binaries-and-installed-ui-full-14-e2e", || {
            let script = root.join(if windows {
                "scripts/e2e/e2e.ps1"
            } else {
                "scripts/e2e/e2e.sh"
            });
            let mut cmd = script_command(shell, &script, environment);
            cmd.current_dir(root)
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
    }
    Ok(())
}

fn metadata(tag: &str, commit: &str, proof: Option<&str>, candidate: bool) -> Result<Value> {
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
    validate_metadata(&data, tag, commit, candidate)?;
    Ok(data)
}

fn validate_metadata(data: &Value, tag: &str, commit: &str, candidate: bool) -> Result<()> {
    ensure!(
        data["repository"]["full_name"] == REPO
            && data["repository"]["private"] == false
            && data["repository"]["fork"] == false,
        "repository must be independently public"
    );
    ensure!(
        data["release"]["tag_name"] == tag
            && data["release"]["draft"] == false
            && data["release"]["prerelease"] == candidate
            && data["release"]["published_at"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
            && data["tag_commit"] == commit,
        "release does not match its explicit candidate or stable publication mode"
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
        assets.len() == names.len()
            && names.iter().all(|name| assets
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
    Ok(())
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
                "deploy.sh",
                "deploy.ps1",
                "uninstall.sh",
                "uninstall.ps1",
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
                    !installed_payload_required(name, cfg!(windows)),
                    "installed application file missing"
                );
            }
        }
    }
    Ok(())
}
fn installed_payload_required(name: &str, windows: bool) -> bool {
    if name.starts_with("bin/") || name.starts_with("admin-web/dist/") || name == "version.txt" {
        return true;
    }
    let suffix = if windows { ".ps1" } else { ".sh" };
    if ["install", "deploy", "update", "uninstall", "start", "stop"]
        .iter()
        .any(|script| name == format!("{script}{suffix}"))
    {
        return true;
    }
    if windows {
        matches!(name, "scripts/lib/mirrors.ps1" | "scripts/lib/layout.ps1")
    } else {
        matches!(
            name,
            "scripts/lib/mirrors.sh" | "scripts/lib/layout.sh" | "scripts/lib/services.sh"
        )
    }
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
fn protect_temporary_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    {
        windows_acl::protect_directory(path)?;
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
fn permissions(prefix: &Path) -> Result<()> {
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
    }
    #[cfg(windows)]
    {
        for name in ["config", "data", "logs", "backups"] {
            let path = prefix.join(name);
            if name == "backups" && !path.exists() {
                continue;
            }
            windows_acl::verify(&path, true)?;
        }
        for name in CONFIGS {
            windows_acl::verify(&prefix.join("config").join(name), false)?;
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
    processes: Option<crate::support::OwnedProcessGroup>,
}
impl Installation {
    fn finish_capture(&mut self) -> Result<()> {
        self.processes
            .take()
            .ok_or_else(|| anyhow!("missing owned launcher"))?
            .finish()
    }
    fn check_capture(&self) -> Result<()> {
        if let Some(processes) = &self.processes {
            processes.check_health()?;
        }
        Ok(())
    }
    fn stop(&mut self) {
        let path = self
            .prefix
            .join(if cfg!(windows) { "stop.ps1" } else { "stop.sh" });
        if path.is_file() {
            let mut cmd = script_command(&self.shell, &path, &self.environment);
            let _ = crate::support::process_output(&mut cmd, None, Duration::from_secs(60));
        }
        // Never let a failed stop script detach the acceptance-owned services.
        self.processes.take();
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
    fn installed_payload_requires_every_native_operator_and_helper() {
        for windows in [false, true] {
            for script in ["install", "deploy", "update", "uninstall", "start", "stop"] {
                assert!(installed_payload_required(
                    &format!("{script}.{}", if windows { "ps1" } else { "sh" }),
                    windows
                ));
            }
            for name in [
                "bin/vpn233-client",
                "admin-web/dist/index.html",
                "version.txt",
            ] {
                assert!(installed_payload_required(name, windows));
            }
            assert!(!installed_payload_required("docs/index.md", windows));
        }
        let fixture = tempfile::tempdir().unwrap();
        let script = if cfg!(windows) {
            "uninstall.ps1"
        } else {
            "uninstall.sh"
        };
        let files = BTreeMap::from([(script.to_owned(), b"synthetic public script".to_vec())]);
        assert!(compare_payload(fixture.path(), &files).is_err());
        fs::write(fixture.path().join(script), b"synthetic public script").unwrap();
        assert!(compare_payload(fixture.path(), &files).is_ok());
    }
    fn release_metadata(candidate: bool) -> Value {
        let mut assets = vec![
            json!({"name":"SHA256SUMS","size":1,"digest":format!("sha256:{}", "a".repeat(64))}),
            json!({"name":"version.txt","size":1,"digest":format!("sha256:{}", "b".repeat(64))}),
        ];
        for platform in [
            "windows-x86_64",
            "linux-x86_64",
            "macos-x86_64",
            "macos-aarch64",
        ] {
            for extension in ["tar.gz", "zip"] {
                assets.push(json!({"name":format!("vpn233-rust-{platform}.{extension}"),"size":1,"digest":format!("sha256:{}", "c".repeat(64))}));
            }
        }
        json!({"repository":{"full_name":REPO,"private":false,"fork":false},
            "tag_commit":"d".repeat(40),"release":{"tag_name":"v2.1.1","draft":false,
                "prerelease":candidate,"published_at":"2026-10-22T00:00:00Z","assets":assets}})
    }
    #[test]
    fn candidate_customer_mode_must_be_explicit_and_keeps_all_metadata_checks() {
        let commit = "d".repeat(40);
        for candidate in [false, true] {
            let valid = release_metadata(candidate);
            assert!(validate_metadata(&valid, "v2.1.1", &commit, candidate).is_ok());
            assert!(validate_metadata(&valid, "v2.1.1", &commit, !candidate).is_err());
            for pointer in ["/release/draft", "/repository/private", "/repository/fork"] {
                let mut invalid = valid.clone();
                *invalid.pointer_mut(pointer).unwrap() = json!(true);
                assert!(validate_metadata(&invalid, "v2.1.1", &commit, candidate).is_err());
            }
            let mut extra = valid.clone();
            extra["release"]["assets"]
                .as_array_mut()
                .unwrap()
                .push(json!({"name":"unexpected-private.log"}));
            assert!(validate_metadata(&extra, "v2.1.1", &commit, candidate).is_err());
            let mut missing = valid.clone();
            missing["release"]["assets"].as_array_mut().unwrap().pop();
            assert!(validate_metadata(&missing, "v2.1.1", &commit, candidate).is_err());
            let mut digest = valid.clone();
            digest["release"]["assets"][0]["digest"] = json!("untrusted");
            assert!(validate_metadata(&digest, "v2.1.1", &commit, candidate).is_err());
            assert!(validate_metadata(&valid, "v2.1.1", &"e".repeat(40), candidate).is_err());
        }
    }
    #[test]
    fn controlled_stage_never_includes_the_private_error_chain() {
        let error = stage("synthetic-private-error", || Err::<(), _>(anyhow!(
            "synthetic-secret https://example.invalid/private?token=synthetic-secret C:/synthetic-private/path"
        ))).unwrap_err().to_string();
        assert!(error.contains("synthetic-private-error"));
        for forbidden in ["synthetic-secret", "https://", "token=", "C:/"] {
            assert!(!error.contains(forbidden));
        }
    }
    #[test]
    fn synthetic_customer_config_is_explicit_and_accepts_crlf_templates() {
        let work = tempfile::tempdir().unwrap();
        fs::create_dir(work.path().join("config")).unwrap();
        fs::create_dir(work.path().join("data")).unwrap();
        for name in CONFIGS {
            let text = if name == "provider-server.yaml" {
                "default_node_ip: 'auto'\r\nauto_detect_public_ip: true\r\nheartbeat_interval_secs: 30\r\n"
            } else {
                "# synthetic config preserved\r\n"
            };
            fs::write(work.path().join("config").join(name), text).unwrap();
        }
        let before = configure_synthetic_state(work.path()).unwrap();
        let provider = fs::read_to_string(work.path().join("config/provider-server.yaml")).unwrap();
        assert!(provider.contains("default_node_ip: '127.0.0.1'\n"));
        assert!(provider.contains("auto_detect_public_ip: false\n"));
        assert!(provider.contains("heartbeat_interval_secs: 1\n"));
        assert_eq!(before.len(), 4);
        assert_eq!(snapshot(work.path()).unwrap(), before);
    }
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
        protect_temporary_directory(temp.path()).unwrap();
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
            windows_acl::verify(temp.path(), true).unwrap();
            windows_acl::verify(&logs, false).unwrap();
            windows_acl::verify(&file, false).unwrap();
            assert_eq!(fs::read(file).unwrap(), b"synthetic fixture only");
        }
    }
}
