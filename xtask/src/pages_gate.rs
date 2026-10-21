//! Publish Pages only for the latest completely verified stable release.
//! Authentication is confined to api.github.com; diagnostics never contain values.
use anyhow::{anyhow, bail, ensure, Result};
use regex::Regex;
use reqwest::{
    blocking::Client,
    header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT},
    redirect::Policy,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::OpenOptions,
    io::{Read, Write},
    path::PathBuf,
    process::Command,
    time::Duration,
};

const TASK: &str = "vpn233-pages-proof";
const RELEASE_PATH: &str = ".github/workflows/release.yml";
const DOCS_PATH: &str = ".github/workflows/docs.yml";
pub(crate) trait Api {
    fn get(&self, path: &str) -> Result<Value>;
    fn post(&self, path: &str, body: &Value) -> Result<Value>;
    fn patch(&self, _path: &str, _body: &Value) -> Result<Value> {
        bail!("github-api-mutation-unavailable")
    }
}
#[derive(Debug)]
pub(crate) struct ApiNotFound;
impl std::fmt::Display for ApiNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("github-api-object-not-found")
    }
}
impl std::error::Error for ApiNotFound {}
pub(crate) struct GitHub {
    client: Client,
    base: String,
}
impl GitHub {
    pub(crate) fn new(repo: &str) -> Result<Self> {
        ensure!(
            Regex::new(r"\A[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9][A-Za-z0-9_.-]*\z")?.is_match(repo),
            "pages-invalid-repository"
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("vpn233-xtask-pages-gate"),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            HeaderValue::from_static("2026-03-10"),
        );
        let token = env::var("GH_TOKEN")
            .or_else(|_| env::var("GITHUB_TOKEN"))
            .map_err(|_| anyhow!("pages-github-token-required"))?;
        ensure!(!token.is_empty(), "pages-github-token-required");
        let mut bearer = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| anyhow!("pages-invalid-authentication"))?;
        bearer.set_sensitive(true);
        headers.insert(AUTHORIZATION, bearer);
        Ok(Self {
            client: Client::builder()
                .default_headers(headers)
                .redirect(Policy::none())
                .https_only(true)
                .timeout(Duration::from_secs(20))
                .connect_timeout(Duration::from_secs(5))
                .build()?,
            base: format!("https://api.github.com/repos/{repo}"),
        })
    }
    fn request(&self, path: &str, body: Option<&Value>, patch: bool) -> Result<Value> {
        ensure!(
            path.is_empty()
                || path.starts_with('/') && !path.contains("..") && !path.contains(['\r', '\n']),
            "pages-invalid-api-path"
        );
        let request = match body {
            Some(value) if patch => self
                .client
                .patch(format!("{}{path}", self.base))
                .json(value),
            Some(value) => self.client.post(format!("{}{path}", self.base)).json(value),
            None => self.client.get(format!("{}{path}", self.base)),
        };
        let response = request
            .send()
            .map_err(|_| anyhow!("pages-github-read-unavailable"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(ApiNotFound.into());
        }
        ensure!(response.status().is_success(), "pages-github-api-rejected");
        let mut bytes = Vec::new();
        response
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow!("pages-github-body-unavailable"))?;
        ensure!(
            bytes.len() <= 4 * 1024 * 1024,
            "pages-github-body-too-large"
        );
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("pages-github-json-invalid"))
    }
}
impl Api for GitHub {
    fn get(&self, path: &str) -> Result<Value> {
        self.request(path, None, false)
    }
    fn post(&self, path: &str, body: &Value) -> Result<Value> {
        self.request(path, Some(body), false)
    }
    fn patch(&self, path: &str, body: &Value) -> Result<Value> {
        self.request(path, Some(body), true)
    }
}
trait GitState {
    fn ancestor(&self, sha: &str) -> Result<bool>;
    fn docs_tree(&self, sha: &str) -> Result<String>;
}
struct LocalGit {
    root: PathBuf,
}
impl GitState for LocalGit {
    fn ancestor(&self, sha: &str) -> Result<bool> {
        let result = crate::support::process_output(
            Command::new("git")
                .args(["merge-base", "--is-ancestor", sha, "origin/main"])
                .current_dir(&self.root),
            None,
            Duration::from_secs(20),
        )?;
        match result.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => bail!("pages-main-ancestry-unavailable"),
        }
    }
    fn docs_tree(&self, sha: &str) -> Result<String> {
        ensure!(valid_sha(sha), "pages-invalid-commit");
        let result = crate::support::process_output(
            Command::new("git")
                .args(["rev-parse", &format!("{sha}:docs")])
                .current_dir(&self.root),
            None,
            Duration::from_secs(20),
        )?;
        ensure!(
            result.status.success(),
            "pages-documentation-tree-unavailable"
        );
        let value = std::str::from_utf8(&result.stdout)?.trim();
        ensure!(valid_sha(value), "pages-documentation-tree-invalid");
        Ok(value.into())
    }
}
pub(crate) fn valid_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| anyhow!("pages-missing-api-field"))
}
fn number(value: &Value, key: &str) -> Result<u64> {
    value[key]
        .as_u64()
        .filter(|v| *v > 0)
        .ok_or_else(|| anyhow!("pages-invalid-api-id"))
}
pub(crate) fn collection(api: &impl Api, path: &str, key: Option<&str>) -> Result<Vec<Value>> {
    let mut output = Vec::new();
    for page in 1..=100 {
        let url = format!(
            "{path}{}per_page=100&page={page}",
            if path.contains('?') { '&' } else { '?' }
        );
        let data = api.get(&url)?;
        let items = match key {
            Some(key) => &data[key],
            None => &data,
        }
        .as_array()
        .ok_or_else(|| anyhow!("pages-api-collection-invalid"))?;
        output.extend(items.iter().cloned());
        if items.len() < 100 {
            return Ok(output);
        }
    }
    bail!("pages-api-pagination-limit")
}
fn successful_job(job: &Value) -> bool {
    job["status"] == "completed" && job["conclusion"] == "success"
}
fn actual_job_completed(job: &Value) -> bool {
    successful_job(job)
        || job["status"] == "in_progress"
            && job["conclusion"].is_null()
            && job["steps"].as_array().is_some_and(|steps| {
                !steps.is_empty()
                    && steps.iter().all(|s| {
                        s["status"] == "completed"
                            && matches!(s["conclusion"].as_str(), Some("success" | "skipped"))
                    })
                    && steps
                        .iter()
                        .any(|s| s["name"] == "Complete job" && s["conclusion"] == "success")
            })
}
pub(crate) fn critical_steps(name: &str) -> &'static [&'static str] {
    match name {
        "Tag and version preflight" => &["Verify main ancestry and shared version"],
        "Bundle macos-x86_64" | "Bundle macos-aarch64" => &[
            "Verify macOS architecture, versions and real loopback E2E",
            "Assemble deterministic fast-compressed archives and verify privacy",
        ],
        "Bundle linux-x86_64" | "Bundle windows-x86_64" => {
            &["Assemble deterministic fast-compressed archives and verify privacy"]
        }
        "Verify all checksums and archive allowlists before publication" => {
            &["Generate and independently verify exact release asset set"]
        }
        "Publish public release candidate" => &[
            "Require a new candidate release",
            "Attach verified assets to a draft",
            "Publish checksum-verified candidate",
            "Preserve candidate identity proof",
        ],
        "Promote accepted release" => &["Promote and independently confirm stable release"],
        "Verify published downloads" => &["Download and verify published release"],
        "Public installation and online update (Linux)"
        | "Public installation and online update (Windows)" => {
            &["Install and update from public release"]
        }
        "Verify (full CI) / Rust (Linux)" | "Verify (full CI) / Rust (Windows)" => &[
            "Format",
            "Clippy",
            "Test",
            "Build release binaries once",
            "Build and scan the privacy-remapped native verification tool",
            "Scan source before creating public build artifacts",
        ],
        "Verify (full CI) / Scripts (Linux)" => &[
            "PowerShell parse, lint and installer tests",
            "Shell syntax, lint and installer tests",
        ],
        "Verify (full CI) / Scripts (Windows)" => &["PowerShell parse, lint and installer tests"],
        "Verify (full CI) / Privacy (full history and candidates)" => &[
            "Scan full history and candidates with Rust",
            "Gitleaks full history (redacted)",
        ],
        "Verify (full CI) / Dependency audit" => &["Rust dependency audit", "Web dependency audit"],
        "Verify (full CI) / Docs and version contract" => {
            &["Verify shared version", "Check documentation contract"]
        }
        "Verify (full CI) / Admin web" => &["Build admin web", "Validate admin web output"],
        "Verify (full CI) / Workflow lint" => &["Validate workflows"],
        "Verify (full CI) / E2E (Linux)" => &[
            "Linux E2E",
            "Validate all generated inbounds with the official Mihomo parser",
            "Real browser UI, session, mobile and zero-external-request tests",
            "Package the real platform binaries and verify public archive contents",
            "Verify installed archive startup, health and owned shutdown",
        ],
        "Verify (full CI) / E2E (Windows)" => &[
            "Windows E2E (PowerShell 5.1)",
            "Validate all generated inbounds with the official Mihomo parser",
            "Package the real platform binaries and verify public archive contents",
            "Verify installed archive startup, health and owned shutdown",
        ],
        _ => &[],
    }
}
pub(crate) fn release_job_names(promoted: bool) -> Vec<&'static str> {
    let mut expected = vec![
        "Tag and version preflight",
        "Bundle linux-x86_64",
        "Bundle windows-x86_64",
        "Bundle macos-x86_64",
        "Bundle macos-aarch64",
        "Verify all checksums and archive allowlists before publication",
        "Publish public release candidate",
        "Verify published downloads",
        "Public installation and online update (Linux)",
        "Public installation and online update (Windows)",
        "Verify (full CI) / Rust (Linux)",
        "Verify (full CI) / Rust (Windows)",
        "Verify (full CI) / Scripts (Linux)",
        "Verify (full CI) / Scripts (Windows)",
        "Verify (full CI) / Privacy (full history and candidates)",
        "Verify (full CI) / Dependency audit",
        "Verify (full CI) / Docs and version contract",
        "Verify (full CI) / Admin web",
        "Verify (full CI) / Workflow lint",
        "Verify (full CI) / E2E (Linux)",
        "Verify (full CI) / E2E (Windows)",
    ];
    if promoted {
        expected.push("Promote accepted release");
    }
    expected
}
pub(crate) fn validate_release_jobs(jobs: &[Value], promoted: bool) -> Result<Vec<String>> {
    let expected = release_job_names(promoted);
    ensure!(
        jobs.len() == expected.len() && jobs.iter().all(actual_job_completed),
        "pages-release-jobs-not-successful"
    );
    for name in expected {
        ensure!(
            jobs.iter().filter(|j| j["name"] == name).count() == 1,
            "pages-release-required-job-missing-or-duplicated"
        );
    }
    for job in jobs {
        for name in critical_steps(text(job, "name")?) {
            ensure!(
                job["steps"].as_array().is_some_and(|steps| steps
                    .iter()
                    .any(|s| s["name"] == *name
                        && s["status"] == "completed"
                        && s["conclusion"] == "success")),
                "pages-required-business-step-not-successful"
            );
        }
    }
    jobs.iter()
        .filter(|j| !successful_job(j))
        .map(|j| text(j, "name").map(str::to_owned))
        .collect::<Result<_>>()
}
pub(crate) fn tag_commit(api: &impl Api, tag: &str) -> Result<String> {
    let reference = api.get(&format!("/git/ref/tags/{tag}"))?;
    let mut object = reference["object"].clone();
    for _ in 0..8 {
        let sha = text(&object, "sha")?.to_owned();
        ensure!(valid_sha(&sha), "pages-invalid-tag-object");
        match text(&object, "type")? {
            "commit" => return Ok(sha),
            "tag" => object = api.get(&format!("/git/tags/{sha}"))?["object"].clone(),
            _ => bail!("pages-tag-is-not-a-commit"),
        }
    }
    bail!("pages-tag-chain-too-long")
}
fn stable_release(release: &Value) -> Result<String> {
    ensure!(
        release["draft"] == false
            && release["prerelease"] == false
            && release["published_at"].as_str().is_some(),
        "pages-stable-published-release-required"
    );
    let tag = text(release, "tag_name")?;
    ensure!(
        Regex::new(r"\Av(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\z")?.is_match(tag),
        "pages-stable-semver-required"
    );
    let assets = release["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("pages-release-assets-missing"))?;
    let mut expected: BTreeSet<String> = ["SHA256SUMS".into(), "version.txt".into()]
        .into_iter()
        .collect();
    for platform in [
        "linux-x86_64",
        "windows-x86_64",
        "macos-x86_64",
        "macos-aarch64",
    ] {
        for extension in ["tar.gz", "zip"] {
            expected.insert(format!("vpn233-rust-{platform}.{extension}"));
        }
    }
    let actual: BTreeSet<_> = assets
        .iter()
        .map(|a| text(a, "name").map(str::to_owned))
        .collect::<Result<_>>()?;
    ensure!(
        actual == expected && assets.len() == 10,
        "pages-release-assets-incomplete"
    );
    for asset in assets {
        ensure!(
            asset["size"].as_u64().is_some_and(|v| v > 0)
                && asset["digest"]
                    .as_str()
                    .is_some_and(|v| v.strip_prefix("sha256:").is_some_and(|s| s.len() == 64
                        && s.bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))),
            "pages-release-api-digest-missing"
        );
    }
    Ok(tag.into())
}
fn previous_docs_tree(api: &impl Api, git: &impl GitState) -> Result<Option<String>> {
    let records = collection(
        api,
        &format!("/deployments?task={TASK}&environment={TASK}"),
        None,
    )?;
    if let Some(record) = records.first() {
        ensure!(
            record["task"] == TASK
                && record["environment"] == TASK
                && record["creator"]["login"] == "github-actions[bot]",
            "pages-proof-origin-invalid"
        );
        let id = number(record, "id")?;
        let statuses = api.get(&format!("/deployments/{id}/statuses?per_page=100"))?;
        ensure!(
            statuses
                .as_array()
                .and_then(|a| a.first())
                .is_some_and(|s| s["state"] == "success"),
            "pages-existing-proof-is-not-successful"
        );
        let sha = text(record, "sha")?;
        let tree = text(&record["payload"], "docs_tree")?;
        ensure!(
            valid_sha(sha)
                && valid_sha(tree)
                && number(&record["payload"], "release_run_id").is_ok()
                && number(&record["payload"], "pages_run_id").is_ok(),
            "pages-proof-payload-invalid"
        );
        ensure!(
            git.docs_tree(sha)? == tree,
            "pages-proof-tree-does-not-match-commit"
        );
        return Ok(Some(tree.into()));
    }
    // Only the old push-triggered workflow had head_sha equal to its checkout.
    // Never infer a release checkout from workflow_run/manual head_sha.
    for run in collection(
        api,
        "/actions/workflows/docs.yml/runs",
        Some("workflow_runs"),
    )? {
        if run["event"] != "push"
            || run["status"] != "completed"
            || run["conclusion"] != "success"
            || run["path"] != DOCS_PATH
        {
            continue;
        }
        let jobs = collection(
            api,
            &format!("/actions/runs/{}/jobs", number(&run, "id")?),
            Some("jobs"),
        )?;
        if jobs
            .iter()
            .any(|j| j["name"] == "Deploy to GitHub Pages" && successful_job(j))
        {
            let sha = text(&run, "head_sha")?;
            ensure!(valid_sha(sha), "pages-legacy-commit-invalid");
            return Ok(Some(git.docs_tree(sha)?));
        }
    }
    for record in collection(api, "/deployments?environment=github-pages", None)? {
        let statuses = api.get(&format!(
            "/deployments/{}/statuses?per_page=100",
            number(&record, "id")?
        ))?;
        ensure!(
            !statuses
                .as_array()
                .ok_or_else(|| anyhow!("pages-status-collection-invalid"))?
                .iter()
                .any(|s| s["state"] == "success" || s["state"] == "inactive"),
            "pages-existing-deployment-has-no-trusted-baseline"
        );
    }
    Ok(None)
}
fn evaluate(api: &impl Api, git: &impl GitState, repo: &str, run_id: u64) -> Result<Value> {
    ensure!(run_id > 0, "pages-release-run-id-required");
    let repository = api.get("")?;
    ensure!(
        repository["full_name"] == repo
            && repository["default_branch"] == "main"
            && repository["private"] == false
            && repository["fork"] == false,
        "pages-repository-identity-invalid"
    );
    let run = api.get(&format!("/actions/runs/{run_id}"))?;
    ensure!(
        run["event"] == "push"
            && run["path"] == RELEASE_PATH
            && run["status"] == "completed"
            && run["conclusion"] == "success"
            && run["repository"]["id"] == repository["id"]
            && run["repository"]["full_name"] == repo,
        "pages-complete-official-release-run-required"
    );
    let sha = text(&run, "head_sha")?;
    ensure!(
        valid_sha(sha) && git.ancestor(sha)?,
        "pages-release-is-not-on-main"
    );
    let summary_lag_jobs = validate_release_jobs(
        &collection(api, &format!("/actions/runs/{run_id}/jobs"), Some("jobs"))?,
        true,
    )?;
    let release = api.get("/releases/latest")?;
    let tag = stable_release(&release)?;
    ensure!(
        tag_commit(api, &tag)? == sha,
        "pages-latest-release-commit-differs"
    );
    // Fetch by tag too: a stale or unrelated latest response must not suffice.
    let by_tag = api.get(&format!("/releases/tags/{tag}"))?;
    ensure!(
        number(&release, "id")? == number(&by_tag, "id")? && stable_release(&by_tag)? == tag,
        "pages-latest-release-identity-differs"
    );
    let tree = git.docs_tree(sha)?;
    let previous = previous_docs_tree(api, git)?;
    Ok(
        json!({"deploy":previous.as_deref()!=Some(&tree),"release_sha":sha,"docs_tree":tree,"version":&tag[1..],"release_run_id":run_id,"previous_docs_tree":previous,"summary_lag_jobs":summary_lag_jobs,"reason":if previous.as_deref()==Some(&tree){"documentation tree unchanged; Jekyll and deployment skipped"}else{"latest stable release fully verified; documentation changed"}}),
    )
}
fn verified_pages_execution(
    run: &Value,
    jobs: &[Value],
    repo: &str,
    repository_id: &Value,
) -> Result<()> {
    ensure!(
        run["path"] == DOCS_PATH
            && matches!(
                run["event"].as_str(),
                Some("workflow_run" | "workflow_dispatch")
            )
            && run["repository"]["full_name"] == repo
            && &run["repository"]["id"] == repository_id
            && !matches!(
                run["conclusion"].as_str(),
                Some("failure" | "cancelled" | "timed_out")
            ),
        "pages-record-workflow-origin-invalid"
    );
    ensure!(
        jobs.iter()
            .any(|j| j["name"] == "Deploy verified documentation"
                && j["steps"].as_array().is_some_and(|steps| steps
                    .iter()
                    .any(|s| s["name"] == "Deploy to GitHub Pages"
                        && s["status"] == "completed"
                        && s["conclusion"] == "success"))),
        "pages-record-requires-actual-successful-deployment"
    );
    Ok(())
}
fn record(
    api: &impl Api,
    git: &impl GitState,
    repo: &str,
    release_run: u64,
    pages_run: u64,
    sha: &str,
    tree: &str,
) -> Result<Value> {
    let decision = evaluate(api, git, repo, release_run)?;
    ensure!(
        decision["release_sha"] == sha && decision["docs_tree"] == tree,
        "pages-record-inputs-differ-from-verified-release"
    );
    let repository = api.get("")?;
    verify_pages_with_retry(api, repo, pages_run, &repository["id"], &|| {
        std::thread::sleep(Duration::from_secs(2))
    })?;
    let created=api.post("/deployments",&json!({"ref":sha,"task":TASK,"environment":TASK,"auto_merge":false,"required_contexts":[],"payload":{"docs_tree":tree,"release_run_id":release_run,"pages_run_id":pages_run},"description":"Verified Pages publication proof","transient_environment":false,"production_environment":false}))?;
    ensure!(
        created["sha"] == sha && created["environment"] == TASK && created["task"] == TASK,
        "pages-proof-creation-response-invalid"
    );
    let id = number(&created, "id")?;
    let status=api.post(&format!("/deployments/{id}/statuses"),&json!({"state":"success","environment":TASK,"description":"Verified documentation publication","log_url":format!("https://github.com/{repo}/actions/runs/{pages_run}"),"auto_inactive":false}))?;
    ensure!(
        status["state"] == "success",
        "pages-proof-success-not-confirmed"
    );
    Ok(json!({"recorded":true,"deployment_id":id,"release_sha":sha,"docs_tree":tree}))
}

fn verify_pages_with_retry(
    api: &impl Api,
    repo: &str,
    pages_run: u64,
    repository_id: &Value,
    wait: &dyn Fn(),
) -> Result<()> {
    for attempt in 0..3 {
        let check = || -> Result<()> {
            let run = api.get(&format!("/actions/runs/{pages_run}"))?;
            let jobs = collection(
                api,
                &format!("/actions/runs/{pages_run}/jobs"),
                Some("jobs"),
            )?;
            verified_pages_execution(&run, &jobs, repo, repository_id)
        };
        if check().is_ok() {
            return Ok(());
        }
        if attempt < 2 {
            wait();
        }
    }
    bail!("pages-record-deployment-not-confirmed-after-bounded-reads")
}

pub fn run(arguments: &[String]) -> Result<()> {
    let is_record = arguments.first().is_some_and(|s| s == "record");
    let mut options = BTreeMap::new();
    for pair in arguments[usize::from(is_record)..].chunks(2) {
        ensure!(
            pair.len() == 2
                && [
                    "--repo",
                    "--run-id",
                    "--github-output",
                    "--pages-run-id",
                    "--release-sha",
                    "--docs-tree"
                ]
                .contains(&pair[0].as_str())
                && options.insert(pair[0].as_str(), pair[1].as_str()).is_none(),
            "pages-invalid-option"
        );
    }
    let repo = *options
        .get("--repo")
        .ok_or_else(|| anyhow!("pages-repository-required"))?;
    let id = options
        .get("--run-id")
        .ok_or_else(|| anyhow!("pages-release-run-id-required"))?
        .parse::<u64>()?;
    let api = GitHub::new(repo)?;
    let git = LocalGit {
        root: crate::support::root()?,
    };
    let result = if is_record {
        let pages_id = options
            .get("--pages-run-id")
            .ok_or_else(|| anyhow!("pages-run-id-required"))?
            .parse::<u64>()?;
        ensure!(
            env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
                && env::var("GITHUB_RUN_ID")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    == Some(pages_id)
                && env::var("GITHUB_REPOSITORY").as_deref() == Ok(repo),
            "pages-record-only-in-current-actions-run"
        );
        record(
            &api,
            &git,
            repo,
            id,
            pages_id,
            options
                .get("--release-sha")
                .ok_or_else(|| anyhow!("pages-release-sha-required"))?,
            options
                .get("--docs-tree")
                .ok_or_else(|| anyhow!("pages-docs-tree-required"))?,
        )?
    } else {
        evaluate(&api, &git, repo, id)?
    };
    if let Some(path) = options.get("--github-output") {
        let mut output = OpenOptions::new().append(true).create(true).open(path)?;
        for key in [
            "deploy",
            "release_sha",
            "docs_tree",
            "version",
            "release_run_id",
        ] {
            if let Some(value) = result.get(key) {
                writeln!(
                    output,
                    "{key}={}",
                    value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string())
                )?;
            }
        }
    }
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const REPO: &str = "fixture/project";
    const SHA: &str = "1111111111111111111111111111111111111111";
    const TREE: &str = "2222222222222222222222222222222222222222";
    const OLD: &str = "3333333333333333333333333333333333333333";
    struct Fixture {
        data: BTreeMap<String, Value>,
        ancestor: bool,
        old_tree: String,
    }
    impl Fixture {
        fn new() -> Self {
            let mut data = BTreeMap::new();
            let repository = json!({"id":1,"full_name":REPO,"private":false,"fork":false,"default_branch":"main"});
            data.insert("".into(), repository.clone());
            data.insert("/actions/runs/7".into(),json!({"id":7,"event":"push","path":RELEASE_PATH,"status":"completed","conclusion":"success","head_sha":SHA,"repository":repository}));
            let names = [
                "Tag and version preflight",
                "Bundle linux-x86_64",
                "Bundle windows-x86_64",
                "Bundle macos-x86_64",
                "Bundle macos-aarch64",
                "Verify all checksums and archive allowlists before publication",
                "Publish public release candidate",
                "Promote accepted release",
                "Verify published downloads",
                "Public installation and online update (Linux)",
                "Public installation and online update (Windows)",
                "Verify (full CI) / Rust (Linux)",
                "Verify (full CI) / Rust (Windows)",
                "Verify (full CI) / Scripts (Linux)",
                "Verify (full CI) / Scripts (Windows)",
                "Verify (full CI) / Privacy (full history and candidates)",
                "Verify (full CI) / Dependency audit",
                "Verify (full CI) / Docs and version contract",
                "Verify (full CI) / Admin web",
                "Verify (full CI) / Workflow lint",
                "Verify (full CI) / E2E (Linux)",
                "Verify (full CI) / E2E (Windows)",
            ];
            let jobs: Vec<_> = names
                .into_iter()
                .map(|name| {
                    let steps: Vec<_> = critical_steps(name)
                        .iter()
                        .chain(["Complete job"].iter())
                        .map(
                            |step| json!({"name":step,"status":"completed","conclusion":"success"}),
                        )
                        .collect();
                    json!({"name":name,"status":"completed","conclusion":"success","steps":steps})
                })
                .collect();
            data.insert(
                "/actions/runs/7/jobs?per_page=100&page=1".into(),
                json!({"jobs":jobs}),
            );
            let mut assets = vec![
                json!({"name":"SHA256SUMS","size":100,"digest":format!("sha256:{}","a".repeat(64))}),
                json!({"name":"version.txt","size":6,"digest":format!("sha256:{}","b".repeat(64))}),
            ];
            for p in [
                "linux-x86_64",
                "windows-x86_64",
                "macos-x86_64",
                "macos-aarch64",
            ] {
                for e in ["tar.gz", "zip"] {
                    assets.push(json!({"name":format!("vpn233-rust-{p}.{e}"),"size":100,"digest":format!("sha256:{}","c".repeat(64))}));
                }
            }
            let release = json!({"id":9,"tag_name":"v2.1.0","draft":false,"prerelease":false,"published_at":"2026-10-22T00:00:00Z","assets":assets});
            data.insert("/releases/latest".into(), release.clone());
            data.insert("/releases/tags/v2.1.0".into(), release);
            data.insert(
                "/git/ref/tags/v2.1.0".into(),
                json!({"object":{"type":"commit","sha":SHA}}),
            );
            data.insert(
                format!("/deployments?task={TASK}&environment={TASK}&per_page=100&page=1"),
                json!([]),
            );
            data.insert(
                "/actions/workflows/docs.yml/runs?per_page=100&page=1".into(),
                json!({"workflow_runs":[]}),
            );
            data.insert(
                "/deployments?environment=github-pages&per_page=100&page=1".into(),
                json!([]),
            );
            Self {
                data,
                ancestor: true,
                old_tree: OLD.into(),
            }
        }
        fn gate(&self) -> Result<Value> {
            evaluate(self, self, REPO, 7)
        }
        fn proof(&mut self) {
            self.data.insert(format!("/deployments?task={TASK}&environment={TASK}&per_page=100&page=1"),json!([{"id":10,"task":TASK,"environment":TASK,"sha":OLD,"creator":{"login":"github-actions[bot]"},"payload":{"docs_tree":self.old_tree,"release_run_id":6,"pages_run_id":8}}]));
            self.data.insert(
                "/deployments/10/statuses?per_page=100".into(),
                json!([{"state":"success"}]),
            );
        }
    }
    impl Api for Fixture {
        fn get(&self, path: &str) -> Result<Value> {
            self.data
                .get(path)
                .cloned()
                .ok_or_else(|| anyhow!("controlled-api-unavailable"))
        }
        fn post(&self, _: &str, _: &Value) -> Result<Value> {
            bail!("no-write-in-readonly-gate")
        }
    }
    impl GitState for Fixture {
        fn ancestor(&self, _: &str) -> Result<bool> {
            Ok(self.ancestor)
        }
        fn docs_tree(&self, sha: &str) -> Result<String> {
            Ok(if sha == SHA {
                TREE.into()
            } else {
                self.old_tree.clone()
            })
        }
    }
    #[test]
    fn first_publish_requires_complete_verified_latest_release() {
        assert_eq!(Fixture::new().gate().unwrap()["deploy"], true);
    }
    #[test]
    fn unchanged_documentation_skips_jekyll_and_deployment() {
        let mut f = Fixture::new();
        f.old_tree = TREE.into();
        f.proof();
        assert_eq!(f.gate().unwrap()["deploy"], false);
    }
    #[test]
    fn changed_docs_publishes_after_successful_proof() {
        let mut f = Fixture::new();
        f.proof();
        assert_eq!(f.gate().unwrap()["deploy"], true);
    }
    #[test]
    fn release_failure_cancelled_queued_and_wrong_workflow_rejected() {
        for (k, v) in [
            ("conclusion", json!("failure")),
            ("conclusion", json!("cancelled")),
            ("status", json!("queued")),
            ("event", json!("workflow_dispatch")),
            ("path", json!(DOCS_PATH)),
        ] {
            let mut f = Fixture::new();
            f.data.get_mut("/actions/runs/7").unwrap()[k] = v;
            assert!(f.gate().is_err());
        }
    }
    #[test]
    fn missing_postdownload_or_native_runtime_rejected() {
        for name in ["Verify published downloads", "Bundle macos-aarch64"] {
            let mut f = Fixture::new();
            let jobs = f
                .data
                .get_mut("/actions/runs/7/jobs?per_page=100&page=1")
                .unwrap()["jobs"]
                .as_array_mut()
                .unwrap();
            jobs.retain(|j| j["name"] != name);
            assert!(f.gate().is_err());
        }
    }
    #[test]
    fn skipped_real_native_step_rejected() {
        let mut f = Fixture::new();
        let jobs = f
            .data
            .get_mut("/actions/runs/7/jobs?per_page=100&page=1")
            .unwrap()["jobs"]
            .as_array_mut()
            .unwrap();
        let job = jobs
            .iter_mut()
            .find(|j| j["name"] == "Bundle macos-aarch64")
            .unwrap();
        job["steps"][0]["conclusion"] = "skipped".into();
        assert!(f.gate().is_err());
    }
    #[test]
    fn draft_prerelease_bad_tag_and_missing_assets_rejected() {
        for (key, value) in [
            ("draft", json!(true)),
            ("prerelease", json!(true)),
            ("tag_name", json!("v2.1.0-rc1")),
            ("assets", json!([])),
        ] {
            let mut f = Fixture::new();
            f.data.get_mut("/releases/latest").unwrap()[key] = value;
            assert!(f.gate().is_err());
        }
    }
    #[test]
    fn old_manual_run_cannot_roll_back_latest() {
        let mut f = Fixture::new();
        f.data.get_mut("/git/ref/tags/v2.1.0").unwrap()["object"]["sha"] = OLD.into();
        assert!(f.gate().is_err());
    }
    #[test]
    fn release_commit_must_be_main_ancestor() {
        let mut f = Fixture::new();
        f.ancestor = false;
        assert!(f.gate().is_err());
    }
    #[test]
    fn cross_repository_release_run_rejected() {
        let mut f = Fixture::new();
        f.data.get_mut("/actions/runs/7").unwrap()["repository"]["id"] = 999.into();
        assert!(f.gate().is_err());
    }
    #[test]
    fn api_failure_never_means_first_publication() {
        let mut f = Fixture::new();
        f.data
            .remove("/deployments?environment=github-pages&per_page=100&page=1");
        assert!(f.gate().is_err());
    }
    #[test]
    fn failed_forged_and_mismatched_proof_rejected() {
        for change in 0..3 {
            let mut f = Fixture::new();
            f.proof();
            match change {
                0 => {
                    f.data
                        .get_mut("/deployments/10/statuses?per_page=100")
                        .unwrap()[0]["state"] = "failure".into()
                }
                1 => {
                    f.data
                        .get_mut(&format!(
                            "/deployments?task={TASK}&environment={TASK}&per_page=100&page=1"
                        ))
                        .unwrap()[0]["creator"]["login"] = "untrusted".into()
                }
                _ => {
                    f.data
                        .get_mut(&format!(
                            "/deployments?task={TASK}&environment={TASK}&per_page=100&page=1"
                        ))
                        .unwrap()[0]["payload"]["docs_tree"] = SHA.into()
                }
            };
            assert!(f.gate().is_err());
        }
    }
    #[test]
    fn legacy_push_success_is_trusted_but_manual_head_is_not() {
        for event in ["push", "workflow_dispatch"] {
            let mut f = Fixture::new();
            f.old_tree = TREE.into();
            f.data.insert("/actions/workflows/docs.yml/runs?per_page=100&page=1".into(),json!({"workflow_runs":[{"id":8,"event":event,"path":DOCS_PATH,"status":"completed","conclusion":"success","head_sha":OLD}]}));
            f.data.insert("/actions/runs/8/jobs?per_page=100&page=1".into(),json!({"jobs":[{"name":"Deploy to GitHub Pages","status":"completed","conclusion":"success"}]}));
            assert_eq!(f.gate().unwrap()["deploy"], event != "push");
        }
    }
    #[test]
    fn existing_site_without_baseline_is_not_first_publish() {
        let mut f = Fixture::new();
        f.data.insert(
            "/deployments?environment=github-pages&per_page=100&page=1".into(),
            json!([{"id":99}]),
        );
        f.data.insert(
            "/deployments/99/statuses?per_page=100".into(),
            json!([{"state":"success"}]),
        );
        assert!(f.gate().is_err());
    }
    #[test]
    fn missing_real_public_customer_job_cannot_publish_pages() {
        for name in [
            "Public installation and online update (Linux)",
            "Public installation and online update (Windows)",
        ] {
            let mut f = Fixture::new();
            f.data
                .get_mut("/actions/runs/7/jobs?per_page=100&page=1")
                .unwrap()["jobs"]
                .as_array_mut()
                .unwrap()
                .retain(|j| j["name"] != name);
            assert!(f.gate().is_err());
        }
    }
    #[test]
    fn promotion_is_mandatory_and_its_real_business_step_cannot_be_skipped() {
        for outcome in ["missing", "failure", "skipped"] {
            let mut f = Fixture::new();
            let jobs = f
                .data
                .get_mut("/actions/runs/7/jobs?per_page=100&page=1")
                .unwrap()["jobs"]
                .as_array_mut()
                .unwrap();
            if outcome == "missing" {
                jobs.retain(|j| j["name"] != "Promote accepted release");
            } else {
                let job = jobs
                    .iter_mut()
                    .find(|j| j["name"] == "Promote accepted release")
                    .unwrap();
                job["steps"][0]["conclusion"] = outcome.into();
            }
            assert!(f.gate().is_err());
        }
    }
    #[test]
    fn annotated_tag_resolves_to_exact_release_commit() {
        let mut f = Fixture::new();
        f.data.insert(
            "/git/ref/tags/v2.1.0".into(),
            json!({"object":{"type":"tag","sha":OLD}}),
        );
        f.data.insert(
            format!("/git/tags/{OLD}"),
            json!({"object":{"type":"commit","sha":SHA}}),
        );
        assert!(f.gate().is_ok());
    }
    #[test]
    fn record_requires_actual_successful_deploy_step() {
        let repository = json!({"id":1,"full_name":REPO});
        let run = json!({"path":DOCS_PATH,"event":"workflow_run","repository":repository});
        let mut job = json!({"name":"Deploy verified documentation","steps":[{"name":"Deploy to GitHub Pages","status":"completed","conclusion":"success"}]});
        assert!(verified_pages_execution(&run, &[job.clone()], REPO, &json!(1)).is_ok());
        job["steps"][0]["conclusion"] = "skipped".into();
        assert!(verified_pages_execution(&run, &[job], REPO, &json!(1)).is_err());
    }
    #[test]
    fn summary_lag_accepts_only_fully_completed_real_steps_and_reports_it() {
        let mut f = Fixture::new();
        let jobs = f
            .data
            .get_mut("/actions/runs/7/jobs?per_page=100&page=1")
            .unwrap()["jobs"]
            .as_array_mut()
            .unwrap();
        let job = jobs
            .iter_mut()
            .find(|j| j["name"] == "Verify (full CI) / Rust (Linux)")
            .unwrap();
        job["status"] = "in_progress".into();
        job["conclusion"] = Value::Null;
        let result = f.gate().unwrap();
        assert_eq!(
            result["summary_lag_jobs"],
            json!(["Verify (full CI) / Rust (Linux)"])
        );
    }
    #[test]
    fn queued_missing_complete_failed_or_skipped_business_steps_never_use_summary_fallback() {
        for change in 0..5 {
            let mut f = Fixture::new();
            let jobs = f
                .data
                .get_mut("/actions/runs/7/jobs?per_page=100&page=1")
                .unwrap()["jobs"]
                .as_array_mut()
                .unwrap();
            let job = jobs
                .iter_mut()
                .find(|j| j["name"] == "Verify (full CI) / Rust (Linux)")
                .unwrap();
            job["status"] = "in_progress".into();
            job["conclusion"] = Value::Null;
            match change {
                0 => job["status"] = "queued".into(),
                1 => job["steps"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|s| s["name"] != "Complete job"),
                2 => job["conclusion"] = "failure".into(),
                3 => {
                    job["steps"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|s| s["name"] == "Test")
                        .unwrap()["conclusion"] = "skipped".into()
                }
                _ => job["steps"][0]["status"] = "in_progress".into(),
            }
            assert!(f.gate().is_err());
        }
    }
    #[test]
    fn record_reads_are_bounded_and_never_assume_a_skipped_deploy_passed() {
        use std::cell::Cell;
        struct Stale {
            reads: Cell<usize>,
            succeeds: bool,
        }
        impl Api for Stale {
            fn get(&self, path: &str) -> Result<Value> {
                if path == "/actions/runs/8" {
                    return Ok(
                        json!({"path":DOCS_PATH,"event":"workflow_run","repository":{"id":1,"full_name":REPO}}),
                    );
                }
                self.reads.set(self.reads.get() + 1);
                Ok(
                    json!({"jobs":[{"name":"Deploy verified documentation","steps":[{"name":"Deploy to GitHub Pages","status":"completed","conclusion":if self.succeeds&&self.reads.get()==3{"success"}else{"skipped"}}]}]}),
                )
            }
            fn post(&self, _: &str, _: &Value) -> Result<Value> {
                bail!("write-before-verification");
            }
        }
        for succeeds in [false, true] {
            let api = Stale {
                reads: Cell::new(0),
                succeeds,
            };
            let waits = Cell::new(0);
            assert_eq!(
                verify_pages_with_retry(&api, REPO, 8, &json!(1), &|| waits.set(waits.get() + 1))
                    .is_ok(),
                succeeds
            );
            assert_eq!(api.reads.get(), 3);
            assert_eq!(waits.get(), 2);
        }
    }
}
