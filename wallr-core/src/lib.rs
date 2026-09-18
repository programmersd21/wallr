//! # wallr-core
//!
//! Core library for Wallr - a native, GPU-accelerated Wayland wallpaper engine.

// `wgpu-core`'s internal types (`Hub`/`Registry`/…) are deep enough to trip
// rustc's `recursion_depth_exceeding_limit` future-compat lint whenever a
// closure captures renderer state (e.g. `Arc<Mutex<RenderState>>` in
// `spawn_blocking`). That depth comes from wgpu, not from our code, and the
// types were accepted under the old limit, so allow the lint here until a
// wgpu upgrade shallows those internals. See rust-lang/rust#159228.
// `unknown_lints` is also allowed so stable (which doesn't know this lint
// yet) doesn't error under `-D warnings`.
#![allow(unknown_lints)]
#![allow(recursion_depth_exceeding_limit)]

pub mod animated;
pub mod animation;
pub mod config;
pub mod daemon;
pub mod ipc;
pub mod renderer;
pub mod shader;
pub mod theme;
pub mod video;
pub mod wallpaper;
