use super::{clean_command, option, path_arg, private_write, ChildGuard};
use anyhow::{anyhow, bail, ensure, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    env, fs,
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

const PROTOCOLS: [&str; 8] = [
    "vless",
    "vmess",
    "trojan",
    "ss",
    "hysteria2",
    "tuic",
    "anytls",
    "socks5",
];

pub(super) fn pins(root: &Path) -> Result<(String, BTreeMap<String, String>)> {
    let config = fs::read_to_string(root.join("provider-server/src/config.rs"))?;
    let version = Regex::new(r#"DEFAULT_MIHOMO_VERSION: &str = "([^"]+)""#)?
        .captures(&config)
        .ok_or_else(|| anyhow!("pinned Mihomo version missing"))?[1]
        .to_owned();
    let source = fs::read_to_string(root.join("provider-server/src/scripts.rs"))?;
    let matcher = Regex::new(r#""(mihomo-[^"]+)"\s*,\s*"([a-f0-9]{64})""#)?;
    let mut map = BTreeMap::new();
    for capture in matcher.captures_iter(&source) {
        ensure!(
            map.insert(capture[1].to_owned(), capture[2].to_owned())
                .is_none(),
            "duplicate official Mihomo digest"
        );
    }
    ensure!(
        map.len() == 6 && map.keys().all(|asset| asset.contains(&version)),
        "official six-platform Mihomo digest matrix differs"
    );
    Ok((version, map))
}

pub(super) fn run(args: &[String]) -> Result<()> {
    let root = crate::support::root()?;
    let provider = crate::support::canonical_path(&PathBuf::from(
        option(args, "--provider-bin")?.unwrap_or_else(|| {
            path_arg(&root.join("target/release").join(if cfg!(windows) {
                "vpn233-provider-server.exe"
            } else {
                "vpn233-provider-server"
            }))
        }),
    ))?;
    let archive = option(args, "--mihomo-archive")?;
    let binary = option(args, "--mihomo-bin")?;
    let download = args.iter().any(|s| s == "--download-core");
    ensure!(
        usize::from(archive.is_some()) + usize::from(binary.is_some()) + usize::from(download) == 1,
        "select exactly one trusted Mihomo source"
    );
    let work = tempfile::Builder::new()
        .prefix("vpn233-mihomo-parser-")
        .tempdir()?;
    tokio::runtime::Runtime::new()?.block_on(async {
        let core = if let Some(binary) = binary { crate::support::canonical_path(Path::new(&binary))? } else {
            let (version, digests) = pins(&root)?;
            let os = match env::consts::OS { "windows" => "windows", "linux" => "linux", "macos" => "darwin", _ => bail!("no pinned Mihomo operating system") };
            let arch = match env::consts::ARCH { "x86_64" => "amd64-compatible", "aarch64" => "arm64", _ => bail!("no pinned Mihomo architecture") };
            let asset = format!("mihomo-{os}-{arch}-{version}.{}", if cfg!(windows) { "zip" } else { "gz" });
            let expected = digests.get(&asset).ok_or_else(|| anyhow!("official Mihomo digest missing for this platform"))?;
            let source = if let Some(archive) = archive { PathBuf::from(archive) } else {
                let default_cache = env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).unwrap_or_else(|| ".".into())).join(".cache")).join("vpn233/mihomo");
                let cache = option(args, "--cache-dir")?.map(PathBuf::from).unwrap_or(default_cache);
                fs::create_dir_all(&cache)?;
                let path = cache.join(&asset);
                if !path.exists() {
                    let partial = tempfile::NamedTempFile::new_in(&cache)?;
                    download_official(&format!("https://github.com/MetaCubeX/mihomo/releases/download/{version}/{asset}"), partial.path()).await?;
                    ensure!(crate::support::sha256_file(partial.path())? == *expected, "downloaded official Mihomo archive SHA-256 mismatch");
                    partial.persist(&path).map_err(|_| anyhow!("official archive cache replacement failed"))?;
                }
                path
            };
            ensure!(crate::support::sha256_file(&source)? == *expected, "official Mihomo archive SHA-256 mismatch; execution refused");
            let target = work.path().join(if cfg!(windows) { "mihomo.exe" } else { "mihomo" });
            if asset.ends_with(".zip") {
                let mut package = zip::ZipArchive::new(fs::File::open(&source)?)?;
                let mut executables = Vec::new();
                for index in 0..package.len() { let entry = package.by_index(index)?; if !entry.is_dir() && entry.name()?.ends_with(".exe") { executables.push(index); } }
                ensure!(executables.len() == 1, "official archive must contain one executable");
                let mut bytes = Vec::new(); package.by_index(executables[0])?.take(512 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= 512 * 1024 * 1024, "official binary extraction exceeds bound"); private_write(&target, &bytes)?;
            } else {
                let mut bytes = Vec::new(); flate2::read::GzDecoder::new(fs::File::open(&source)?).take(512 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= 512 * 1024 * 1024, "official binary extraction exceeds bound"); private_write(&target, &bytes)?;
            }
            #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; fs::set_permissions(&target, fs::Permissions::from_mode(0o700))?; }
            println!("Official archive SHA-256 verified: {asset}"); target
        };
        ensure!(core.is_file(), "trusted Mihomo binary is not a file");
        generated(&provider, &core, work.path()).await?;
        let mut total = 9;
        if let Some(directory) = option(args, "--core-fixtures-dir")? {
            let mut fixtures = fs::read_dir(directory)?.map(|entry| entry.map(|e| e.path())).collect::<std::io::Result<Vec<_>>>()?;
            fixtures.retain(|path| path.extension().is_some_and(|ext| ext == "yaml") && path.is_file()); fixtures.sort();
            ensure!(!fixtures.is_empty(), "core fixture directory contains no YAML files");
            for (index, fixture) in fixtures.iter().enumerate() { parser(&core, fixture, work.path(), &format!("core-case-{}", index + 1))?; total += 1; }
        }
        println!("Official parser: {total} real Mihomo -t checks passed; no proxy listeners started.");
        Ok(())
    })
}

async fn download_official(url: &str, destination: &Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(90))
        .build()?;
    for _ in 0..2 {
        let offset = fs::metadata(destination)?.len();
        let mut request = client
            .get(url)
            .header("Accept-Encoding", "identity")
            .header("User-Agent", "vpn233-public-parser-check");
        if offset > 0 {
            request = request.header("Range", format!("bytes={offset}-"));
        }
        let Ok(mut response) = request.send().await else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        if offset > 0 {
            let range = response
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            ensure!(
                response.status() == reqwest::StatusCode::PARTIAL_CONTENT
                    && range.starts_with(&format!("bytes {offset}-")),
                "official download server refused safe resumption"
            );
        }
        let mut output = fs::OpenOptions::new().append(true).open(destination)?;
        let mut complete = true;
        loop {
            match response.chunk().await {
                Ok(Some(bytes)) => {
                    ensure!(
                        output.metadata()?.len() + bytes.len() as u64 <= 100 * 1024 * 1024,
                        "official archive exceeds bound"
                    );
                    output.write_all(&bytes)?;
                }
                Ok(None) => break,
                Err(_) => {
                    complete = false;
                    break;
                }
            }
        }
        if complete {
            return Ok(());
        }
    }
    bail!("official Mihomo download failed after two bounded TLS-verified attempts")
}

fn parser(core: &Path, config: &Path, work: &Path, name: &str) -> Result<()> {
    let mut command = clean_command(core);
    command
        .args(["-t", "-d"])
        .arg(crate::support::normalize_path(work))
        .arg("-f")
        .arg(crate::support::normalize_path(config));
    let output = crate::support::process_output(&mut command, None, Duration::from_secs(30))
        .map_err(|_| anyhow!("official parser process failed or exceeded its deadline"))?;
    ensure!(
        output.status.success(),
        "official Mihomo rejected controlled case {name}; parser output remains private"
    );
    println!("PASS official Mihomo -t: {name}");
    Ok(())
}

async fn json_request(
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
    payload: Option<Value>,
) -> Result<Value> {
    let mut request = if let Some(payload) = payload {
        client.post(url).json(&payload)
    } else {
        client.get(url)
    };
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request
        .send()
        .await
        .map_err(|_| anyhow!("synthetic provider request failed"))?
        .error_for_status()
        .map_err(|_| anyhow!("synthetic provider response status failed"))?
        .json()
        .await
        .map_err(|_| anyhow!("synthetic provider returned invalid JSON"))
}

async fn generated(provider: &Path, core: &Path, work: &Path) -> Result<()> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let secret = format!("synthetic-parser-{}", rand_id());
    let config = work.join("provider-fixture.yaml");
    private_write(&config, format!("listen_addr: \"127.0.0.1\"\nlisten_port: {port}\nadmin_user: \"parser-probe\"\nadmin_password: \"{secret}\"\ndefault_node_ip: \"203.0.113.10\"\ndefault_port_base: 22000\nauto_detect_public_ip: false\nregistry_url: \"\"\nprovider_access_token: \"\"\n").as_bytes())?;
    let log_path = work.join("provider-synthetic.log");
    private_write(&log_path, b"")?;
    let log = fs::OpenOptions::new().append(true).open(log_path)?;
    let mut command = clean_command(provider);
    command
        .arg("--config")
        .arg(crate::support::normalize_path(&config))
        .current_dir(work)
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    let mut process = ChildGuard(
        command
            .spawn()
            .map_err(|_| anyhow!("synthetic provider could not start"))?,
    );
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?;
    let base = format!("http://127.0.0.1:{port}");
    let started = Instant::now();
    loop {
        ensure!(
            process.0.try_wait()?.is_none(),
            "synthetic provider exited before readiness"
        );
        if json_request(&client, &format!("{base}/api/v1/health"), None, None)
            .await
            .is_ok()
        {
            break;
        }
        ensure!(
            started.elapsed() < Duration::from_secs(6),
            "synthetic provider readiness timed out"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let login = json_request(
        &client,
        &format!("{base}/api/v1/login"),
        None,
        Some(json!({"username":"parser-probe","password":secret})),
    )
    .await?;
    let token = login["token"]
        .as_str()
        .ok_or_else(|| anyhow!("synthetic login token missing"))?;
    let generated = json_request(&client, &format!("{base}/api/v1/generate?node_name=parser-probe&node_ip=203.0.113.10&port_base=22000&uuid=123e4567-e89b-42d3-a456-426614174000&password=synthetic-inbound-parser-password"), Some(token), None).await?;
    let actual: Vec<_> = generated["node"]["ports"]
        .as_array()
        .ok_or_else(|| anyhow!("generated inbound inventory missing"))?
        .iter()
        .map(|entry| entry["protocol"].as_str().unwrap_or(""))
        .collect();
    ensure!(
        actual == PROTOCOLS,
        "provider did not generate all eight expected inbound protocols"
    );
    let script = generated["sh"]
        .as_str()
        .ok_or_else(|| anyhow!("generated SH script missing"))?;
    let capture = Regex::new(
        r"(?ms)^render_config\(\) \{\r?\n  cat <<VPN233_YAML\r?\n(.*?)\r?\nVPN233_YAML\r?\n\}",
    )?
    .captures(script)
    .ok_or_else(|| anyhow!("generated configuration heredoc missing"))?;
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["example.invalid".to_owned()])?;
    let certificate = work.join("synthetic-certificate.pem");
    let key = work.join("synthetic-private-key.pem");
    private_write(&certificate, cert.pem().as_bytes())?;
    private_write(&key, signing_key.serialize_pem().as_bytes())?;
    let yaml = capture[1]
        .replace("$TLS_CERT", &path_arg(&certificate))
        .replace("$TLS_KEY", &path_arg(&key));
    let (header, listeners) = yaml
        .split_once("listeners:\n")
        .ok_or_else(|| anyhow!("generated listeners missing"))?;
    let header = format!("{header}listeners:\n");
    let mut blocks = Vec::<String>::new();
    for line in listeners.lines() {
        if line.starts_with("  - name:") {
            blocks.push(String::new());
        }
        if let Some(block) = blocks.last_mut() {
            block.push_str(line);
            block.push('\n');
        }
    }
    ensure!(blocks.len() == 8, "generated listener block count differs");
    let combined = work.join("all-eight.yaml");
    private_write(&combined, yaml.as_bytes())?;
    parser(core, &combined, work, "all-eight")?;
    for (protocol, block) in PROTOCOLS.iter().zip(blocks) {
        let path = work.join(format!("{protocol}.yaml"));
        private_write(&path, format!("{header}{block}").as_bytes())?;
        parser(core, &path, work, protocol)?;
    }
    Ok(())
}

fn rand_id() -> String {
    use rand_core::{OsRng, RngCore};
    let mut bytes = [0; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn six_platform_pins_match_provider() {
        let root = crate::support::root().unwrap();
        assert_eq!(pins(&root).unwrap().1.len(), 6);
    }
}
