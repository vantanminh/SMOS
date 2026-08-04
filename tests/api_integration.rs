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

/// Ready state with operator account + session token.
fn test_state_authed(dir: &std::path::Path) -> (AppState, String) {
    let state = test_state(dir);
    let (login, _) = state
        .inner
        .auth
        .setup_with_totp_option("admin@example.com", "password123", false)
        .expect("setup account");
    let token = login.token.expect("session token");
    (state, token)
}

async fn json_get(app: &axum::Router, path: &str) -> (StatusCode, serde_json::Value) {
    json_get_auth(app, path, None).await
}

async fn json_get_auth(
    app: &axum::Router,
    path: &str,
    token: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().uri(path);
    if let Some(t) = token {
        builder = builder.header("x-smos-session", t);
    }
    let res = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
        serde_json::json!({ "raw": String::from_utf8_lossy(&bytes) })
    });
    (status, v)
}

async fn json_put_auth(
    app: &axum::Router,
    path: &str,
    token: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(path)
                .header("content-type", "application/json")
                .header("x-smos-session", token)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
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
    http_get_auth(addr, path, None).await
}

async fn http_get_auth(addr: SocketAddr, path: &str, token: Option<&str>) -> String {
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let auth_h = token
        .map(|t| format!("X-SMOS-Session: {t}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {}\r\n{auth_h}Connection: close\r\n\r\n",
        addr
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).to_string()
}

#[tokio::test]
async fn health_and_auth_status_public() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state(dir.path());
    let app = api::router(state, resolve_static_dir());

    let (st, health) = json_get(&app, "/api/health").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(health["status"], "ok");
    assert_eq!(health["host_label"], "test-host");
    assert_eq!(health["setup_required"], true);
    assert_eq!(health["authenticated"], false);

    let (st, status) = json_get(&app, "/api/auth/status").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(status["setup_required"], true);
    assert_eq!(status["authenticated"], false);
}

#[tokio::test]
async fn setup_required_blocks_metrics() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state(dir.path());
    let app = api::router(state, resolve_static_dir());

    let (st, _) = json_get(&app, "/api/metrics").await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn onboarding_setup_and_login() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state(dir.path());
    let app = api::router(state, resolve_static_dir());

    let body = serde_json::json!({
        "email": "ops@example.com",
        "password": "securepass1",
        "enable_totp": false
    });
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/setup")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let setup: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(setup["status"], "ok");
    let token = setup["token"].as_str().unwrap().to_string();

    let (st, metrics) = json_get_auth(&app, "/api/metrics", Some(&token)).await;
    assert_eq!(st, StatusCode::OK);
    assert!(metrics["memory"]["total_bytes"].as_u64().unwrap() > 0);

    // Logout then login again
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("x-smos-session", &token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let login_body = serde_json::json!({
        "email": "ops@example.com",
        "password": "securepass1"
    });
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(login_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let login: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(login["status"], "ok");
    assert!(login["token"].as_str().unwrap().len() > 20);
}

#[tokio::test]
async fn health_metrics_processes_shape() {
    let dir = tempfile::tempdir().unwrap();
    let (state, token) = test_state_authed(dir.path());
    let app = api::router(state, resolve_static_dir());

    let (st, health) = json_get_auth(&app, "/api/health", Some(&token)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(health["status"], "ok");
    assert_eq!(health["host_label"], "test-host");
    assert_eq!(health["setup_required"], false);
    assert_eq!(health["authenticated"], true);
    assert!(health["version"].as_str().unwrap().len() > 0);

    let (st, metrics) = json_get_auth(&app, "/api/metrics", Some(&token)).await;
    assert_eq!(st, StatusCode::OK);
    assert!(metrics["memory"]["total_bytes"].as_u64().unwrap() > 0);
    assert!(metrics["cpu"]["core_count"].as_u64().unwrap() > 0);
    assert!(!metrics["hostname"].as_str().unwrap().is_empty());
    assert!(!metrics["disks"].as_array().unwrap().is_empty());

    let (st, procs) = json_get_auth(&app, "/api/processes", Some(&token)).await;
    assert_eq!(st, StatusCode::OK);
    let arr = procs.as_array().unwrap();
    assert!(!arr.is_empty());
    assert!(arr[0]["pid"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn config_update_and_audit() {
    let dir = tempfile::tempdir().unwrap();
    let (state, token) = test_state_authed(dir.path());
    let app = api::router(state, resolve_static_dir());

    let body = serde_json::json!({
        "host_label": "updated-label",
        "log_tail_lines": 321,
        "history_retention_days": 90
    });
    let (st, cfg) = json_put_auth(&app, "/api/config", &token, body).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(cfg["host_label"], "updated-label");
    assert_eq!(cfg["log_tail_lines"], 321);
    assert_eq!(cfg["history_retention_days"], 90);

    let (st, cfg2) = json_get_auth(&app, "/api/config", Some(&token)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(cfg2["host_label"], "updated-label");
    assert_eq!(cfg2["history_retention_days"], 90);
    assert_eq!(cfg2["metrics_history_interval_secs"], 60);

    let (st, audit) = json_get_auth(&app, "/api/audit?limit=20", Some(&token)).await;
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
async fn metrics_history_and_status() {
    let dir = tempfile::tempdir().unwrap();
    let (state, token) = test_state_authed(dir.path());
    let app = api::router(state, resolve_static_dir());

    let snap = smos::metrics::collect_metrics();
    smos::history::record_metrics_snapshot(dir.path(), &snap).unwrap();
    smos::history::record_metrics_snapshot(dir.path(), &snap).unwrap();

    let (st, hist) =
        json_get_auth(&app, "/api/metrics/history?hours=24&limit=100", Some(&token)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(hist["retention_days"], 30);
    assert_eq!(hist["sample_interval_secs"], 60);
    let samples = hist["samples"].as_array().unwrap();
    assert!(samples.len() >= 2, "expected seeded samples: {hist}");
    assert!(samples[0]["cpu"].as_f64().is_some());
    assert!(samples[0]["mem"].as_f64().is_some());

    let (st, status) = json_get_auth(&app, "/api/history", Some(&token)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(status["retention_days"], 30);
    assert!(status["metrics_samples_on_disk"].as_u64().unwrap() >= 2);
    assert!(status["metrics_bytes"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn invalid_config_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (state, token) = test_state_authed(dir.path());
    let app = api::router(state, resolve_static_dir());

    let (st, before) = json_get_auth(&app, "/api/config", Some(&token)).await;
    assert_eq!(st, StatusCode::OK);
    let label_before = before["host_label"].as_str().unwrap().to_string();
    let lines_before = before["log_tail_lines"].as_u64().unwrap();

    let body = serde_json::json!({
        "host_label": "should-not-stick",
        "log_tail_lines": 0
    });
    let (st, _) = json_put_auth(&app, "/api/config", &token, body).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    let (st, after) = json_get_auth(&app, "/api/config", Some(&token)).await;
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
    let (state, token) = test_state_authed(dir.path());
    let app = api::router(state, resolve_static_dir());

    let (st, sources) = json_get_auth(&app, "/api/logs", Some(&token)).await;
    assert_eq!(st, StatusCode::OK);
    assert!(sources
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["id"] == "smos-service"));

    let (st, tail) = json_get_auth(&app, "/api/logs/smos-service?lines=50", Some(&token)).await;
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
    let (state, token) = test_state_authed(dir.path());
    let app = api::router(state, resolve_static_dir());
    let pid = std::process::id();
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{pid}/action"))
                .header("content-type", "application/json")
                .header("x-smos-session", token)
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
    let mut child = spawn_sleep_child();
    let pid = child.id();
    let result = smos::processes::act_on_process(pid, smos::processes::ProcessAction::Kill);
    match result {
        Ok(r) => {
            assert!(r.success);
            assert_eq!(r.pid, pid);
        }
        Err(e) => {
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
    let (login, _) = state
        .inner
        .auth
        .setup_with_totp_option("live@example.com", "password123", false)
        .unwrap();
    let token = login.token.unwrap();
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

    let metrics = http_get_auth(addr, "/api/metrics", Some(&token)).await;
    assert!(metrics.contains("total_bytes"), "metrics body: {metrics}");

    let procs = http_get_auth(addr, "/api/processes", Some(&token)).await;
    assert!(procs.contains("\"pid\""), "processes body: {procs}");

    let ui = http_get(addr, "/").await;
    assert!(ui.contains("SMOS"), "UI body: {}", &ui[..ui.len().min(200)]);
}
