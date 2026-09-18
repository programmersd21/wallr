//! Compatibility re-export: the config schema now lives in `wallr-common`
//! so the client can resolve sockets and flags without the engine.
//! Existing `crate::config::...` paths keep working unchanged.
pub use wallr_common::config::*;
pub use wallr_common::types::{GpuSelection, ScalingMode, ThemeProvider};
