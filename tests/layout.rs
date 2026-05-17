//! `FilterRecord` layout verification.
//!
//! Wrong offsets here cause silent pixel corruption or crashes inside
//! Affinity. The expected values below are the canonical layout reported
//! by a `clang -arch arm64` probe against `PIFilter.h` from the Adobe
//! Photoshop SDK 2026 v2 (`pluginsdk/photoshopapi/photoshop/PIFilter.h`).
//!
//! If you upgrade the SDK and these tests fail, re-run the probe in
//! `/tmp/pslayout/probe.c` (the recipe lives in git history of this file's
//! introduction) and update both the struct and these constants together.

use std::mem::{offset_of, size_of};

use GmicFilter::ps_types::{FilterRecord, Point, PSRGBColor, Rect};

#[test]
fn primitive_sizes_match_sdk() {
    assert_eq!(size_of::<Rect>(),       8, "Rect");
    assert_eq!(size_of::<Point>(),      4, "Point");
    assert_eq!(size_of::<PSRGBColor>(), 6, "PSRGBColor");
}

#[test]
fn filter_record_total_size() {
    // sizeof(FilterRecord) in the 2026 v2 macOS 64-bit SDK is 648 bytes.
    assert_eq!(size_of::<FilterRecord>(), 648);
}

#[test]
fn filter_record_field_offsets_match_sdk() {
    assert_eq!(offset_of!(FilterRecord, serial_number),   0,   "serial_number");
    assert_eq!(offset_of!(FilterRecord, abort_proc),      8,   "abort_proc");
    assert_eq!(offset_of!(FilterRecord, progress_proc),  16,   "progress_proc");
    assert_eq!(offset_of!(FilterRecord, parameters),     24,   "parameters");
    assert_eq!(offset_of!(FilterRecord, image_size),     32,   "image_size");
    assert_eq!(offset_of!(FilterRecord, planes),         36,   "planes");
    assert_eq!(offset_of!(FilterRecord, filter_rect),    38,   "filter_rect");
    assert_eq!(offset_of!(FilterRecord, background),     46,   "background");
    assert_eq!(offset_of!(FilterRecord, foreground),     52,   "foreground");
    assert_eq!(offset_of!(FilterRecord, max_space),      60,   "max_space");
    assert_eq!(offset_of!(FilterRecord, buffer_space),   64,   "buffer_space");
    assert_eq!(offset_of!(FilterRecord, in_rect),        68,   "in_rect");
    assert_eq!(offset_of!(FilterRecord, in_lo_plane),    76,   "in_lo_plane");
    assert_eq!(offset_of!(FilterRecord, in_hi_plane),    78,   "in_hi_plane");
    assert_eq!(offset_of!(FilterRecord, out_rect),       80,   "out_rect");
    assert_eq!(offset_of!(FilterRecord, out_lo_plane),   88,   "out_lo_plane");
    assert_eq!(offset_of!(FilterRecord, out_hi_plane),   90,   "out_hi_plane");
    assert_eq!(offset_of!(FilterRecord, in_data),        96,   "in_data");
    assert_eq!(offset_of!(FilterRecord, in_row_bytes),  104,   "in_row_bytes");
    assert_eq!(offset_of!(FilterRecord, out_data),      112,   "out_data");
    assert_eq!(offset_of!(FilterRecord, out_row_bytes), 120,   "out_row_bytes");
    assert_eq!(offset_of!(FilterRecord, advance_state), 296,   "advance_state");
}
