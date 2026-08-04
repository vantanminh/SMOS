//! Structural checks for the shipped GitHub Actions CI workflow.
//! Ensures push-triggered build/test steps stay present in-repo.

use std::fs;
use std::path::PathBuf;

fn workflow_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/ci.yml")
}

fn read_workflow() -> String {
    let path = workflow_path();
    assert!(
        path.is_file(),
        "CI workflow must exist at {}",
        path.display()
    );
    fs::read_to_string(&path).expect("read CI workflow")
}

#[test]
fn ci_workflow_exists_and_is_nonempty() {
    let body = read_workflow();
    assert!(
        body.trim().len() > 50,
        "workflow YAML must not be an empty placeholder"
    );
}

#[test]
fn ci_workflow_triggers_on_push() {
    let body = read_workflow();
    // Accept common forms: `on:\n  push:` or `on: [push, pull_request]`
    let has_push = body.contains("push:")
        || body.contains("push ")
        || body.contains("[push")
        || body.contains("push,")
        || body.contains("push]");
    assert!(
        has_push,
        "workflow must trigger on push; got:\n{body}"
    );
}

#[test]
fn ci_workflow_installs_rust_and_runs_cargo() {
    let body = read_workflow();
    let has_rust = body.contains("rust-toolchain")
        || body.contains("rustup")
        || body.contains("dtolnay/rust-toolchain")
        || body.contains("actions-rs/toolchain");
    assert!(
        has_rust,
        "workflow must install/select a Rust toolchain; got:\n{body}"
    );

    assert!(
        body.contains("cargo build"),
        "workflow must run cargo build; got:\n{body}"
    );
    assert!(
        body.contains("cargo test"),
        "workflow must run cargo test; got:\n{body}"
    );

    // No empty run stubs for the cargo steps
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("run:") {
            let rest = trimmed.trim_start_matches("run:").trim();
            assert!(
                !rest.is_empty(),
                "workflow has empty run: stub: {line}"
            );
        }
    }
}
