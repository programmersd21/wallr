//! Shared protocol between the `wallr` client and the daemon engine.
//!
//! This crate holds the data both sides must agree on: transition effects,
//! the IPC command/response vocabulary, CLI argument shapes, and the config
//! schema. It depends only on serialization and CLI parsing so the client
//! binary stays small; GPU, Wayland, and decoding live in `wallr-core`.

pub mod cli;
pub mod config;
pub mod effect;
pub mod ipc;
pub mod types;
