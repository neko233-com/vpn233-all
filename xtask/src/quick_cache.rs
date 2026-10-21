//! Content-addressed local Quick reports. Full/CI never consume these reports.
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

use crate::support::{canonical_path, normalize_path, process_output, sha256_file};

const SCHEMA: u64 = 2;
type Environment = BTreeMap<String, String>;

fn environment_values(
    values: impl Iterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Result<Environment> {
    values
        .map(|(key, value)| {
            Ok((
                key.into_string()
                    .map_err(|_| anyhow!("unsupported-environment-encoding"))?,
                value
                    .into_string()
                    .map_err(|_| anyhow!("unsupported-environment-encoding"))?,
            ))
        })
        .collect()
}

fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}
fn canonical(value: &Value) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(value)?)
}
fn checked_output(root: &Path, program: &str, arguments: &[&str]) -> Result<Vec<u8>> {
    let output = process_output(
        Command::new(program).args(arguments).current_dir(root),
        None,
        Duration::from_secs(20),
    )?;
    if !output.status.success() {
        bail!("native-command-unavailable");
    }
    Ok(output.stdout)
}
fn git(root: &Path, arguments: &[&str]) -> Result<Vec<u8>> {
    checked_output(root, "git", arguments)
}
fn mode(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        if metadata.is_dir() {
            if metadata.permissions().readonly() {
                0o555
            } else {
                0o777
            }
        } else if metadata.permissions().readonly() {
            0o444
        } else {
            0o666
        }
    }
}
fn path_text(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| anyhow!("unsupported-path-encoding"))?;
    Ok(if cfg!(windows) {
        value.replace('\\', "/")
    } else {
        value.into()
    })
}
fn hash_field(hash: &mut Sha256, field: impl AsRef<[u8]>) {
    hash.update(field.as_ref());
    hash.update([0]);
}

fn tree_digest(root: &Path) -> Result<String> {
    if !root.is_dir() {
        return Ok("missing".into());
    }
    let resolved = canonical_path(root)?;
    let mut hash = Sha256::new();
    let mut queue = vec![root.to_owned()];
    while let Some(folder) = queue.pop() {
        let mut entries = fs::read_dir(folder)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            hash_field(&mut hash, mode(&metadata).to_string());
            let name = path_text(path.strip_prefix(root)?)?;
            if metadata.file_type().is_symlink() {
                if !canonical_path(&path)?.starts_with(&resolved) {
                    bail!("dependency-link-outside-tree");
                }
                hash_field(&mut hash, "link");
                hash_field(&mut hash, name);
                hash_field(&mut hash, path_text(&fs::read_link(path)?)?);
            } else if metadata.is_dir() {
                hash_field(&mut hash, "directory");
                hash_field(&mut hash, name);
                queue.push(path);
            } else if metadata.is_file() {
                hash_field(&mut hash, "file");
                hash_field(&mut hash, name);
                hash_field(&mut hash, sha256_file(&path)?);
            } else {
                bail!("unsupported-dependency-file");
            }
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn find_program(name: &str) -> Result<PathBuf> {
    let path = Path::new(name);
    if path.components().count() > 1 && path.is_file() {
        return Ok(normalize_path(path));
    }
    let extensions: Vec<String> = if cfg!(windows) {
        env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .map(str::to_owned)
            .collect()
    } else {
        vec![String::new()]
    };
    for directory in env::split_paths(&env::var_os("PATH").unwrap_or_default()) {
        let direct = directory.join(name);
        if direct.is_file()
            && (!cfg!(windows)
                || extensions.iter().any(|extension| {
                    name.to_ascii_uppercase()
                        .ends_with(&extension.to_ascii_uppercase())
                }))
        {
            return Ok(direct);
        }
        for extension in &extensions {
            let candidate = directory.join(format!("{name}{extension}"));
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    bail!("required-tool-unavailable")
}
fn bash_path() -> Result<PathBuf> {
    if cfg!(windows) {
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(directory) = env::var_os(variable) {
                let path = PathBuf::from(directory).join("Git/bin/bash.exe");
                if path.is_file() {
                    return Ok(path);
                }
            }
        }
    }
    find_program("bash")
}
fn tool_evidence(root: &Path, flavor: &str, engine: &str) -> Result<Value> {
    let shell = if flavor == "powershell" {
        find_program(engine)?
    } else {
        find_program("pwsh")?
    };
    let module = "$m=Import-Module PSScriptAnalyzer -PassThru -ErrorAction Stop; [pscustomobject]@{version=$PSVersionTable.PSVersion.ToString();module_version=$m.Version.ToString();module_base=$m.ModuleBase}|ConvertTo-Json -Compress";
    let mut commands: Vec<(String, Vec<String>)> = [
        ("git", vec!["git", "--version"]),
        ("cargo", vec!["cargo", "--version"]),
        ("rustc", vec!["rustc", "-vV"]),
        ("rustfmt", vec!["rustfmt", "--version"]),
        ("clippy", vec!["cargo", "clippy", "--version"]),
        ("node", vec!["node", "--version"]),
        ("npm", vec!["npm", "--version"]),
        ("shellcheck", vec!["shellcheck", "--version"]),
        ("actionlint", vec!["actionlint", "-version"]),
    ]
    .into_iter()
    .map(|(name, command)| {
        (
            name.into(),
            command.into_iter().map(str::to_owned).collect(),
        )
    })
    .collect();
    commands.push((
        "bash".into(),
        vec![
            bash_path()?.to_string_lossy().into_owned(),
            "--version".into(),
        ],
    ));
    commands.push((
        "psmodule".into(),
        vec![
            shell.to_string_lossy().into_owned(),
            "-NoProfile".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-Command".into(),
            module.into(),
        ],
    ));
    if flavor == "powershell" {
        commands.push((
            "powershell-fixtures".into(),
            vec![
                "powershell".into(),
                "-NoProfile".into(),
                "-Command".into(),
                "$PSVersionTable.PSVersion.ToString()".into(),
            ],
        ));
    }
    if cfg!(target_os = "linux") {
        commands.push((
            "systemd-analyze".into(),
            vec!["systemd-analyze".into(), "--version".into()],
        ));
    }
    let mut evidence = serde_json::Map::new();
    for batch in commands.chunks(8) {
        let results = std::thread::scope(|scope| {
            let workers: Vec<_> = batch
                .iter()
                .map(|(name, command)| {
                    scope.spawn(move || -> Result<(String, Value)> {
                        let executable = find_program(&command[0])?;
                        let arguments: Vec<_> = command[1..].iter().map(String::as_str).collect();
                        let output = checked_output(
                            root,
                            executable
                                .to_str()
                                .ok_or_else(|| anyhow!("unsupported-tool-path"))?,
                            &arguments,
                        )?;
                        let mut files = BTreeMap::new();
                        files.insert(path_text(&executable)?, sha256_file(&executable)?);
                        for suffix in ["exe", "cmd", "ps1", "bat"] {
                            let sibling = executable.with_extension(suffix);
                            if sibling.is_file() {
                                files.insert(path_text(&sibling)?, sha256_file(&sibling)?);
                            }
                        }
                        let mut value = json!({"files":files,"output":digest(&output)});
                        if name == "psmodule" {
                            let text = std::str::from_utf8(&output)?.trim_start_matches('\u{feff}');
                            let metadata: Value = serde_json::from_str(text)?;
                            value["module"] = tree_digest(Path::new(
                                metadata["module_base"]
                                    .as_str()
                                    .ok_or_else(|| anyhow!("module-metadata-unavailable"))?,
                            ))?
                            .into();
                        }
                        Ok((name.clone(), value))
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().map_err(|_| anyhow!("tool-worker-failed"))?)
                .collect::<Result<Vec<_>>>()
        })?;
        for (name, value) in results {
            evidence.insert(name, value);
        }
    }
    let executable = canonical_path(&env::current_exe()?)?;
    evidence.insert("xtask".into(), json!({"binary_sha256":sha256_file(&executable)?,"path":path_text(&executable)?,"schema":SCHEMA}));
    Ok(evidence.into())
}

fn scoped_environment(environment: &Environment) -> Environment {
    const KEYS: &[&str] = &[
        "PATH",
        "PATHEXT",
        "PSMODULEPATH",
        "LANG",
        "LC_ALL",
        "TZ",
        "SOURCE_DATE_EPOCH",
        "LIB",
        "INCLUDE",
        "CL",
        "LINK",
        "CC",
        "CXX",
        "AR",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "CFLAGS",
        "CXXFLAGS",
        "CPPFLAGS",
        "LDFLAGS",
        "LIBPATH",
        "EXTERNAL_INCLUDE",
    ];
    const PREFIXES: &[&str] = &[
        "NPM_",
        "CARGO_",
        "RUST",
        "NODE_",
        "GIT_CONFIG_",
        "VITE_",
        "VPN233_",
        "CC_",
        "CXX_",
        "AR_",
        "CFLAGS_",
        "CXXFLAGS_",
        "CPPFLAGS_",
        "LDFLAGS_",
        "BASH_",
        "MSYS",
    ];
    environment
        .iter()
        .filter(|(key, _)| {
            let upper = key.to_ascii_uppercase();
            KEYS.contains(&upper.as_str())
                || PREFIXES.iter().any(|prefix| upper.starts_with(prefix))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
fn environment_disabled(environment: &Environment) -> bool {
    let values: BTreeMap<_, _> = environment
        .iter()
        .map(|(k, v)| (k.to_ascii_uppercase(), v.to_ascii_lowercase()))
        .collect();
    !matches!(
        values.get("CI").map(String::as_str).unwrap_or(""),
        "" | "0" | "false"
    ) || values.get("GITHUB_ACTIONS").is_some_and(|v| !v.is_empty())
        || [
            "BASH_ENV",
            "NODE_OPTIONS",
            "RUSTC",
            "RUSTC_WRAPPER",
            "RUSTC_WORKSPACE_WRAPPER",
        ]
        .iter()
        .any(|key| values.get(*key).is_some_and(|v| !v.is_empty()))
}
fn home_directory(environment: &Environment) -> Option<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    environment
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| PathBuf::from(v))
        .or_else(|| env::var_os(key).map(PathBuf::from))
}
fn source_fingerprint(
    root: &Path,
    flavor: &str,
    engine: &str,
    evidence: &Value,
    environment: &Environment,
) -> Result<String> {
    let root = canonical_path(root)?;
    let raw = git(
        &root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?;
    let candidates: BTreeSet<_> = raw.split(|b| *b == 0).filter(|v| !v.is_empty()).collect();
    let mut source = Sha256::new();
    for raw in candidates {
        let name = std::str::from_utf8(raw).map_err(|_| anyhow!("unsupported-source-path"))?;
        let path = root.join(name);
        hash_field(&mut source, raw);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let target = canonical_path(&path)?;
                if !target.starts_with(&root) {
                    bail!("source-link-outside-repository");
                }
                hash_field(&mut source, "link");
                hash_field(&mut source, path_text(&fs::read_link(path)?)?);
                if target.is_file() {
                    hash_field(&mut source, sha256_file(&target)?);
                }
            }
            Ok(metadata) if metadata.is_file() => {
                hash_field(&mut source, mode(&metadata).to_string());
                hash_field(&mut source, sha256_file(&path)?);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hash_field(&mut source, "deleted")
            }
            _ => bail!("unsupported-source-file"),
        }
    }
    for args in [
        &["ls-files", "--stage", "-z"][..],
        &["rev-list", "--all", "--objects"],
        &["show-ref", "--head"],
        &["config", "--list", "--show-origin"],
    ] {
        hash_field(&mut source, git(&root, args)?);
    }
    let mut configuration = Vec::new();
    let admin = root.join("admin-web");
    if admin.is_dir() {
        let mut files = fs::read_dir(&admin)?.collect::<std::io::Result<Vec<_>>>()?;
        files.sort_by_key(|entry| entry.file_name());
        for entry in files {
            if entry.file_name().to_string_lossy().starts_with(".env") && entry.path().is_file() {
                configuration.push(entry.path());
            }
        }
    }
    configuration.push(admin.join(".npmrc"));
    configuration.extend(
        environment
            .iter()
            .filter(|(k, v)| {
                matches!(
                    k.to_ascii_uppercase().as_str(),
                    "NPM_CONFIG_USERCONFIG" | "NPM_CONFIG_GLOBALCONFIG"
                ) && !v.is_empty()
            })
            .map(|(_, v)| PathBuf::from(v)),
    );
    let home = home_directory(environment);
    let cargo = environment
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("CARGO_HOME"))
        .map(|(_, v)| PathBuf::from(v))
        .or_else(|| home.as_ref().map(|p| p.join(".cargo")));
    let mut folders: Vec<_> = root.ancestors().map(Path::to_path_buf).collect();
    folders.extend(home.iter().cloned());
    folders.extend(cargo.iter().cloned());
    for folder in folders {
        configuration.extend([
            folder.join(".cargo/config"),
            folder.join(".cargo/config.toml"),
        ]);
        if folder == root || home.as_ref() == Some(&folder) {
            configuration.push(folder.join(".npmrc"));
        }
        if cargo.as_ref() == Some(&folder) {
            configuration.extend([folder.join("config"), folder.join("config.toml")]);
        }
    }
    let mut configs = Vec::new();
    for path in configuration {
        if path.is_file() {
            configs.push((path_text(&path)?, sha256_file(&path)?));
        }
    }
    let context = json!({"schema":SCHEMA,"flavor":flavor,"engine":engine,"system":[env::consts::OS,env::consts::ARCH],"environment":scoped_environment(environment),"configuration":configs,"tools":evidence});
    hash_field(&mut source, canonical(&context)?);
    Ok(format!("{:x}", source.finalize()))
}
fn marker_valid(root: &Path) -> Result<bool> {
    let lock = root.join("admin-web/package-lock.json");
    let marker = root.join("admin-web/node_modules/.vpn233-lock.sha256");
    Ok(lock.is_file()
        && marker.is_file()
        && fs::read_to_string(marker)?
            .trim()
            .eq_ignore_ascii_case(&sha256_file(&lock)?))
}
fn fingerprints_with_hook(
    root: &Path,
    flavor: &str,
    engine: &str,
    evidence: Option<&Value>,
    environment: &Environment,
    after_source: Option<&dyn Fn()>,
) -> Result<Value> {
    std::thread::scope(|scope| {
        let dependencies = scope.spawn(|| tree_digest(&root.join("admin-web/node_modules")));
        let output = scope.spawn(|| tree_digest(&root.join("admin-web/dist")));
        let evidence = match evidence {
            Some(value) => value.clone(),
            None => tool_evidence(root, flavor, engine)?,
        };
        let source = source_fingerprint(root, flavor, engine, &evidence, environment)?;
        if let Some(hook) = after_source {
            hook();
        }
        let dependencies = dependencies
            .join()
            .map_err(|_| anyhow!("dependency-worker-failed"))??;
        let output = output
            .join()
            .map_err(|_| anyhow!("output-worker-failed"))??;
        let ready = marker_valid(root)?;
        if source != source_fingerprint(root, flavor, engine, &evidence, environment)? {
            bail!("source-changed-during-fingerprint");
        }
        let fingerprint = digest(canonical(
            &json!({"source":source,"dependencies":dependencies,"output":output}),
        )?);
        Ok(
            json!({"source":source,"dependencies":dependencies,"output":output,"dependencies_were_ready":ready,"fingerprint":fingerprint}),
        )
    })
}
fn cache_directory(root: &Path) -> Result<PathBuf> {
    let root = canonical_path(root)?;
    let directory = root.join(".cache/quick-verify");
    let existing = directory
        .ancestors()
        .find(|p| p.exists())
        .ok_or_else(|| anyhow!("unsafe-cache-directory"))?;
    if !canonical_path(existing)?.starts_with(&root)
        || fs::symlink_metadata(&directory).is_ok_and(|m| m.file_type().is_symlink())
    {
        bail!("unsafe-cache-directory");
    }
    fs::create_dir_all(&directory)?;
    if !canonical_path(&directory)?.starts_with(&root) {
        bail!("unsafe-cache-directory");
    }
    Ok(directory)
}
fn remove_record(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
fn read_record(path: &Path) -> Option<Value> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() || metadata.len() > 16384 {
        return None;
    }
    let value: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    value.is_object().then_some(value)
}
fn atomic_record(path: &Path, value: &Value) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| anyhow!("invalid-record-path"))?,
    )?;
    file.write_all(&canonical(value)?)?;
    file.persist(path)
        .map_err(|_| anyhow!("atomic-report-write-failed"))?;
    Ok(())
}
fn report_valid(record: &Value, fingerprint: &str) -> bool {
    if record["schema"] != SCHEMA
        || record["mode"] != "quick"
        || record["scope"] != "all-quick"
        || record["success"] != true
        || record["fingerprint"] != fingerprint
        || record["checks"].as_u64().is_none_or(|v| v == 0)
        || record["elapsed_seconds"]
            .as_f64()
            .is_none_or(|v| !v.is_finite() || v < 0.0)
    {
        return false;
    }
    record["completed_at"].as_str().is_some_and(|text| {
        OffsetDateTime::parse(text, &Rfc3339).is_ok_and(|stamp| {
            stamp.nanosecond() == 0 && stamp.format(&Rfc3339).is_ok_and(|encoded| encoded == text)
        })
    })
}
fn probe(
    root: &Path,
    flavor: &str,
    engine: &str,
    evidence: Option<&Value>,
    environment: &Environment,
    hook: Option<&dyn Fn()>,
) -> Result<Value> {
    let directory = cache_directory(root)?;
    let report = directory.join(format!("{flavor}-success.json"));
    if environment_disabled(environment) {
        remove_record(&report)?;
        return Ok(
            json!({"hit":false,"ticket":"","message":"Quick cache disabled in CI or custom execution environment; real checks required"}),
        );
    }
    let current = match fingerprints_with_hook(root, flavor, engine, evidence, environment, hook) {
        Ok(value) => value,
        Err(error) => {
            remove_record(&report)?;
            return Err(error);
        }
    };
    if current["dependencies_were_ready"] == true {
        if let Some(previous) = read_record(&report)
            .filter(|value| report_valid(value, current["fingerprint"].as_str().unwrap_or("")))
        {
            return Ok(
                json!({"hit":true,"ticket":"","message":format!("QUICK CACHED (not release validation): reused {} successful checks from {}; current privacy/version/diff gates ran live",previous["checks"],previous["completed_at"].as_str().unwrap_or(""))}),
            );
        }
    }
    remove_record(&report)?;
    let ticket = Uuid::new_v4().simple().to_string();
    atomic_record(
        &directory.join(format!("{ticket}.pending.json")),
        &json!({"schema":SCHEMA,"flavor":flavor,"source":current["source"],"dependencies":current["dependencies"],"dependencies_were_ready":current["dependencies_were_ready"]}),
    )?;
    Ok(json!({"hit":false,"ticket":ticket,"message":"Quick cache miss: real checks required"}))
}
#[allow(clippy::too_many_arguments)]
fn finish(
    root: &Path,
    flavor: &str,
    engine: &str,
    ticket: &str,
    checks: u64,
    elapsed: f64,
    success: bool,
    evidence: Option<&Value>,
    environment: &Environment,
    hook: Option<&dyn Fn()>,
) -> Result<Value> {
    let directory = cache_directory(root)?;
    let report = directory.join(format!("{flavor}-success.json"));
    if ticket.len() != 32
        || !ticket
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        bail!("invalid-ticket");
    }
    let pending = directory.join(format!("{ticket}.pending.json"));
    let record = read_record(&pending);
    remove_record(&pending)?;
    remove_record(&report)?;
    if !success || environment_disabled(environment) {
        return Ok(
            json!({"stored":false,"message":"Quick cache invalidated after failure or interruption"}),
        );
    }
    let current = fingerprints_with_hook(root, flavor, engine, evidence, environment, hook)?;
    let valid = record.is_some_and(|r| {
        r["schema"] == SCHEMA
            && r["flavor"] == flavor
            && r["source"] == current["source"]
            && r["dependencies_were_ready"] == true
            && r["dependencies"] == current["dependencies"]
    });
    if !valid
        || checks < 1
        || !elapsed.is_finite()
        || elapsed < 0.0
        || current["dependencies_were_ready"] != true
    {
        return Ok(
            json!({"stored":false,"message":"Quick inputs changed during verification; no reusable report was saved"}),
        );
    }
    let completed = OffsetDateTime::now_utc()
        .replace_nanosecond(0)?
        .format(&Rfc3339)?;
    atomic_record(
        &report,
        &json!({"schema":SCHEMA,"mode":"quick","scope":"all-quick","success":true,"fingerprint":current["fingerprint"],"checks":checks,"elapsed_seconds":elapsed,"completed_at":completed}),
    )?;
    Ok(
        json!({"stored":true,"message":"Quick successful report saved locally; Full/CI never reuse it"}),
    )
}

pub fn run(arguments: &[String]) -> Result<()> {
    let execute = || -> Result<Value> {
        let action = arguments
            .first()
            .map(String::as_str)
            .ok_or_else(|| anyhow!("missing-cache-action"))?;
        let mut options = BTreeMap::new();
        for pair in arguments[1..].chunks(2) {
            if pair.len() != 2
                || !["--flavor", "--engine", "--ticket", "--checks", "--elapsed"]
                    .contains(&pair[0].as_str())
                || options.insert(pair[0].as_str(), pair[1].as_str()).is_some()
            {
                bail!("invalid-cache-option");
            }
        }
        let flavor = *options
            .get("--flavor")
            .ok_or_else(|| anyhow!("missing-cache-flavor"))?;
        if !["bash", "powershell"].contains(&flavor) {
            bail!("invalid-cache-flavor");
        }
        let engine = *options
            .get("--engine")
            .ok_or_else(|| anyhow!("missing-cache-engine"))?;
        let root = crate::support::root()?;
        let environment = environment_values(env::vars_os())?;
        match action {
            "clear" => {
                remove_record(&cache_directory(&root)?.join(format!("{flavor}-success.json")))?;
                Ok(json!({"stored":false,"message":"Quick cache cleared"}))
            }
            "probe" => probe(&root, flavor, engine, None, &environment, None),
            "store" | "invalidate" => finish(
                &root,
                flavor,
                engine,
                options.get("--ticket").copied().unwrap_or(""),
                options.get("--checks").copied().unwrap_or("0").parse()?,
                options.get("--elapsed").copied().unwrap_or("0").parse()?,
                action == "store",
                None,
                &environment,
                None,
            ),
            _ => bail!("invalid-cache-action"),
        }
    };
    match execute() {
        Ok(value) => {
            println!("{}", serde_json::to_string(&value)?);
            Ok(())
        }
        Err(_) => {
            println!(
                "{}",
                json!({"hit":false,"ticket":"","stored":false,"message":"Quick cache unavailable; real checks required"})
            );
            bail!("quick-cache-unavailable")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_environment_encoding_is_controlled_failure() {
        #[cfg(unix)]
        let invalid = {
            use std::os::unix::ffi::OsStringExt;
            std::ffi::OsString::from_vec(vec![255])
        };
        #[cfg(windows)]
        let invalid = {
            use std::os::windows::ffi::OsStringExt;
            std::ffi::OsString::from_wide(&[0xd800])
        };
        assert!(
            environment_values([(std::ffi::OsString::from("SYNTHETIC"), invalid)].into_iter())
                .is_err()
        );
    }
    struct Fixture {
        directory: tempfile::TempDir,
        evidence: Value,
    }
    impl Fixture {
        fn new() -> Self {
            let value = Self {
                directory: tempfile::tempdir().unwrap(),
                evidence: json!({"compiler":"synthetic-version-1"}),
            };
            value.git(&["init", "--quiet"]);
            value.git(&["config", "user.name", "fixture"]);
            value.git(&["config", "user.email", "fixture@example.invalid"]);
            value.write(
                ".gitignore",
                ".cache/\nadmin-web/node_modules/\nadmin-web/dist/\nadmin-web/.env*\nruntime/\n",
            );
            value.write("source.rs", "pub fn synthetic() {}\n");
            value.write("admin-web/package-lock.json", "{}");
            value.write(
                "admin-web/node_modules/module/index.js",
                "module.exports = 1;",
            );
            value.marker();
            value.write("admin-web/dist/index.html", "<main>fixture</main>");
            value.git(&["add", "."]);
            value.git(&["commit", "--quiet", "-m", "synthetic fixture"]);
            value
        }
        fn root(&self) -> &Path {
            self.directory.path()
        }
        fn write(&self, name: &str, text: &str) {
            let p = self.root().join(name);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        fn git(&self, args: &[&str]) {
            git(self.root(), args).unwrap();
        }
        fn marker(&self) {
            self.write(
                "admin-web/node_modules/.vpn233-lock.sha256",
                &sha256_file(&self.root().join("admin-web/package-lock.json")).unwrap(),
            );
        }
        fn probe_env(&self, environment: &Environment, evidence: &Value) -> Value {
            probe(
                self.root(),
                "powershell",
                "fixture-engine",
                Some(evidence),
                environment,
                None,
            )
            .unwrap()
        }
        fn probe(&self) -> Value {
            self.probe_env(&Environment::new(), &self.evidence)
        }
        fn finish_env(&self, ticket: &str, success: bool, environment: &Environment) -> Value {
            finish(
                self.root(),
                "powershell",
                "fixture-engine",
                ticket,
                11,
                12.5,
                success,
                Some(&self.evidence),
                environment,
                None,
            )
            .unwrap()
        }
        fn finish(&self, ticket: &str) -> Value {
            self.finish_env(ticket, true, &Environment::new())
        }
        fn seed(&self) {
            let r = self.probe();
            assert_eq!(r["hit"], false);
            assert_eq!(self.finish(r["ticket"].as_str().unwrap())["stored"], true);
            assert_eq!(self.probe()["hit"], true);
        }
        fn report(&self) -> PathBuf {
            self.root()
                .join(".cache/quick-verify/powershell-success.json")
        }
    }
    fn environment(key: &str, value: &str) -> Environment {
        [(key.into(), value.into())].into_iter().collect()
    }
    #[test]
    fn success_reused_only_digests_persisted() {
        let f = Fixture::new();
        f.seed();
        let text = fs::read_to_string(f.report()).unwrap();
        for secret in [
            f.root().to_str().unwrap(),
            "synthetic-version",
            "module.exports",
        ] {
            assert!(!text.contains(secret));
        }
    }
    #[test]
    fn same_size_and_mtime_source_edit_misses() {
        let f = Fixture::new();
        f.seed();
        let path = f.root().join("source.rs");
        let time = fs::metadata(&path).unwrap().modified().unwrap();
        f.write("source.rs", "pub fn Synthetic() {}\n");
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(time))
            .unwrap();
        assert_eq!(f.probe()["hit"], false);
    }
    #[test]
    fn new_deleted_and_index_changes_miss() {
        for change in 0..3 {
            let f = Fixture::new();
            f.seed();
            match change {
                0 => f.write("new.rs", "//fixture"),
                1 => fs::remove_file(f.root().join("source.rs")).unwrap(),
                _ => f.git(&["rm", "--cached", "source.rs"]),
            };
            assert_eq!(f.probe()["hit"], false);
        }
    }
    #[test]
    fn history_refs_miss_with_identical_tree() {
        let f = Fixture::new();
        f.seed();
        f.git(&["commit", "--allow-empty", "--quiet", "-m", "second fixture"]);
        assert_eq!(f.probe()["hit"], false);
    }
    #[test]
    fn tools_environment_configuration_miss() {
        let f = Fixture::new();
        f.seed();
        assert_eq!(
            f.probe_env(
                &Environment::new(),
                &json!({"compiler":"synthetic-version-2"})
            )["hit"],
            false
        );
        f.seed();
        assert_eq!(
            f.probe_env(&environment("RUSTFLAGS", "--cfg synthetic"), &f.evidence)["hit"],
            false
        );
        f.seed();
        f.write(".cargo/config.toml", "[build]\njobs=1\n");
        assert_eq!(f.probe()["hit"], false);
    }
    #[test]
    fn vite_environment_ignored_env_miss() {
        let f = Fixture::new();
        f.seed();
        assert_eq!(
            f.probe_env(
                &environment("VITE_API_BASE_URL", "https://example.invalid"),
                &f.evidence
            )["hit"],
            false
        );
        f.seed();
        f.write(
            "admin-web/.env.production.local",
            "VITE_PUBLIC_SUBSCRIPTION_ORIGIN=https://example.invalid\n",
        );
        assert_eq!(f.probe()["hit"], false);
    }
    #[test]
    fn case_variant_and_native_flags_miss() {
        let f = Fixture::new();
        f.seed();
        for (key, value) in [
            ("Path", "synthetic-path"),
            ("CFLAGS", "-DSYNTHETIC"),
            ("BASH_ENV", "synthetic-startup"),
            ("CARGO_PROFILE_DEV_OPT_LEVEL", "1"),
            ("RUSTDOCFLAGS", "--cfg synthetic"),
            ("MSYS_NO_PATHCONV", "1"),
        ] {
            assert_eq!(
                f.probe_env(&environment(key, value), &f.evidence)["hit"],
                false
            );
        }
    }
    #[test]
    fn ignored_and_external_npm_configuration_contents_miss() {
        let f = Fixture::new();
        f.seed();
        f.write("admin-web/.npmrc", "fund=false\n");
        assert_eq!(f.probe()["hit"], false);
        f.write("runtime/npm-config", "fund=false\n");
        let e = environment(
            "NPM_CONFIG_USERCONFIG",
            f.root().join("runtime/npm-config").to_str().unwrap(),
        );
        let first = f.probe_env(&e, &f.evidence);
        assert_eq!(
            f.finish_env(first["ticket"].as_str().unwrap(), true, &e)["stored"],
            true
        );
        f.write("runtime/npm-config", "fund=true\n");
        assert_eq!(f.probe_env(&e, &f.evidence)["hit"], false);
    }
    #[test]
    fn dependency_permissions_hashed() {
        let f = Fixture::new();
        f.seed();
        let p = f.root().join("admin-web/node_modules/module/index.js");
        let original = fs::metadata(&p).unwrap().permissions();
        let mut changed = original.clone();
        changed.set_readonly(true);
        fs::set_permissions(&p, changed).unwrap();
        assert_eq!(f.probe()["hit"], false);
        fs::set_permissions(p, original).unwrap();
    }
    #[test]
    fn dependencies_output_content_hashed_without_mtime() {
        for p in [
            "admin-web/node_modules/module/index.js",
            "admin-web/dist/index.html",
        ] {
            let f = Fixture::new();
            f.seed();
            let path = f.root().join(p);
            let time = fs::metadata(&path).unwrap().modified().unwrap();
            let text = fs::read_to_string(&path)
                .unwrap()
                .replace('1', "2")
                .replace("fixture", "Fixture");
            f.write(p, &text);
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(time))
                .unwrap();
            assert_eq!(f.probe()["hit"], false);
        }
    }
    #[test]
    fn ignored_runtime_does_not_invalidate() {
        let f = Fixture::new();
        f.seed();
        f.write("runtime/synthetic.txt", "synthetic runtime");
        assert_eq!(f.probe()["hit"], true);
    }
    #[test]
    fn bad_marker_ci_missing_dependencies_never_hit() {
        let f = Fixture::new();
        f.seed();
        assert_eq!(
            f.probe_env(&environment("CI", "true"), &f.evidence)["hit"],
            false
        );
        f.seed();
        f.write("admin-web/node_modules/.vpn233-lock.sha256", "wrong");
        assert_eq!(f.probe()["hit"], false);
        fs::remove_file(f.root().join("admin-web/node_modules/.vpn233-lock.sha256")).unwrap();
        assert_eq!(f.probe()["hit"], false);
    }
    #[test]
    fn failed_aborted_reports_removed() {
        let f = Fixture::new();
        let first = f.probe();
        assert_eq!(
            f.finish_env(
                first["ticket"].as_str().unwrap(),
                false,
                &Environment::new()
            )["stored"],
            false
        );
        assert!(!f.report().exists());
        f.seed();
        f.write("source.rs", "// changed");
        let missed = f.probe();
        assert!(!f.report().exists());
        assert_eq!(
            f.finish_env(
                missed["ticket"].as_str().unwrap(),
                false,
                &Environment::new()
            )["stored"],
            false
        );
        assert_eq!(f.probe()["hit"], false);
    }
    #[test]
    fn source_dependency_change_during_checks_cannot_seed() {
        for p in ["source.rs", "admin-web/node_modules/module/index.js"] {
            let f = Fixture::new();
            let first = f.probe();
            f.write(p, "// changed during verification");
            assert_eq!(f.finish(first["ticket"].as_str().unwrap())["stored"], false);
        }
    }
    #[test]
    fn parallel_snapshot_bookend_rejects_probe_and_store() {
        for store in [false, true] {
            let f = Fixture::new();
            f.seed();
            let ticket = if store {
                f.write("source.rs", "// verification input");
                f.probe()["ticket"].as_str().unwrap().to_owned()
            } else {
                String::new()
            };
            let hook = || f.write("source.rs", "// modified during parallel fingerprint");
            let result = if store {
                finish(
                    f.root(),
                    "powershell",
                    "fixture-engine",
                    &ticket,
                    11,
                    1.,
                    true,
                    Some(&f.evidence),
                    &Environment::new(),
                    Some(&hook),
                )
            } else {
                probe(
                    f.root(),
                    "powershell",
                    "fixture-engine",
                    Some(&f.evidence),
                    &Environment::new(),
                    Some(&hook),
                )
            };
            assert!(result.is_err());
            assert!(!f.report().exists());
        }
    }
    #[test]
    fn first_install_needs_another_stable_verification() {
        let f = Fixture::new();
        fs::remove_file(f.root().join("admin-web/node_modules/.vpn233-lock.sha256")).unwrap();
        let first = f.probe();
        f.write(
            "admin-web/node_modules/module/index.js",
            "new installed fixture",
        );
        f.marker();
        f.write("admin-web/dist/index.html", "rebuilt output");
        assert_eq!(f.finish(first["ticket"].as_str().unwrap())["stored"], false);
        assert_eq!(
            f.finish(f.probe()["ticket"].as_str().unwrap())["stored"],
            true
        );
        assert_eq!(f.probe()["hit"], true);
    }
    #[test]
    fn store_independently_rejects_ci() {
        let f = Fixture::new();
        let first = f.probe();
        assert_eq!(
            f.finish_env(
                first["ticket"].as_str().unwrap(),
                true,
                &environment("CI", "true")
            )["stored"],
            false
        );
        assert!(!f.report().exists());
    }
    #[test]
    fn corrupt_partial_failed_wrong_schema_never_hit() {
        let f = Fixture::new();
        f.seed();
        let original = read_record(&f.report()).unwrap();
        for (key, value) in [
            ("schema", json!(0)),
            ("mode", json!("full")),
            ("scope", json!("partial")),
            ("success", json!(false)),
            ("checks", json!(true)),
            ("elapsed_seconds", json!(-1)),
            ("fingerprint", json!("wrong")),
            ("completed_at", json!("invalid control text")),
        ] {
            let mut changed = original.clone();
            changed[key] = value;
            fs::write(f.report(), serde_json::to_vec(&changed).unwrap()).unwrap();
            assert_eq!(f.probe()["hit"], false);
        }
        fs::write(f.report(), b"{corrupt").unwrap();
        assert_eq!(f.probe()["hit"], false);
    }
    #[test]
    fn tickets_single_use_and_traversal_rejected() {
        let f = Fixture::new();
        let first = f.probe();
        let ticket = first["ticket"].as_str().unwrap();
        assert_eq!(f.finish(ticket)["stored"], true);
        assert_eq!(f.finish(ticket)["stored"], false);
        assert!(finish(
            f.root(),
            "powershell",
            "fixture-engine",
            "../invalid-ticket",
            11,
            1.,
            true,
            Some(&f.evidence),
            &Environment::new(),
            None
        )
        .is_err());
    }
    #[test]
    fn external_dependency_symlink_disables_cache() {
        let f = Fixture::new();
        let destination = f.root().join("admin-web/node_modules/external");
        #[cfg(unix)]
        std::os::unix::fs::symlink(f.root().join("source.rs"), destination).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_file(f.root().join("source.rs"), destination).is_err() {
            return;
        }
        assert!(probe(
            f.root(),
            "powershell",
            "fixture-engine",
            Some(&f.evidence),
            &Environment::new(),
            None
        )
        .is_err());
    }
}
