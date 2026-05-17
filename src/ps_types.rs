//! Photoshop-compatible filter plugin C types.
//!
//! The definitions here mirror Adobe's `PIFilter.h` and friends. Only a small
//! subset of the full `FilterRecord` is named; the remaining ~600 bytes are
//! covered by a trailing `_pad` so the host's pointer arithmetic into our
//! struct still lands on valid memory.
//!
//! IMPORTANT: The exact byte offsets of `in_data`, `out_data` and friends
//! must match the layout of `FilterRecord` in the Adobe Photoshop SDK header.
//! See `tests/layout.rs` for the verification test. As of M1 we have not yet
//! reconciled the PRD's two inconsistent offset claims against the real SDK
//! header; the no-op `PluginMain` in M1 never dereferences these fields so a
//! mismatch cannot crash Affinity at this stage. M3 owns the verification.

#![allow(dead_code)]

use std::ffi::c_void;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VRect {
    pub top: i16,
    pub left: i16,
    pub bottom: i16,
    pub right: i16,
}

pub type TestAbortProc = Option<unsafe extern "C" fn() -> i16>;
pub type ProgressProc = Option<unsafe extern "C" fn(i32, i32)>;

/// Partial `FilterRecord`. Field names follow PRD §6.2 and Adobe's
/// `PIFilter.h`. The trailing `_pad` reserves space for the SDK fields we
/// don't name so the struct's size at least matches the documented 796 bytes
/// on 64-bit macOS.
///
/// The pad size below is conservative; it may be tightened once the real
/// layout is reconciled against `PIFilter.h`.
#[repr(C)]
pub struct FilterRecord {
    pub serial_number: i32,
    pub abort_proc: TestAbortProc,
    pub progress_proc: ProgressProc,
    pub parameters: *mut c_void,
    pub background: [u8; 6], // RGBColor
    pub foreground: [u8; 6], // RGBColor
    pub buffer_space: i32,
    pub max_space: i32,
    pub image_modes_allowed: i32,
    pub filter_rect: VRect,
    pub image_mode: i16,
    pub image_size: VRect,
    pub planes: i16,
    pub in_rect: VRect,
    pub in_lo_plane: i16,
    pub in_hi_plane: i16,
    pub out_rect: VRect,
    pub out_lo_plane: i16,
    pub out_hi_plane: i16,
    pub in_data: *mut u8,
    pub in_row_bytes: i32,
    pub out_data: *mut u8,
    pub out_row_bytes: i32,
    // Cover the remaining SDK fields without naming them.
    pub _pad: [u8; 700],
}

// Selector codes as defined in Adobe's `PIFilter.h`. PRD §4.2 omits
// `filterSelectorAbout` and is therefore off-by-one; trust §6.3 / the SDK
// header instead.
pub const SELECTOR_ABOUT: i16 = 0;
pub const SELECTOR_PARAMETERS: i16 = 1;
pub const SELECTOR_PREPARE: i16 = 2;
pub const SELECTOR_START: i16 = 3;
pub const SELECTOR_CONTINUE: i16 = 4;
pub const SELECTOR_FINISH: i16 = 5;

// Result codes.
pub const NO_ERR: i16 = 0;
pub const USER_CANCEL: i16 = 1; // filterBadParameters / cancel-ish; refined later
pub const FILTER_BAD_MODE: i16 = -1;

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn vrect_is_8_bytes() {
        assert_eq!(size_of::<VRect>(), 8, "VRect must be 8 bytes (4 x i16)");
    }
}
