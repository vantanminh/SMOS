//! SMOS — Server Management OS library.
//!
//! Domain modules are shared by the binary entry point and tests so assertions
//! always exercise the same code paths the HTTP handlers use.

pub mod api;
pub mod audit;
pub mod config;
pub mod history;
pub mod logs;
pub mod metrics;
pub mod processes;
pub mod state;

pub use config::{SmosConfig, SmosConfigFile};
pub use state::AppState;
