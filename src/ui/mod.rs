//! UI helpers for the plugin.
//!
//! `alert` is pure Rust and is always compiled so the PluginMain
//! error-handling matrix can be table-tested in the default build.
//!
//! `picker`, `runloop`, `modal_close_delegate`, and
//! `picker_catalogue_data_source` drive AppKit and are only compiled
//! under `--features live` (the build that actually dereferences
//! `FilterRecord` and renders the modal panel inside Affinity Photo).

pub mod alert;

#[cfg(feature = "live")]
pub(crate) mod modal_close_delegate;
#[cfg(feature = "live")]
pub mod picker;
#[cfg(feature = "live")]
pub(crate) mod picker_actions;
#[cfg(feature = "live")]
pub(crate) mod picker_catalogue_data_source;
#[cfg(feature = "live")]
pub(crate) mod picker_form;
#[cfg(feature = "live")]
pub mod runloop;
