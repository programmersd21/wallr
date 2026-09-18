//! Compatibility re-export: transition effects now live in `wallr-common`
//! alongside the rest of the client/daemon protocol. Existing
//! `crate::animation::Effect` paths keep working unchanged.
pub use wallr_common::effect::*;
