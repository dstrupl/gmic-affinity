//! Photoshop-compatible filter plugin C types.
//!
//! These definitions mirror the canonical layout of `FilterRecord` and
//! friends from `PIFilter.h` in the Adobe Photoshop SDK 2026 v2, verified
//! by `tests/layout.rs` via `offset_of!`.
//!
//! Only a small subset of the full `FilterRecord` is named (the fields up
//! to `outRowBytes` plus a trailing `_pad` covering the rest); pointer
//! arithmetic into our struct still lands on valid memory because the
//! padded struct size matches `sizeof(FilterRecord) == 648`.

#![allow(dead_code)]

use std::ffi::c_void;

/// QuickDraw `Rect` (`struct { int16 top, left, bottom, right }`).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub top:    i16,
    pub left:   i16,
    pub bottom: i16,
    pub right:  i16,
}

// Compatibility alias for code that still imports the old PRD name.
pub type VRect = Rect;

/// QuickDraw `Point` (`struct { int16 v, h }`).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Point {
    pub v: i16,
    pub h: i16,
}

/// SDK `PSRGBColor`: three 16-bit channels packed without padding,
/// `sizeof(PSRGBColor) == 6`.
pub type PSRGBColor = [u8; 6];

pub type TestAbortProc    = Option<unsafe extern "C" fn() -> i16>;
pub type ProgressProc     = Option<unsafe extern "C" fn(i32, i32)>;
pub type AdvanceStateProc = Option<unsafe extern "C" fn() -> i16>;

/// Partial `FilterRecord`. Field order and names match `PIFilter.h` exactly
/// up through `outRowBytes`; everything from there to `sizeof(FilterRecord)
/// == 648` is collapsed into `_pad`. Renaming the field set, or reordering
/// any of them, requires updating `tests/layout.rs`.
#[repr(C)]
pub struct FilterRecord {
    pub serial_number: i32,         // offset 0
    // 4 bytes of padding here for 8-byte pointer alignment.
    pub abort_proc:    TestAbortProc, // offset 8
    pub progress_proc: ProgressProc,  // offset 16
    pub parameters:    *mut c_void,   // offset 24 (Handle)
    pub image_size:    Point,         // offset 32 (legacy 16-bit Point)
    pub planes:        i16,           // offset 36
    pub filter_rect:   Rect,          // offset 38
    pub background:    PSRGBColor,    // offset 46
    pub foreground:    PSRGBColor,    // offset 52
    // 2 bytes of padding for i32 alignment.
    pub max_space:     i32,           // offset 60
    pub buffer_space:  i32,           // offset 64
    pub in_rect:       Rect,          // offset 68
    pub in_lo_plane:   i16,           // offset 76
    pub in_hi_plane:   i16,           // offset 78
    pub out_rect:      Rect,          // offset 80
    pub out_lo_plane:  i16,           // offset 88
    pub out_hi_plane:  i16,           // offset 90
    // 4 bytes of padding for 8-byte pointer alignment.
    pub in_data:       *mut u8,       // offset 96
    pub in_row_bytes:  i32,           // offset 104
    // 4 bytes of padding for 8-byte pointer alignment.
    pub out_data:      *mut u8,       // offset 112
    pub out_row_bytes: i32,           // offset 120
    // Bytes 124..296 cover isFloating/haveMask/autoMask, maskRect/maskData,
    // imageMode, imageHRes/VRes, floatCoord, wholeSize and the other
    // 3.0-era fields. We don't read them; the host's pointer arithmetic
    // still needs them to be sized correctly though.
    pub _pad1:         [u8; 296 - 124],
    /// Function the plugin calls *inside* `SELECTOR_START` (and again
    /// from `SELECTOR_CONTINUE` if tiling) to ask the host to fill
    /// `in_data` / `in_row_bytes` and allocate `out_data` matching the
    /// rects we set. Without this call Affinity (and Adobe Photoshop)
    /// leaves both at zero and our `validate_filter_record` rejects the
    /// run with "in_data row stride 0". Offset is from the SDK
    /// `offsetof` probe in tests/layout.rs.
    pub advance_state: AdvanceStateProc, // offset 296
    // Remaining post-advanceState fields (supportsAbsolute, padding,
    // inputPadding, outputPadding, etc.). Still opaque to us.
    pub _pad2:         [u8; 648 - 304],
}

// Selector codes from PIFilter.h (verified via `offsetof` probe against
// the SDK 2026 v2 headers).
pub const SELECTOR_ABOUT:      i16 = 0;
pub const SELECTOR_PARAMETERS: i16 = 1;
pub const SELECTOR_PREPARE:    i16 = 2;
pub const SELECTOR_START:      i16 = 3;
pub const SELECTOR_CONTINUE:   i16 = 4;
pub const SELECTOR_FINISH:     i16 = 5;

// Result codes from PIGeneral.h.
pub const NO_ERR:           i16 = 0;
pub const USER_CANCELED:    i16 = -128;
pub const FILTER_BAD_MODE:  i16 = -30101;

// Legacy alias kept so M3 code that returned `USER_CANCEL` still compiles
// to the right value.
pub const USER_CANCEL: i16 = USER_CANCELED;

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn rect_is_8_bytes() {
        assert_eq!(size_of::<Rect>(), 8);
    }

    #[test]
    fn point_is_4_bytes() {
        assert_eq!(size_of::<Point>(), 4);
    }

    #[test]
    fn psrgbcolor_is_6_bytes() {
        assert_eq!(size_of::<PSRGBColor>(), 6);
    }
}
