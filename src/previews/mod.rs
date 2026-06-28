//! Build-time preview-generation support, shared between the
//! `gen-previews` binary and the live picker UI.
//!
//! Everything here compiles in the default (non-`live`) build: it is
//! pure Rust with no AppKit dependency. The binary uses the whole
//! module; the UI uses only [`sanitise_key`] to re-derive a filter's
//! preview filename without parsing the manifest at runtime.

pub mod manifest;

// sanitise_key, format_float, default_argv are added in later tasks.
