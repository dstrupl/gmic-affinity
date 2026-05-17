//! macOS AppKit-facing UI for the plugin.
//!
//! Everything in this module needs to run on the main thread, where
//! Affinity calls `PluginMain` for us. This module is only exposed when the
//! crate is built with the `live` feature (`#[cfg(feature = "live")] pub mod ui`
//! in `lib.rs`).

pub mod runloop;
pub mod picker;
