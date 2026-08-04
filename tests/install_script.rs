//! Structural checks for the shipped release-based install script + README one-liner.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn install_script_path() -> PathBuf {
    repo_root().join("scripts/install.sh")
}

fn release_workflow_path() -> PathBuf {
    repo_root().join(".github/workflows/release.yml")
}

fn read_install_script() -> String {
    let path = install_script_path();
    assert!(path.is_file(), "install script must exist at {}", path.display());
    let body = fs::read_to_string(&path).expect("read install.sh");
    assert!(body.trim().len() > 200, "install.sh must not be empty/stub");
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
fn install_script_downloads_github_release_not_server_build_by_default() {
    let body = read_install_script();
    // Primary path: GitHub Releases API + tarball download
    assert!(
        body.contains("api.github.com") || body.contains("releases/latest") || body.contains("/releases/"),
        "install.sh must query GitHub Releases"
    );
    assert!(
        body.contains("browser_download_url")
            || body.contains("ASSET_URL")
            || body.contains("resolve_asset_url"),
        "install.sh must resolve a release asset URL"
    );
    assert!(
        body.contains("curl") && (body.contains("tar ") || body.contains("tar -")),
        "install.sh must download and extract a tarball"
    );
    // Default path must NOT require cargo build on the server
    assert!(
        !body.contains("cargo build --release")
            || body.contains("SMOS_FROM_SOURCE"),
        "cargo build should only be optional fallback"
    );
    // When FROM_SOURCE appears, it must be gated
    if body.contains("cargo build --release") {
        assert!(
            body.contains("SMOS_FROM_SOURCE"),
            "source build must be behind SMOS_FROM_SOURCE"
        );
    }
}

#[test]
fn install_script_automates_place_and_service() {
    let body = read_install_script();
    for needle in [
        "SMOS_PREFIX",
        "static",
        "install_files",
        "systemd",
        "systemctl",
        "install_systemd",
    ] {
        assert!(body.contains(needle), "missing phase marker: {needle}");
    }
}

#[test]
fn install_script_documents_github_raw_usage() {
    let body = read_install_script();
    assert!(
        body.contains("raw.githubusercontent.com/vantanminh/SMOS") || body.contains("curl -fsSL"),
        "script should document curl|bash usage"
    );
    assert!(
        body.contains("vantanminh/SMOS"),
        "script should reference this GitHub repo"
    );
}

#[test]
fn release_workflow_builds_and_publishes_assets() {
    let path = release_workflow_path();
    assert!(path.is_file(), "missing {}", path.display());
    let body = fs::read_to_string(&path).expect("read release.yml");
    assert!(
        body.contains("tags:") || body.contains("v*"),
        "release workflow should trigger on version tags"
    );
    assert!(
        body.contains("cargo build --release"),
        "release workflow must cargo build --release"
    );
    assert!(
        body.contains("tar") || body.contains(".tar.gz"),
        "release workflow must package tar.gz assets"
    );
    assert!(
        body.contains("action-gh-release")
            || body.contains("softprops/action-gh-release")
            || body.contains("upload-release-asset")
            || body.contains("gh release"),
        "release workflow must publish a GitHub Release"
    );
    assert!(
        body.contains("static"),
        "release package must include static WebUI assets"
    );
}

#[test]
fn readme_documents_curl_bash_oneliner_and_release_flow() {
    let readme = fs::read_to_string(repo_root().join("README.md")).expect("README");
    assert!(
        readme.contains(
            "curl -fsSL https://raw.githubusercontent.com/vantanminh/SMOS/main/scripts/install.sh"
        ),
        "README must document the raw GitHub one-liner URL"
    );
    assert!(
        readme.contains("| bash") || readme.contains("|bash"),
        "README one-liner must pipe to bash"
    );
    assert!(
        readme.to_lowercase().contains("release")
            || readme.contains("prebuilt")
            || readme.contains("GitHub Release"),
        "README should mention prebuilt/release install"
    );
}

#[test]
fn bash_syntax_check_when_available() {
    let path = install_script_path();
    for shell in ["bash", "sh"] {
        let which = Command::new(shell).arg("-c").arg("echo ok").output();
        if which.map(|o| o.status.success()).unwrap_or(false) {
            let out = Command::new(shell)
                .arg("-n")
                .arg(&path)
                .output()
                .unwrap_or_else(|e| panic!("failed to spawn {shell}: {e}"));
            assert!(
                out.status.success(),
                "{shell} -n failed:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
            return;
        }
    }
    eprintln!("bash/sh not available; skipped bash -n");
}
