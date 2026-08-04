//! Shared application state.

use crate::audit::AuditLog;
use crate::auth::AuthManager;
use crate::config::SmosConfig;
use parking_lot::RwLock;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub inner: Arc<AppStateInner>,
}

pub struct AppStateInner {
    pub config: RwLock<SmosConfig>,
    pub audit: AuditLog,
    pub auth: AuthManager,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub version: &'static str,
}

impl AppState {
    pub fn new(config: SmosConfig) -> Self {
        let audit_path = SmosConfig::audit_path(&config.data_dir);
        let _ = std::fs::create_dir_all(&config.data_dir);
        let auth = AuthManager::load(&config.data_dir).unwrap_or_else(|e| {
            panic!(
                "failed to load auth store from {}: {e}",
                config.data_dir.display()
            )
        });
        Self {
            inner: Arc::new(AppStateInner {
                config: RwLock::new(config),
                audit: AuditLog::new(audit_path),
                auth,
                started_at: chrono::Utc::now(),
                version: env!("CARGO_PKG_VERSION"),
            }),
        }
    }

    pub fn data_dir(&self) -> PathBuf {
        self.inner.config.read().data_dir.clone()
    }

    pub fn auth_token(&self) -> Option<String> {
        self.inner.config.read().auth_token.clone()
    }
}
