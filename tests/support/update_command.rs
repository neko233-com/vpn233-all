// Shared executable regression: updating must work without loading live config,
// preserve script paths with spaces, run Bash syntax on Unix, and propagate failure.
fn check_update_command(executable: &str, succeeds: bool) {
    let scratch = tempfile::Builder::new()
        .prefix("vpn233 update fixture ")
        .tempdir()
        .unwrap();
    let script = scratch.path().join(if cfg!(windows) {
        "update fixture.ps1"
    } else {
        "update fixture.sh"
    });
    let content = if cfg!(windows) {
        if succeeds {
            "Write-Output 'synthetic-update-ok'\nexit 0\n"
        } else {
            "exit 7\n"
        }
    } else if succeeds {
        "set -Eeuo pipefail\nvalues=(synthetic-update-ok)\n[[ ${values[0]} == synthetic-update-ok ]]\nprintf '%s\\n' \"${values[0]}\"\n"
    } else {
        "set -Eeuo pipefail\nexit 7\n"
    };
    std::fs::write(&script, content).unwrap();
    let output = std::process::Command::new(executable)
        .arg("update")
        .env("VPN233_UPDATE_SCRIPT", &script)
        .current_dir(scratch.path())
        .output()
        .unwrap();
    assert_eq!(output.status.success(), succeeds);
    if succeeds {
        assert!(String::from_utf8_lossy(&output.stdout).contains("synthetic-update-ok"));
    }
}
