//! Structural checks for the shipped one-line install script + README one-liner.
//! These tests read the real files from the repo (not re-implementations).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn install_script_path() -> PathBuf {
    repo_root().join("scripts/install.sh")
}

fn read_install_script() -> String {
    let path = install_script_path();
    assert!(
        path.is_file(),
        "install script must exist at {}",
        path.display()
    );
    let body = fs::read_to_string(&path).expect("read install.sh");
    assert!(
        body.trim().len() > 200,
        "install.sh must not be empty/stub"
    );
    body
}

#[test]
fn install_script_is_shell_with_shebang() {
    let body = read_install_script();
    let first = body.lines().next().unwrap_or("");
    assert!(
        first.starts_with("#!") && (first.contains("bash") || first.contains("sh")),
        "expected bash/sh shebang, got: {first}"
    );
    assert!(
        body.contains("set -euo pipefail") || body.contains("set -e"),
        "script should use strict mode"
    );
}

#[test]
fn install_script_automates_full_install_phases() {
    let body = read_install_script();
    let phases: &[(&str, &[&str])] = &[
        (
            "deps",
            &["apt-get", "build-essential", "install_deps"],
        ),
        ("rust", &["rustup", "ensure_rust", "cargo"]),
        (
            "source_or_clone",
            &["git clone", "obtain_source", "SMOS_REPO"],
        ),
        (
            "build",
            &["cargo build --release", "build_binary"],
        ),
        (
            "place_binary_and_static",
            &["install_files", "static", "SMOS_PREFIX"],
        ),
        (
            "service_or_run",
            &["systemd", "install_systemd", "systemctl"],
        ),
    ];
    for (name, needles) in phases {
        let ok = needles.iter().any(|n| body.contains(n));
        assert!(
            ok,
            "install.sh missing phase '{name}' (looked for one of {needles:?})"
        );
    }
}

#[test]
fn install_script_documents_github_raw_usage() {
    let body = read_install_script();
    assert!(
        body.contains("raw.githubusercontent.com/vantanminh/SMOS")
            || body.contains("curl -fsSL"),
        "script should document curl|bash / raw GitHub usage"
    );
    assert!(
        body.contains("vantanminh/SMOS"),
        "script should reference this GitHub repo"
    );
}

#[test]
fn readme_documents_curl_bash_oneliner() {
    let readme = fs::read_to_string(repo_root().join("README.md")).expect("README");
    assert!(
        readme.contains("curl -fsSL https://raw.githubusercontent.com/vantanminh/SMOS/main/scripts/install.sh"),
        "README must document the exact raw GitHub one-liner URL"
    );
    assert!(
        readme.contains("| bash") || readme.contains("|bash"),
        "README one-liner must pipe to bash"
    );
    // URL path must match in-repo script
    assert!(
        install_script_path().ends_with("scripts/install.sh"),
        "script path must stay scripts/install.sh for raw URL stability"
    );
}

#[test]
fn bash_syntax_check_when_available() {
    let path = install_script_path();
    // Try bash, then sh -n (sh -n is weaker but better than nothing)
    let candidates = ["bash", "sh"];
    let mut ran = false;
    for shell in candidates {
        let which = Command::new(shell).arg("-c").arg("echo ok").output();
        if which.map(|o| o.status.success()).unwrap_or(false) {
            let out = Command::new(shell)
                .arg("-n")
                .arg(&path)
                .output()
                .unwrap_or_else(|e| panic!("failed to spawn {shell}: {e}"));
            assert!(
                out.status.success(),
                "{shell} -n failed:\nstdout={}\nstderr={}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            ran = true;
            break;
        }
    }
    if !ran {
        // Windows without bash: structural tests above still gate content.
        eprintln!("bash/sh not available; skipped bash -n (structural tests cover content)");
    }
}
