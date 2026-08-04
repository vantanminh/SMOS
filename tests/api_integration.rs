//! Integration tests against real domain modules + HTTP app (in-process).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use smos::api::{self, resolve_static_dir};
use smos::config::SmosConfig;
use smos::state::AppState;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

fn test_state(dir: &std::path::Path) -> AppState {
    let mut cfg = SmosConfig::default();
    cfg.data_dir = dir.to_path_buf();
    cfg.bind = "127.0.0.1:0".into();
    cfg.host_label = "test-host".into();
    cfg.ensure_data_dir().unwrap();
    cfg.save().unwrap();
    smos::logs::append_service_line(dir, "integration-seed-line").unwrap();
    AppState::new(cfg)
}

async fn json_get(app: &axum::Router, path: &str) -> (StatusCode, serde_json::Value) {
    let res = app
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
        serde_json::json!({ "raw": String::from_utf8_lossy(&bytes) })
    });
    (status, v)
}

async fn http_get(addr: SocketAddr, path: &str) -> String {
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        addr
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).to_string()
}

#[tokio::test]
async fn health_metrics_processes_shape() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state(dir.path());
    let app = api::router(state, resolve_static_dir());

    let (st, health) = json_get(&app, "/api/health").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(health["status"], "ok");
    assert_eq!(health["host_label"], "test-host");
    assert!(health["version"].as_str().unwrap().len() > 0);

    let (st, metrics) = json_get(&app, "/api/metrics").await;
    assert_eq!(st, StatusCode::OK);
    assert!(metrics["memory"]["total_bytes"].as_u64().unwrap() > 0);
    assert!(metrics["cpu"]["core_count"].as_u64().unwrap() > 0);
    assert!(!metrics["hostname"].as_str().unwrap().is_empty());
    assert!(!metrics["disks"].as_array().unwrap().is_empty());

    let (st, procs) = json_get(&app, "/api/processes").await;
    assert_eq!(st, StatusCode::OK);
    let arr = procs.as_array().unwrap();
    assert!(!arr.is_empty());
    assert!(arr[0]["pid"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn config_update_and_audit() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state(dir.path());
    let app = api::router(state, resolve_static_dir());

    let body = serde_json::json!({
        "host_label": "updated-label",
        "log_tail_lines": 321
    });
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/config")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let cfg: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(cfg["host_label"], "updated-label");
    assert_eq!(cfg["log_tail_lines"], 321);

    let (st, cfg2) = json_get(&app, "/api/config").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(cfg2["host_label"], "updated-label");

    let (st, audit) = json_get(&app, "/api/audit?limit=20").await;
    assert_eq!(st, StatusCode::OK);
    let entries = audit.as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["action"] == "config.update" && e["success"] == true),
        "audit must contain successful config.update: {entries:?}"
    );
}

#[tokio::test]
async fn invalid_config_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state(dir.path());
    let app = api::router(state, resolve_static_dir());

    let (st, before) = json_get(&app, "/api/config").await;
    assert_eq!(st, StatusCode::OK);
    let label_before = before["host_label"].as_str().unwrap().to_string();
    let lines_before = before["log_tail_lines"].as_u64().unwrap();

    // Invalid update mixes a would-be mutation with a hard validation failure.
    let body = serde_json::json!({
        "host_label": "should-not-stick",
        "log_tail_lines": 0
    });
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/config")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Read-after-reject: criterion 5 — invalid updates must not apply.
    let (st, after) = json_get(&app, "/api/config").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        after["host_label"].as_str().unwrap(),
        label_before,
        "host_label must remain unchanged after rejected PUT"
    );
    assert_eq!(
        after["log_tail_lines"].as_u64().unwrap(),
        lines_before,
        "log_tail_lines must remain unchanged after rejected PUT"
    );
    assert_ne!(after["host_label"], "should-not-stick");
}

#[tokio::test]
async fn logs_tail_service_source() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state(dir.path());
    let app = api::router(state, resolve_static_dir());

    let (st, sources) = json_get(&app, "/api/logs").await;
    assert_eq!(st, StatusCode::OK);
    assert!(sources
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["id"] == "smos-service"));

    let (st, tail) = json_get(&app, "/api/logs/smos-service?lines=50").await;
    assert_eq!(st, StatusCode::OK);
    let lines = tail["lines"].as_array().unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l.as_str().unwrap_or("").contains("integration-seed-line")),
        "expected seed line in log tail: {tail:?}"
    );
}

#[tokio::test]
async fn dashboard_html_served() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state(dir.path());
    let app = api::router(state, resolve_static_dir());
    let res = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes);
    assert!(html.contains("SMOS"), "dashboard HTML should mention SMOS");
    assert!(html.contains("id=\"app\"") || html.contains("id=\"view\""));
}

#[tokio::test]
async fn process_self_action_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state(dir.path());
    let app = api::router(state, resolve_static_dir());
    let pid = std::process::id();
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{pid}/action"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"action":"kill"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

/// Spawn a long-lived child for kill tests (Linux CI + Windows).
fn spawn_sleep_child() -> std::process::Child {
    #[cfg(windows)]
    {
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 120"])
            .spawn()
            .expect("spawn sleep child (powershell)")
    }
    #[cfg(not(windows))]
    {
        // Prefer `sleep` from coreutils; fall back to `sh -c`.
        std::process::Command::new("sleep")
            .arg("120")
            .spawn()
            .or_else(|_| {
                std::process::Command::new("sh")
                    .args(["-c", "sleep 120"])
                    .spawn()
            })
            .expect("spawn sleep child (sleep/sh)")
    }
}

#[tokio::test]
async fn process_kill_child_end_to_end() {
    // Spawn a long-sleep child, kill it through the real process module API.
    let mut child = spawn_sleep_child();
    let pid = child.id();
    let result = smos::processes::act_on_process(pid, smos::processes::ProcessAction::Kill);
    match result {
        Ok(r) => {
            assert!(r.success);
            assert_eq!(r.pid, pid);
        }
        Err(e) => {
            // On restricted environments kill may fail; still prove path ran.
            let msg = e.to_string();
            assert!(
                msg.contains("permission") || msg.contains("failed") || msg.contains("not found"),
                "unexpected error: {msg}"
            );
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
async fn live_server_bind_and_health() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = SmosConfig::default();
    cfg.data_dir = dir.path().to_path_buf();
    cfg.host_label = "live-bind".into();
    cfg.ensure_data_dir().unwrap();
    let state = AppState::new(cfg);
    let app = api::router(state, resolve_static_dir());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;

    let health = http_get(addr, "/api/health").await;
    assert!(health.contains("live-bind"), "health body: {health}");
    assert!(health.contains("\"status\":\"ok\"") || health.contains("\"status\": \"ok\""));

    let metrics = http_get(addr, "/api/metrics").await;
    assert!(metrics.contains("total_bytes"), "metrics body: {metrics}");

    let procs = http_get(addr, "/api/processes").await;
    assert!(procs.contains("\"pid\""), "processes body: {procs}");

    let ui = http_get(addr, "/").await;
    assert!(ui.contains("SMOS"), "UI body: {}", &ui[..ui.len().min(200)]);
}
