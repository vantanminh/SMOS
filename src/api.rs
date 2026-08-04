//! HTTP API routes for SMOS.

use crate::audit::AuditEntry;
use crate::config::{ConfigUpdate, PublicConfig, SmosConfig};
use crate::logs::{self, LogSourceInfo, LogTail, SERVICE_LOG_ID};
use crate::metrics::{self, MetricsSnapshot};
use crate::processes::{self, ProcessActionRequest, ProcessActionResult, ProcessInfo};
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

#[derive(Debug, Serialize)]
struct ApiError {
    error: String,
}

// re-export Serialize for ApiError
use serde::Serialize;

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    host_label: String,
    started_at: chrono::DateTime<chrono::Utc>,
    uptime_secs: i64,
}

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    pub lines: Option<usize>,
}

pub fn router(state: AppState, static_dir: PathBuf) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(get_metrics))
        .route("/processes", get(get_processes))
        .route("/processes/{pid}/action", post(process_action))
        .route("/logs", get(list_log_sources))
        .route("/logs/{source_id}", get(get_log_tail))
        .route("/config", get(get_config).put(put_config))
        .route("/audit", get(list_audit))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .nest("/api", api)
        .route("/", get(index_html))
        .route("/dashboard", get(index_html))
        .route("/dashboard/{*rest}", get(index_html))
        .nest_service(
            "/assets",
            ServeDir::new(static_dir.join("assets")),
        )
        .fallback_service(ServeDir::new(static_dir).append_index_html_on_directories(true))
        .layer(cors)
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-cache"),
        ))
        .with_state(state)
}

async fn index_html(State(state): State<AppState>) -> impl IntoResponse {
    let static_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("static").join("index.html")))
        .filter(|p| p.exists())
        .or_else(|| {
            let cwd = std::env::current_dir().ok()?;
            let p = cwd.join("static").join("index.html");
            if p.exists() {
                Some(p)
            } else {
                None
            }
        });

    // Prefer reading from known static dir next to CARGO_MANIFEST_DIR at build/dev time
    let candidates = [
        static_path,
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static/index.html")),
        std::env::current_dir()
            .ok()
            .map(|d| d.join("static/index.html")),
    ];

    for cand in candidates.into_iter().flatten() {
        if let Ok(body) = std::fs::read_to_string(&cand) {
            return Html(body).into_response();
        }
    }

    // Inline minimal fallback so health/UI never 404 on missing assets
    let _ = state;
    Html(include_str!("../static/index.html")).into_response()
}

async fn auth_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    // Read-only routes are open; mutating routes require token when configured.
    let is_mutating = method != axum::http::Method::GET && method != axum::http::Method::HEAD;
    if !is_mutating {
        return Ok(next.run(req).await);
    }

    let Some(expected) = state.auth_token() else {
        return Ok(next.run(req).await);
    };

    let headers = req.headers();
    let provided = extract_token(headers);
    if provided.as_deref() == Some(expected.as_str()) {
        return Ok(next.run(req).await);
    }

    tracing::warn!("auth failed for {method} {path}");
    Err(StatusCode::UNAUTHORIZED)
}

fn extract_token(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("x-smos-token").and_then(|v| v.to_str().ok()) {
        return Some(v.to_string());
    }
    if let Some(v) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        let v = v.trim();
        if let Some(rest) = v.strip_prefix("Bearer ") {
            return Some(rest.to_string());
        }
        return Some(v.to_string());
    }
    None
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let cfg = state.inner.config.read();
    let started = state.inner.started_at;
    let uptime = (chrono::Utc::now() - started).num_seconds();
    Json(HealthResponse {
        status: "ok",
        version: state.inner.version,
        host_label: cfg.host_label.clone(),
        started_at: started,
        uptime_secs: uptime,
    })
}

async fn get_metrics() -> Json<MetricsSnapshot> {
    // Blocking sysinfo work off the async runtime.
    let snap = tokio::task::spawn_blocking(metrics::collect_metrics)
        .await
        .unwrap_or_else(|_| metrics::collect_metrics());
    Json(snap)
}

async fn get_processes() -> Json<Vec<ProcessInfo>> {
    let list = tokio::task::spawn_blocking(processes::list_processes)
        .await
        .unwrap_or_else(|_| processes::list_processes());
    Json(list)
}

async fn process_action(
    State(state): State<AppState>,
    Path(pid): Path<u32>,
    Json(body): Json<ProcessActionRequest>,
) -> Result<Json<ProcessActionResult>, (StatusCode, Json<ApiError>)> {
    let action = body.action;
    let result = tokio::task::spawn_blocking(move || processes::act_on_process(pid, action))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: e.to_string(),
                }),
            )
        })?;

    match result {
        Ok(res) => {
            let _ = state.inner.audit.append(
                format!("process.{:?}", res.action).to_lowercase(),
                "operator",
                format!("pid:{pid}"),
                json!({ "pid": pid, "message": res.message }),
                res.success,
            );
            Ok(Json(res))
        }
        Err(e) => {
            let _ = state.inner.audit.append(
                format!("process.{action:?}").to_lowercase(),
                "operator",
                format!("pid:{pid}"),
                json!({ "pid": pid, "error": e.to_string() }),
                false,
            );
            let status = match &e {
                processes::ProcessError::NotFound(_) => StatusCode::NOT_FOUND,
                processes::ProcessError::SelfProcess(_) => StatusCode::BAD_REQUEST,
                processes::ProcessError::ActionFailed { .. } => StatusCode::FORBIDDEN,
            };
            Err((
                status,
                Json(ApiError {
                    error: e.to_string(),
                }),
            ))
        }
    }
}

async fn list_log_sources(State(state): State<AppState>) -> Json<Vec<LogSourceInfo>> {
    let cfg = state.inner.config.read().clone();
    let mut sources = vec![logs::service_log_source(&cfg.data_dir)];
    for s in &cfg.log_sources {
        sources.push(logs::source_info(&s.id, &s.label, &s.path));
    }
    Json(sources)
}

async fn get_log_tail(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
    Query(q): Query<LogQuery>,
) -> Result<Json<LogTail>, (StatusCode, Json<ApiError>)> {
    let cfg = state.inner.config.read().clone();
    let max_lines = q.lines.unwrap_or(cfg.log_tail_lines).clamp(1, 50_000);

    let path = if source_id == SERVICE_LOG_ID {
        SmosConfig::service_log_path(&cfg.data_dir)
    } else {
        cfg.log_sources
            .iter()
            .find(|s| s.id == source_id)
            .map(|s| s.path.clone())
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    Json(ApiError {
                        error: format!("unknown log source '{source_id}'"),
                    }),
                )
            })?
    };

    let sid = source_id.clone();
    let tail = tokio::task::spawn_blocking(move || logs::tail_source(&sid, &path, max_lines))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: e.to_string(),
                }),
            )
        })?
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: e.to_string(),
                }),
            )
        })?;

    Ok(Json(tail))
}

async fn get_config(State(state): State<AppState>) -> Json<PublicConfig> {
    let cfg = state.inner.config.read();
    Json(cfg.public_view())
}

async fn put_config(
    State(state): State<AppState>,
    Json(update): Json<ConfigUpdate>,
) -> Result<Json<PublicConfig>, (StatusCode, Json<ApiError>)> {
    let mut cfg = state.inner.config.write();
    let before = cfg.public_view();
    match cfg.apply_update(update) {
        Ok(()) => {
            if let Err(e) = cfg.save() {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiError {
                        error: format!("persist failed: {e}"),
                    }),
                ));
            }
            let view = cfg.public_view();
            drop(cfg);
            let _ = state.inner.audit.append(
                "config.update",
                "operator",
                "config",
                json!({ "before": before, "after": view }),
                true,
            );
            Ok(Json(view))
        }
        Err(e) => {
            drop(cfg);
            let _ = state.inner.audit.append(
                "config.update",
                "operator",
                "config",
                json!({ "error": e.to_string() }),
                false,
            );
            Err((
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: e.to_string(),
                }),
            ))
        }
    }
}

async fn list_audit(
    State(state): State<AppState>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Vec<AuditEntry>>, (StatusCode, Json<ApiError>)> {
    let limit = q.limit.unwrap_or(100).min(1000);
    state
        .inner
        .audit
        .list(limit)
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: e.to_string(),
                }),
            )
        })
}

/// Resolve static assets directory for ServeDir.
pub fn resolve_static_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static");
    if manifest.exists() {
        return manifest;
    }
    if let Ok(exe) = std::env::current_exe() {
        let beside = exe.parent().map(|p| p.join("static"));
        if let Some(p) = beside {
            if p.exists() {
                return p;
            }
        }
    }
    std::env::current_dir()
        .map(|d| d.join("static"))
        .unwrap_or_else(|_| PathBuf::from("static"))
}

// silence unused import if Body unused
#[allow(dead_code)]
fn _body_type(_: Body) {}

#[allow(dead_code)]
fn _value_type(_: Value) {}
