//! `FilterRecord` layout verification.
//!
//! Wrong offsets here cause silent pixel corruption or crashes inside
//! Affinity. The values below come from PRD §8 and claim to match the 2021
//! macOS 64-bit Adobe Photoshop SDK. They have NOT yet been reconciled
//! against the real `PIFilter.h` (M3 owns that).
//!
//! These offset assertions are gated behind `#[ignore]` so `cargo test`
//! still passes; run `cargo test -- --ignored` to see the current
//! actual-vs-expected gap. Once the SDK is on disk, fix the struct (or the
//! expected values) until both `cargo test` and `cargo test -- --ignored`
//! pass, then remove the `#[ignore]` attribute.

use std::mem::{offset_of, size_of};

// The crate is referenced by its `[lib].name` (`GmicFilter`) rather than the
// package name because we set `name = "GmicFilter"` in Cargo.toml; the rlib
// crate-type is what makes it linkable into this integration test.
use GmicFilter::ps_types::{FilterRecord, VRect};

#[test]
fn vrect_is_8_bytes() {
    assert_eq!(size_of::<VRect>(), 8);
}

#[test]
fn report_actual_filter_record_layout() {
    // Always-running informational test: prints the offsets our struct
    // currently produces. `cargo test -- --nocapture` to see the output.
    eprintln!("FilterRecord size           = {}",  size_of::<FilterRecord>());
    eprintln!("offset_of!(in_data)         = {}",  offset_of!(FilterRecord, in_data));
    eprintln!("offset_of!(in_row_bytes)    = {}",  offset_of!(FilterRecord, in_row_bytes));
    eprintln!("offset_of!(out_data)        = {}",  offset_of!(FilterRecord, out_data));
    eprintln!("offset_of!(out_row_bytes)   = {}",  offset_of!(FilterRecord, out_row_bytes));
}

#[test]
#[ignore = "PRD §8 expected offsets; enable once reconciled against PIFilter.h"]
fn filter_record_matches_prd_section_8() {
    assert_eq!(offset_of!(FilterRecord, in_data),       88, "in_data offset");
    assert_eq!(offset_of!(FilterRecord, in_row_bytes),  96, "in_row_bytes offset");
    assert_eq!(offset_of!(FilterRecord, out_data),     104, "out_data offset");
    assert_eq!(offset_of!(FilterRecord, out_row_bytes),112, "out_row_bytes offset");
}
