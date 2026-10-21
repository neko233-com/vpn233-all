//! Publish immutable candidate bytes first; promote metadata only after customer acceptance.
use crate::pages_gate::{
    collection, critical_steps, release_job_names, tag_commit, valid_sha, validate_release_jobs,
    Api, ApiNotFound, GitHub,
};
use anyhow::{anyhow, bail, ensure, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::Path,
    time::Duration,
};

const PROMOTION: &str = "Promote accepted release";
type Assets = BTreeMap<String, (u64, String)>;
type AssetIds = BTreeMap<String, u64>;

fn version(tag: &str) -> Result<(u64, u64, u64)> {
    ensure!(
        Regex::new(r"\Av(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\z")?.is_match(tag),
        "candidate-invalid-version"
    );
    let values = tag[1..]
        .split('.')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok((values[0], values[1], values[2]))
}
fn names() -> Vec<String> {
    let mut names = vec!["SHA256SUMS".into(), "version.txt".into()];
    for platform in [
        "linux-x86_64",
        "windows-x86_64",
        "macos-x86_64",
        "macos-aarch64",
    ] {
        for extension in ["tar.gz", "zip"] {
            names.push(format!("vpn233-rust-{platform}.{extension}"));
        }
    }
    names
}
fn assets(release: &Value) -> Result<Assets> {
    let values = release["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("candidate-assets-missing"))?;
    ensure!(values.len() == 10, "candidate-asset-count");
    let mut result = Assets::new();
    for value in values {
        let name = value["name"]
            .as_str()
            .ok_or_else(|| anyhow!("candidate-asset-name"))?;
        let size = value["size"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or_else(|| anyhow!("candidate-asset-size"))?;
        let digest = value["digest"]
            .as_str()
            .and_then(|s| s.strip_prefix("sha256:"))
            .ok_or_else(|| anyhow!("candidate-asset-digest"))?;
        ensure!(
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "candidate-asset-digest"
        );
        ensure!(
            result.insert(name.into(), (size, digest.into())).is_none(),
            "candidate-asset-duplicate"
        );
    }
    ensure!(
        names().iter().all(|name| result.contains_key(name)),
        "candidate-asset-set"
    );
    Ok(result)
}
fn id(release: &Value) -> Result<u64> {
    release["id"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or_else(|| anyhow!("candidate-release-id"))
}
fn validate_asset_ids(ids: &AssetIds) -> Result<()> {
    ensure!(
        ids.len() == 10
            && names().iter().all(|name| ids.contains_key(name))
            && ids.values().all(|value| *value > 0)
            && ids.values().copied().collect::<BTreeSet<_>>().len() == 10,
        "candidate-asset-id-set-invalid"
    );
    Ok(())
}
fn asset_ids(release: &Value) -> Result<AssetIds> {
    let values = release["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("candidate-assets-missing"))?;
    let mut ids = AssetIds::new();
    for value in values {
        let name = value["name"]
            .as_str()
            .ok_or_else(|| anyhow!("candidate-asset-name"))?;
        let asset_id = value["id"]
            .as_u64()
            .filter(|value| *value > 0)
            .ok_or_else(|| anyhow!("candidate-asset-id-missing"))?;
        ensure!(
            ids.insert(name.into(), asset_id).is_none(),
            "candidate-asset-name-duplicated"
        );
    }
    validate_asset_ids(&ids)?;
    Ok(ids)
}
fn phase(release: &Value, tag: &str, draft: bool, prerelease: bool) -> Result<()> {
    ensure!(
        release["tag_name"] == tag
            && release["draft"] == draft
            && release["prerelease"] == prerelease,
        "candidate-release-phase"
    );
    if !draft {
        ensure!(
            release["published_at"]
                .as_str()
                .is_some_and(|s| !s.is_empty()),
            "candidate-publication-unconfirmed"
        );
    }
    Ok(())
}
fn validate_run(api: &impl Api, repo: &str, tag: &str, sha: &str, run_id: u64) -> Result<()> {
    version(tag)?;
    ensure!(valid_sha(sha) && run_id > 0, "candidate-invalid-source");
    let repository = api.get("")?;
    ensure!(
        repository["full_name"] == repo
            && repository["private"] == false
            && repository["fork"] == false
            && repository["default_branch"] == "main",
        "candidate-repository-identity"
    );
    let run = api.get(&format!("/actions/runs/{run_id}"))?;
    ensure!(
        run["repository"]["id"] == repository["id"]
            && repository["id"].as_u64().is_some_and(|n| n > 0)
            && run["repository"]["full_name"] == repo
            && run["path"] == ".github/workflows/release.yml"
            && run["event"] == "push"
            && run["head_sha"] == sha
            && run["head_branch"] == tag
            && run["status"] == "in_progress"
            && run["conclusion"].is_null(),
        "candidate-official-active-release-run-required"
    );
    ensure!(tag_commit(api, tag)? == sha, "candidate-tag-commit-differs");
    Ok(())
}
fn ensure_new(api: &impl Api, repo: &str, tag: &str, sha: &str, run_id: u64) -> Result<()> {
    validate_run(api, repo, tag, sha, run_id)?;
    match api.get(&format!("/releases/tags/{tag}")) {
        Err(error) if error.downcast_ref::<ApiNotFound>().is_some() => Ok(()),
        _ => bail!("candidate-existing-release-or-unavailable-api"),
    }
}
fn candidate_proof(
    repo: &str,
    tag: &str,
    sha: &str,
    run_id: u64,
    release: &Value,
) -> Result<Value> {
    Ok(
        json!({"schema":2,"repository":repo,"tag":tag,"commit":sha,"release_run_id":run_id,"release_id":id(release)?,"assets":assets(release)?,"asset_ids":asset_ids(release)?}),
    )
}
fn publish(
    api: &impl Api,
    repo: &str,
    tag: &str,
    sha: &str,
    run_id: u64,
    expected: &Assets,
) -> Result<Value> {
    validate_run(api, repo, tag, sha, run_id)?;
    let draft = api.get(&format!("/releases/tags/{tag}"))?;
    phase(&draft, tag, true, true)?;
    ensure!(
        assets(&draft)? == *expected,
        "candidate-draft-assets-differ"
    );
    let release_id = id(&draft)?;
    let expected_ids = asset_ids(&draft)?;
    let changed = api.patch(
        &format!("/releases/{release_id}"),
        &json!({"draft":false,"prerelease":true,"make_latest":"false"}),
    )?;
    phase(&changed, tag, false, true)?;
    ensure!(
        id(&changed)? == release_id
            && assets(&changed)? == *expected
            && asset_ids(&changed)? == expected_ids,
        "candidate-publication-response-differs"
    );
    let published = api.get(&format!("/releases/tags/{tag}"))?;
    phase(&published, tag, false, true)?;
    ensure!(
        id(&published)? == release_id
            && assets(&published)? == *expected
            && asset_ids(&published)? == expected_ids
            && tag_commit(api, tag)? == sha,
        "candidate-publication-readback-differs"
    );
    candidate_proof(repo, tag, sha, run_id, &published)
}
fn validate_proof(
    proof: &Value,
    repo: &str,
    tag: &str,
    sha: &str,
    run_id: u64,
) -> Result<(u64, Assets, AssetIds)> {
    ensure!(
        proof["schema"] == 2
            && proof["repository"] == repo
            && proof["tag"] == tag
            && proof["commit"] == sha
            && proof["release_run_id"] == run_id,
        "candidate-proof-context-differs"
    );
    let release_id = proof["release_id"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or_else(|| anyhow!("candidate-proof-id"))?;
    let expected: Assets = serde_json::from_value(proof["assets"].clone())?;
    let expected_ids: AssetIds = serde_json::from_value(proof["asset_ids"].clone())?;
    validate_asset_ids(&expected_ids)?;
    // Validate the saved fingerprint with the same exact asset-set rules as the API.
    let values: Vec<_> = expected.iter().map(|(name, (size, digest))| json!({"name":name,"size":size,"digest":format!("sha256:{digest}")})).collect();
    ensure!(
        assets(&json!({"assets":values}))? == expected,
        "candidate-proof-assets"
    );
    Ok((release_id, expected, expected_ids))
}
fn prerequisite_snapshot(api: &impl Api, run_id: u64) -> Result<bool> {
    let jobs = collection(api, &format!("/actions/runs/{run_id}/jobs"), Some("jobs"))?;
    let promotion: Vec<_> = jobs.iter().filter(|j| j["name"] == PROMOTION).collect();
    ensure!(
        promotion.len() == 1
            && promotion[0]["status"] == "in_progress"
            && promotion[0].get("conclusion") == Some(&Value::Null),
        "candidate-active-promotion-job-required"
    );
    let previous: Vec<_> = jobs
        .into_iter()
        .filter(|j| j["name"] != PROMOTION)
        .collect();
    let expected = release_job_names(false);
    ensure!(
        previous.len() == expected.len(),
        "candidate-prerequisite-job-count"
    );
    let mut pending = false;
    for name in expected {
        let matches: Vec<_> = previous.iter().filter(|job| job["name"] == name).collect();
        ensure!(
            matches.len() == 1,
            "candidate-prerequisite-job-missing-or-duplicated"
        );
        let job = matches[0];
        match job["status"].as_str() {
            Some("completed") => ensure!(
                job["conclusion"] == "success",
                "candidate-prerequisite-job-terminal-failure"
            ),
            Some("queued" | "in_progress") => {
                ensure!(
                    job.get("conclusion") == Some(&Value::Null),
                    "candidate-prerequisite-status-conflict"
                );
                pending = true;
            }
            _ => bail!("candidate-prerequisite-status-invalid"),
        }
        // Only a job-summary lag may be retried. Missing, unfinished or skipped
        // business steps remain an immediate rejection even for a pending job.
        let steps = job["steps"]
            .as_array()
            .ok_or_else(|| anyhow!("candidate-prerequisite-steps-missing"))?;
        for step in critical_steps(name) {
            let matches: Vec<_> = steps
                .iter()
                .filter(|value| value["name"] == *step)
                .collect();
            ensure!(
                matches.len() == 1
                    && matches[0]["status"] == "completed"
                    && matches[0]["conclusion"] == "success",
                "candidate-prerequisite-critical-step-not-successful"
            );
        }
    }
    if !pending {
        validate_release_jobs(&previous, false)?;
    }
    Ok(!pending)
}
fn prerequisites(api: &impl Api, run_id: u64, wait: &dyn Fn()) -> Result<()> {
    for attempt in 0..3 {
        if prerequisite_snapshot(api, run_id)? {
            return Ok(());
        }
        if attempt < 2 {
            wait();
        }
    }
    bail!("candidate-prerequisite-jobs-not-complete-after-bounded-reads")
}
fn confirm_stable(
    api: &impl Api,
    tag: &str,
    sha: &str,
    release_id: u64,
    expected: &Assets,
    expected_ids: &AssetIds,
) -> Result<()> {
    let published = api.get(&format!("/releases/tags/{tag}"))?;
    let latest = api.get("/releases/latest")?;
    for release in [&published, &latest] {
        phase(release, tag, false, false)?;
        ensure!(
            id(release)? == release_id
                && assets(release)? == *expected
                && asset_ids(release)? == *expected_ids,
            "candidate-stable-readback-differs"
        );
    }
    ensure!(
        tag_commit(api, tag)? == sha,
        "candidate-promoted-tag-differs"
    );
    Ok(())
}
fn promote(
    api: &impl Api,
    repo: &str,
    tag: &str,
    sha: &str,
    run_id: u64,
    proof: &Value,
    wait: &dyn Fn(),
) -> Result<Value> {
    validate_run(api, repo, tag, sha, run_id)?;
    prerequisites(api, run_id, wait)?;
    let (release_id, expected, expected_ids) = validate_proof(proof, repo, tag, sha, run_id)?;
    let release = api.get(&format!("/releases/tags/{tag}"))?;
    phase(&release, tag, false, true)?;
    ensure!(
        id(&release)? == release_id
            && assets(&release)? == expected
            && asset_ids(&release)? == expected_ids,
        "candidate-assets-changed-before-promotion"
    );
    match api.get("/releases/latest") {
        Ok(latest) => {
            ensure!(
                latest["draft"] == false && latest["prerelease"] == false,
                "candidate-previous-latest-invalid"
            );
            ensure!(
                version(
                    latest["tag_name"]
                        .as_str()
                        .ok_or_else(|| anyhow!("candidate-latest-tag"))?
                )? < version(tag)?,
                "candidate-newer-or-equal-stable-release-exists"
            );
        }
        Err(error) if error.downcast_ref::<ApiNotFound>().is_some() => {}
        Err(_) => bail!("candidate-latest-api-unavailable"),
    }
    let updated = api.patch(
        &format!("/releases/{release_id}"),
        &json!({"prerelease":false,"make_latest":"true"}),
    )?;
    phase(&updated, tag, false, false)?;
    ensure!(
        id(&updated)? == release_id
            && assets(&updated)? == expected
            && asset_ids(&updated)? == expected_ids,
        "candidate-promotion-response-differs"
    );
    for attempt in 0..3 {
        if confirm_stable(api, tag, sha, release_id, &expected, &expected_ids).is_ok() {
            return Ok(
                json!({"promoted":true,"release_id":release_id,"release_run_id":run_id,"commit":sha,"tag":tag,"assets_verified":10}),
            );
        }
        if attempt < 2 {
            wait();
        }
    }
    bail!("candidate-stable-readback-not-confirmed")
}
pub fn run(args: &[String]) -> Result<()> {
    let operation = args.first().map(String::as_str).unwrap_or("");
    ensure!(
        ["check-new", "publish", "promote"].contains(&operation),
        "candidate-invalid-operation"
    );
    let mut options = BTreeMap::new();
    for pair in args[1..].chunks(2) {
        ensure!(
            pair.len() == 2
                && ["--repo", "--tag", "--commit", "--run-id", "--assets", "--proof"]
                    .contains(&pair[0].as_str())
                && options.insert(pair[0].as_str(), pair[1].as_str()).is_none(),
            "candidate-invalid-options"
        );
    }
    let get = |key| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| anyhow!("candidate-required-option-missing"))
    };
    let (repo, tag, sha) = (get("--repo")?, get("--tag")?, get("--commit")?);
    let run_id = get("--run-id")?.parse::<u64>()?;
    ensure!(
        env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && env::var("GITHUB_REPOSITORY").as_deref() == Ok(repo)
            && env::var("GITHUB_RUN_ID")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                == Some(run_id)
            && env::var("GITHUB_SHA").as_deref() == Ok(sha)
            && env::var("GITHUB_REF_NAME").as_deref() == Ok(tag),
        "candidate-only-current-official-actions-run"
    );
    let api = GitHub::new(repo)?;
    let result = match operation {
        "check-new" => {
            ensure_new(&api, repo, tag, sha, run_id)?;
            json!({"new_candidate":true})
        }
        "publish" => {
            let directory = Path::new(get("--assets")?);
            crate::verify_release::run(&[
                directory.to_string_lossy().into_owned(),
                tag.trim_start_matches('v').into(),
            ])?;
            let mut expected = Assets::new();
            for name in names() {
                let path = directory.join(&name);
                let meta = fs::symlink_metadata(&path)?;
                ensure!(
                    meta.is_file() && !meta.file_type().is_symlink(),
                    "candidate-local-asset-not-regular"
                );
                expected.insert(name, (meta.len(), crate::support::sha256_file(&path)?));
            }
            let proof = publish(&api, repo, tag, sha, run_id, &expected)?;
            fs::write(get("--proof")?, serde_json::to_vec(&proof)?)?;
            json!({"candidate_published":true,"assets_verified":10})
        }
        "promote" => {
            let proof_path = Path::new(get("--proof")?);
            ensure!(
                fs::metadata(proof_path)?.len() <= 65536,
                "candidate-proof-too-large"
            );
            let proof: Value = serde_json::from_slice(&fs::read(proof_path)?)?;
            promote(&api, repo, tag, sha, run_id, &proof, &|| {
                std::thread::sleep(Duration::from_secs(2))
            })?
        }
        _ => unreachable!(),
    };
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages_gate::{critical_steps, release_job_names};
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
    };
    const REPO: &str = "fixture/project";
    const TAG: &str = "v2.1.1";
    const SHA: &str = "1111111111111111111111111111111111111111";
    struct Fixture {
        data: RefCell<BTreeMap<String, Value>>,
        changes: RefCell<Vec<Value>>,
        stale_latest: bool,
        unavailable: bool,
    }
    impl Fixture {
        fn new() -> Self {
            let repo = json!({"id":1,"full_name":REPO,"private":false,"fork":false,"default_branch":"main"});
            let files: Vec<_> = names().into_iter().enumerate().map(|(index,name)| json!({"id":index as u64+1,"name":name,"size":100,"digest":format!("sha256:{}","a".repeat(64))})).collect();
            let release = json!({"id":9,"tag_name":TAG,"draft":false,"prerelease":true,"published_at":"2026-10-22T00:00:00Z","assets":files});
            let jobs: Vec<_> = release_job_names(false)
                .into_iter()
                .map(|name| {
                    let steps: Vec<_> = critical_steps(name)
                        .iter()
                        .map(|s| json!({"name":s,"status":"completed","conclusion":"success"}))
                        .collect();
                    json!({"name":name,"status":"completed","conclusion":"success","steps":steps})
                })
                .chain([json!({"name":PROMOTION,"status":"in_progress","conclusion":null})])
                .collect();
            let mut data = BTreeMap::new();
            data.insert("".into(), repo.clone());
            data.insert("/actions/runs/7".into(), json!({"id":7,"repository":repo,"event":"push","path":".github/workflows/release.yml","head_sha":SHA,"head_branch":TAG,"status":"in_progress","conclusion":null}));
            data.insert(
                "/actions/runs/7/jobs?per_page=100&page=1".into(),
                json!({"jobs":jobs}),
            );
            data.insert(
                format!("/git/ref/tags/{TAG}"),
                json!({"object":{"type":"commit","sha":SHA}}),
            );
            data.insert(format!("/releases/tags/{TAG}"), release.clone());
            let mut previous = release;
            previous["id"] = 8.into();
            previous["tag_name"] = "v2.1.0".into();
            previous["prerelease"] = false.into();
            data.insert("/releases/latest".into(), previous);
            Self {
                data: RefCell::new(data),
                changes: RefCell::new(Vec::new()),
                stale_latest: false,
                unavailable: false,
            }
        }
        fn proof(&self) -> Value {
            candidate_proof(
                REPO,
                TAG,
                SHA,
                7,
                &self.get(&format!("/releases/tags/{TAG}")).unwrap(),
            )
            .unwrap()
        }
        fn promote(&self) -> Result<Value> {
            promote(self, REPO, TAG, SHA, 7, &self.proof(), &|| {})
        }
        fn edit_jobs(&self, edit: impl FnOnce(&mut Vec<Value>)) {
            let mut data = self.data.borrow_mut();
            edit(
                data.get_mut("/actions/runs/7/jobs?per_page=100&page=1")
                    .unwrap()["jobs"]
                    .as_array_mut()
                    .unwrap(),
            );
        }
    }
    impl Api for Fixture {
        fn get(&self, path: &str) -> Result<Value> {
            if self.unavailable {
                bail!("controlled-private-marker-api-unavailable");
            }
            self.data
                .borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| ApiNotFound.into())
        }
        fn post(&self, _: &str, _: &Value) -> Result<Value> {
            bail!("unexpected-create");
        }
        fn patch(&self, path: &str, body: &Value) -> Result<Value> {
            ensure!(path == "/releases/9", "unexpected-release-mutation");
            self.changes.borrow_mut().push(body.clone());
            let mut data = self.data.borrow_mut();
            let release = data.get_mut(&format!("/releases/tags/{TAG}")).unwrap();
            for key in ["draft", "prerelease"] {
                if let Some(value) = body.get(key) {
                    release[key] = value.clone();
                }
            }
            let result = release.clone();
            if body["make_latest"] == "true" && !self.stale_latest {
                data.insert("/releases/latest".into(), result.clone());
            }
            Ok(result)
        }
    }
    struct JobSnapshots {
        fixture: Fixture,
        snapshots: RefCell<VecDeque<Value>>,
        reads: Cell<usize>,
    }
    impl JobSnapshots {
        fn new(edit: impl FnOnce(&mut Value)) -> Self {
            let fixture = Fixture::new();
            let ready = fixture
                .get("/actions/runs/7/jobs?per_page=100&page=1")
                .unwrap();
            let mut first = ready.clone();
            edit(&mut first);
            Self {
                fixture,
                snapshots: RefCell::new(VecDeque::from([first, ready])),
                reads: Cell::new(0),
            }
        }
        fn pending() -> Self {
            Self::new(|snapshot| {
                let job = snapshot["jobs"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|job| job["name"] == "Public installation and online update (Windows)")
                    .unwrap();
                job["status"] = "in_progress".into();
                job["conclusion"] = Value::Null;
            })
        }
        fn promote(&self, wait: &dyn Fn()) -> Result<Value> {
            promote(self, REPO, TAG, SHA, 7, &self.fixture.proof(), wait)
        }
    }
    impl Api for JobSnapshots {
        fn get(&self, path: &str) -> Result<Value> {
            if path == "/actions/runs/7/jobs?per_page=100&page=1" {
                self.reads.set(self.reads.get() + 1);
                let mut snapshots = self.snapshots.borrow_mut();
                return Ok(if snapshots.len() > 1 {
                    snapshots.pop_front().unwrap()
                } else {
                    snapshots[0].clone()
                });
            }
            self.fixture.get(path)
        }
        fn post(&self, path: &str, body: &Value) -> Result<Value> {
            self.fixture.post(path, body)
        }
        fn patch(&self, path: &str, body: &Value) -> Result<Value> {
            self.fixture.patch(path, body)
        }
    }
    #[test]
    fn transient_summary_lag_rechecks_reads_before_a_single_promotion() {
        let api = JobSnapshots::pending();
        let waits = Cell::new(0);
        assert_eq!(
            api.promote(&|| {
                assert!(api.fixture.changes.borrow().is_empty());
                waits.set(waits.get() + 1);
            })
            .unwrap()["promoted"],
            true
        );
        assert_eq!(api.reads.get(), 2);
        assert_eq!(waits.get(), 1);
        assert_eq!(api.fixture.changes.borrow().len(), 1);
    }
    #[test]
    fn summary_lag_followed_by_terminal_failure_does_not_retry_or_mutate_again() {
        let api = JobSnapshots::pending();
        let waits = Cell::new(0);
        let mut failed = api.snapshots.borrow().back().unwrap().clone();
        let job = failed["jobs"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|job| job["name"] == "Public installation and online update (Windows)")
            .unwrap();
        job["conclusion"] = "failure".into();
        api.snapshots.borrow_mut().insert(1, failed);
        assert!(api.promote(&|| waits.set(waits.get() + 1)).is_err());
        assert_eq!(api.reads.get(), 2);
        assert_eq!(waits.get(), 1);
        assert!(api.fixture.changes.borrow().is_empty());
    }
    #[test]
    fn persistent_summary_lag_stops_after_three_reads_and_two_waits() {
        let api = JobSnapshots::pending();
        api.snapshots.borrow_mut().pop_back();
        let waits = Cell::new(0);
        assert!(api.promote(&|| waits.set(waits.get() + 1)).is_err());
        assert_eq!(api.reads.get(), 3);
        assert_eq!(waits.get(), 2);
        assert!(api.fixture.changes.borrow().is_empty());
    }
    #[test]
    fn missing_duplicate_terminal_or_bad_critical_steps_fail_before_waiting() {
        for outcome in [
            "missing",
            "duplicate",
            "failure",
            "skipped",
            "cancelled",
            "timed_out",
            "status-conflict",
            "conclusion-missing",
            "critical-missing",
            "critical-skipped",
            "critical-failure",
            "critical-pending",
        ] {
            let api = JobSnapshots::new(|snapshot| {
                let jobs = snapshot["jobs"].as_array_mut().unwrap();
                let index = jobs
                    .iter()
                    .position(|job| {
                        job["name"] == "Public installation and online update (Windows)"
                    })
                    .unwrap();
                match outcome {
                    "missing" => {
                        jobs.remove(index);
                    }
                    "duplicate" => jobs.push(jobs[index].clone()),
                    "status-conflict" => {
                        jobs[index]["status"] = "in_progress".into();
                    }
                    "conclusion-missing" => {
                        jobs[index]["status"] = "in_progress".into();
                        jobs[index].as_object_mut().unwrap().remove("conclusion");
                    }
                    "critical-missing" => jobs[index]["steps"] = json!([]),
                    "critical-skipped" => jobs[index]["steps"][0]["conclusion"] = "skipped".into(),
                    "critical-failure" => jobs[index]["steps"][0]["conclusion"] = "failure".into(),
                    "critical-pending" => {
                        jobs[index]["status"] = "in_progress".into();
                        jobs[index]["conclusion"] = Value::Null;
                        jobs[index]["steps"][0]["status"] = "in_progress".into();
                        jobs[index]["steps"][0]["conclusion"] = Value::Null;
                    }
                    _ => jobs[index]["conclusion"] = outcome.into(),
                }
            });
            let waits = Cell::new(0);
            assert!(api.promote(&|| waits.set(waits.get() + 1)).is_err());
            assert_eq!(api.reads.get(), 1);
            assert_eq!(waits.get(), 0);
            assert!(api.fixture.changes.borrow().is_empty());
        }
    }
    #[test]
    fn accepted_candidate_promotes_metadata_once_and_reads_back_exact_assets() {
        let f = Fixture::new();
        let before = f.proof();
        assert_eq!(f.promote().unwrap()["promoted"], true);
        assert_eq!(
            f.changes.borrow().as_slice(),
            &[json!({"prerelease":false,"make_latest":"true"})]
        );
        assert_eq!(before["assets"], f.proof()["assets"]);
        assert_eq!(before["asset_ids"], f.proof()["asset_ids"]);
    }
    #[test]
    fn missing_failed_skipped_queued_or_duplicate_customer_job_cannot_mutate_release() {
        for outcome in ["missing", "failure", "skipped", "queued", "duplicate"] {
            for platform in ["Linux", "Windows"] {
                let f = Fixture::new();
                let name = format!("Public installation and online update ({platform})");
                f.edit_jobs(|jobs| {
                    let index = jobs.iter().position(|j| j["name"] == name).unwrap();
                    match outcome {
                        "missing" => {
                            jobs.remove(index);
                        }
                        "duplicate" => jobs.push(jobs[index].clone()),
                        "queued" => {
                            jobs[index]["status"] = "queued".into();
                            jobs[index]["conclusion"] = Value::Null;
                        }
                        _ => jobs[index]["conclusion"] = outcome.into(),
                    }
                });
                assert!(f.promote().is_err());
                assert!(f.changes.borrow().is_empty());
            }
        }
    }
    #[test]
    fn skipped_customer_business_step_and_unknown_extra_job_cannot_promote() {
        for extra in [false, true] {
            let f = Fixture::new();
            f.edit_jobs(|jobs|if extra { jobs.push(json!({"name":"unverified extra","status":"completed","conclusion":"success"})); } else { let j=jobs.iter_mut().find(|j|j["name"]=="Public installation and online update (Windows)").unwrap();j["steps"][0]["conclusion"]="skipped".into(); });
            assert!(f.promote().is_err());
            assert!(f.changes.borrow().is_empty());
        }
    }
    #[test]
    fn stale_commit_cross_repository_failed_run_and_wrong_workflow_are_rejected() {
        for (field, value) in [
            ("head_sha", json!("2".repeat(40))),
            ("path", json!(".github/workflows/ci.yml")),
            ("conclusion", json!("failure")),
            ("event", json!("workflow_dispatch")),
            ("head_branch", json!("v2.1.2")),
        ] {
            let f = Fixture::new();
            f.data.borrow_mut().get_mut("/actions/runs/7").unwrap()[field] = value;
            assert!(f.promote().is_err());
            assert!(f.changes.borrow().is_empty());
        }
        let f = Fixture::new();
        f.data.borrow_mut().get_mut("/actions/runs/7").unwrap()["repository"]["id"] = 2.into();
        assert!(f.promote().is_err());
        assert!(f.changes.borrow().is_empty());
    }
    #[test]
    fn saved_publication_proof_detects_asset_size_digest_or_release_identity_changes() {
        for field in ["size", "digest", "id"] {
            let f = Fixture::new();
            let proof = f.proof();
            let mut data = f.data.borrow_mut();
            let release = data.get_mut(&format!("/releases/tags/{TAG}")).unwrap();
            match field {
                "size" => release["assets"][0][field] = 101.into(),
                "digest" => {
                    release["assets"][0][field] = format!("sha256:{}", "b".repeat(64)).into()
                }
                _ => release[field] = 10.into(),
            };
            drop(data);
            assert!(promote(&f, REPO, TAG, SHA, 7, &proof, &|| {}).is_err());
            assert!(f.changes.borrow().is_empty());
        }
    }
    #[test]
    fn newer_or_equal_stable_version_never_rolls_back_latest() {
        for tag in ["v2.1.1", "v2.2.0", "v3.0.0"] {
            let f = Fixture::new();
            f.data.borrow_mut().get_mut("/releases/latest").unwrap()["tag_name"] = tag.into();
            assert!(f.promote().is_err());
            assert!(f.changes.borrow().is_empty());
        }
    }
    #[test]
    fn missing_prior_stable_is_distinct_from_api_outage() {
        let f = Fixture::new();
        f.data.borrow_mut().remove("/releases/latest");
        assert!(f.promote().is_ok());
        let mut f = Fixture::new();
        f.unavailable = true;
        assert!(promote(&f, REPO, TAG, SHA, 7, &json!({}), &|| {}).is_err());
        assert!(f.changes.borrow().is_empty());
    }
    #[test]
    fn candidate_publication_is_a_draft_transition_and_never_sets_latest() {
        let f = Fixture::new();
        f.data
            .borrow_mut()
            .get_mut(&format!("/releases/tags/{TAG}"))
            .unwrap()["draft"] = true.into();
        let expected = assets(&f.get(&format!("/releases/tags/{TAG}")).unwrap()).unwrap();
        let proof = publish(&f, REPO, TAG, SHA, 7, &expected).unwrap();
        assert_eq!(proof["assets"], f.proof()["assets"]);
        assert_eq!(proof["asset_ids"], f.proof()["asset_ids"]);
        assert_eq!(
            f.changes.borrow().as_slice(),
            &[json!({"draft":false,"prerelease":true,"make_latest":"false"})]
        );
        assert_eq!(f.get("/releases/latest").unwrap()["tag_name"], "v2.1.0");
    }
    #[test]
    fn existing_published_assets_and_wrong_draft_bytes_are_never_overwritten() {
        let f = Fixture::new();
        assert!(ensure_new(&f, REPO, TAG, SHA, 7).is_err());
        assert!(f.changes.borrow().is_empty());
        let f = Fixture::new();
        let expected = assets(&f.get(&format!("/releases/tags/{TAG}")).unwrap()).unwrap();
        f.data
            .borrow_mut()
            .get_mut(&format!("/releases/tags/{TAG}"))
            .unwrap()["draft"] = true.into();
        f.data
            .borrow_mut()
            .get_mut(&format!("/releases/tags/{TAG}"))
            .unwrap()["assets"][0]["size"] = 101.into();
        assert!(publish(&f, REPO, TAG, SHA, 7, &expected).is_err());
        assert!(f.changes.borrow().is_empty());
    }
    #[test]
    fn stable_readback_mismatch_fails_closed_with_one_patch_and_two_waits() {
        let mut f = Fixture::new();
        f.stale_latest = true;
        let waits = std::cell::Cell::new(0);
        assert!(promote(&f, REPO, TAG, SHA, 7, &f.proof(), &|| waits
            .set(waits.get() + 1))
        .is_err());
        assert_eq!(waits.get(), 2);
        assert_eq!(f.changes.borrow().len(), 1);
    }
    #[test]
    fn malformed_proof_missing_extra_or_duplicate_assets_are_rejected() {
        let f = Fixture::new();
        for field in ["commit", "tag", "release_run_id", "release_id"] {
            let mut proof = f.proof();
            proof[field] = json!("private-marker");
            assert!(validate_proof(&proof, REPO, TAG, SHA, 7).is_err());
        }
        for mode in ["missing", "extra", "duplicate", "digest"] {
            let mut release = f.get(&format!("/releases/tags/{TAG}")).unwrap();
            let files = release["assets"].as_array_mut().unwrap();
            match mode {
                "missing" => {
                    files.pop();
                }
                "extra" => files.push(json!({"name":"extra"})),
                "duplicate" => files[0] = files[1].clone(),
                _ => files[0]["digest"] = "private-marker".into(),
            };
            assert!(assets(&release).is_err());
        }
    }
    #[test]
    fn identical_bytes_under_a_replaced_asset_id_are_rejected_before_promotion() {
        let fixture = Fixture::new();
        let proof = fixture.proof();
        fixture
            .data
            .borrow_mut()
            .get_mut(&format!("/releases/tags/{TAG}"))
            .unwrap()["assets"][0]["id"] = 1001.into();
        assert!(promote(&fixture, REPO, TAG, SHA, 7, &proof, &|| {}).is_err());
        assert!(fixture.changes.borrow().is_empty());
    }
    #[test]
    fn asset_ids_must_be_present_positive_and_unique() {
        for invalid in [Value::Null, json!(0), json!(-1), json!("1"), json!(2)] {
            let fixture = Fixture::new();
            let mut release = fixture.get(&format!("/releases/tags/{TAG}")).unwrap();
            release["assets"][0]["id"] = invalid;
            assert!(asset_ids(&release).is_err());
        }
        let fixture = Fixture::new();
        let mut release = fixture.get(&format!("/releases/tags/{TAG}")).unwrap();
        release["assets"][0].as_object_mut().unwrap().remove("id");
        assert!(asset_ids(&release).is_err());
    }
    #[test]
    fn publication_proof_requires_the_complete_unique_asset_id_map() {
        for invalid in [
            "missing-map",
            "missing-name",
            "extra-name",
            "duplicate-id",
            "zero-id",
        ] {
            let fixture = Fixture::new();
            let mut proof = fixture.proof();
            match invalid {
                "missing-map" => {
                    proof.as_object_mut().unwrap().remove("asset_ids");
                }
                "missing-name" => {
                    proof["asset_ids"]
                        .as_object_mut()
                        .unwrap()
                        .remove("version.txt");
                }
                "extra-name" => proof["asset_ids"]["unexpected"] = 1001.into(),
                "duplicate-id" => {
                    proof["asset_ids"]["version.txt"] = proof["asset_ids"]["SHA256SUMS"].clone()
                }
                _ => proof["asset_ids"]["version.txt"] = 0.into(),
            }
            assert!(validate_proof(&proof, REPO, TAG, SHA, 7).is_err());
        }
    }
    struct ReplacedIdResponse {
        fixture: Fixture,
        replace_patch: bool,
    }
    impl Api for ReplacedIdResponse {
        fn get(&self, path: &str) -> Result<Value> {
            let mut result = self.fixture.get(path)?;
            if !self.replace_patch
                && !self.fixture.changes.borrow().is_empty()
                && (path == format!("/releases/tags/{TAG}") || path == "/releases/latest")
            {
                result["assets"][0]["id"] = 1001.into();
            }
            Ok(result)
        }
        fn post(&self, path: &str, body: &Value) -> Result<Value> {
            self.fixture.post(path, body)
        }
        fn patch(&self, path: &str, body: &Value) -> Result<Value> {
            let mut result = self.fixture.patch(path, body)?;
            if self.replace_patch {
                result["assets"][0]["id"] = 1001.into();
            }
            Ok(result)
        }
    }
    #[test]
    fn draft_asset_ids_are_anchored_before_publication_and_checked_in_both_responses() {
        for replace_patch in [false, true] {
            let api = ReplacedIdResponse {
                fixture: Fixture::new(),
                replace_patch,
            };
            api.fixture
                .data
                .borrow_mut()
                .get_mut(&format!("/releases/tags/{TAG}"))
                .unwrap()["draft"] = true.into();
            let expected =
                assets(&api.fixture.get(&format!("/releases/tags/{TAG}")).unwrap()).unwrap();
            assert!(publish(&api, REPO, TAG, SHA, 7, &expected).is_err());
            assert_eq!(api.fixture.changes.borrow().len(), 1);
        }
    }
    #[test]
    fn promotion_response_and_stable_readbacks_reject_replaced_asset_ids() {
        for replace_patch in [false, true] {
            let api = ReplacedIdResponse {
                fixture: Fixture::new(),
                replace_patch,
            };
            let proof = api.fixture.proof();
            assert!(promote(&api, REPO, TAG, SHA, 7, &proof, &|| {}).is_err());
            assert_eq!(api.fixture.changes.borrow().len(), 1);
        }
        let fixture = Fixture::new();
        let proof = fixture.proof();
        assert!(fixture.promote().is_ok());
        fixture
            .data
            .borrow_mut()
            .get_mut("/releases/latest")
            .unwrap()["assets"][0]["id"] = 1001.into();
        let (release_id, expected, ids) = validate_proof(&proof, REPO, TAG, SHA, 7).unwrap();
        assert!(confirm_stable(&fixture, TAG, SHA, release_id, &expected, &ids).is_err());
    }
}
