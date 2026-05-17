//! Pixel transformation entry points.
//!
//! M3: `run_passthrough` proves that we can safely read every input pixel
//! from the host and write back to the output buffer, honouring row strides
//! and channel counts, without ever crashing the host. It inverts only the
//! red channel so the result is visually distinct from the input (an obvious
//! "the filter ran" signal) but trivially reversible for testing.
//!
//! M4 will add `run_gmic` (TIFF round-trip via subprocess).
//!
//! All defensive bounds checks live here so the `PluginMain` glue in
//! `lib.rs` stays thin.

use crate::ps_types::FilterRecord;

/// Maximum supported edge length. Anything larger almost certainly means the
/// FilterRecord we are reading is misaligned and the dimensions are bogus;
/// refusing to proceed beats segfaulting inside the host.
pub const MAX_EDGE: i32 = 32_768;

#[derive(Debug)]
pub enum FilterError {
    NullPointer(&'static str),
    BadDimensions { width: i32, height: i32, planes: i16 },
    UnsupportedPlanes(i16),
    RowStrideTooSmall { row_bytes: i32, needed: i32, which: &'static str },
    RowBufferOverflow { which: &'static str },
}

impl std::fmt::Display for FilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterError::NullPointer(field) => write!(f, "null pointer in field {field}"),
            FilterError::BadDimensions { width, height, planes } => {
                write!(f, "bad dimensions: {width}x{height}, {planes} planes")
            }
            FilterError::UnsupportedPlanes(p) => write!(f, "unsupported plane count {p}"),
            FilterError::RowStrideTooSmall { row_bytes, needed, which } => write!(
                f,
                "{which} row stride {row_bytes} smaller than minimum {needed}"
            ),
            FilterError::RowBufferOverflow { which } => {
                write!(f, "{which} row range would overflow address space")
            }
        }
    }
}

impl std::error::Error for FilterError {}

/// Validated, plain-data view of the host's pixel buffers and their
/// geometry. Constructed via `validate_filter_record`.
pub(crate) struct PixelBuffers {
    pub width:         i32,
    pub height:        i32,
    pub planes:        i16,
    pub in_data:       *mut u8,
    pub in_row_bytes:  i32,
    pub out_data:      *mut u8,
    pub out_row_bytes: i32,
}

/// Validate every field of `FilterRecord` we are about to use and reject
/// anything that smells wrong. This is the only function in the crate that
/// turns raw host-supplied pointers into a typed view; if it returns Ok the
/// rest of the filter can do row-by-row work with normal slice accesses.
pub(crate) fn validate_filter_record(fr: &FilterRecord) -> Result<PixelBuffers, FilterError> {
    let rect   = fr.filter_rect;
    let width  = (rect.right  as i32) - (rect.left as i32);
    let height = (rect.bottom as i32) - (rect.top  as i32);
    let planes = fr.planes;

    if width <= 0 || height <= 0 || width > MAX_EDGE || height > MAX_EDGE {
        return Err(FilterError::BadDimensions { width, height, planes });
    }
    if !(planes == 1 || planes == 3 || planes == 4) {
        return Err(FilterError::UnsupportedPlanes(planes));
    }

    let needed = width
        .checked_mul(planes as i32)
        .ok_or(FilterError::RowBufferOverflow { which: "row width" })?;

    if fr.in_row_bytes < needed {
        return Err(FilterError::RowStrideTooSmall {
            row_bytes: fr.in_row_bytes,
            needed,
            which:     "in_data",
        });
    }
    if fr.out_row_bytes < needed {
        return Err(FilterError::RowStrideTooSmall {
            row_bytes: fr.out_row_bytes,
            needed,
            which:     "out_data",
        });
    }

    // Confirm the (row_bytes * height) computation fits in usize, so that
    // slice::from_raw_parts cannot construct an oversized slice.
    let _ = (fr.in_row_bytes as usize)
        .checked_mul(height as usize)
        .ok_or(FilterError::RowBufferOverflow { which: "in_data" })?;
    let _ = (fr.out_row_bytes as usize)
        .checked_mul(height as usize)
        .ok_or(FilterError::RowBufferOverflow { which: "out_data" })?;

    if fr.in_data.is_null() {
        return Err(FilterError::NullPointer("in_data"));
    }
    if fr.out_data.is_null() {
        return Err(FilterError::NullPointer("out_data"));
    }

    Ok(PixelBuffers {
        width,
        height,
        planes,
        in_data:       fr.in_data,
        in_row_bytes:  fr.in_row_bytes,
        out_data:      fr.out_data,
        out_row_bytes: fr.out_row_bytes,
    })
}

/// M3: copy input pixels to the output buffer, inverting the red channel as
/// a visible "filter ran" marker. Row-stride aware.
pub fn run_passthrough(fr: &mut FilterRecord) -> Result<(), FilterError> {
    let buf = validate_filter_record(fr)?;

    let row_pixels: usize = (buf.width as usize) * (buf.planes as usize);

    for y in 0..buf.height as usize {
        let in_start  = y * (buf.in_row_bytes  as usize);
        let out_start = y * (buf.out_row_bytes as usize);

        let in_row = unsafe {
            std::slice::from_raw_parts(buf.in_data.add(in_start), row_pixels)
        };
        let out_row = unsafe {
            std::slice::from_raw_parts_mut(buf.out_data.add(out_start), row_pixels)
        };

        invert_red_into(in_row, out_row, buf.planes);
    }

    Ok(())
}

fn invert_red_into(src: &[u8], dst: &mut [u8], planes: i16) {
    debug_assert_eq!(src.len(), dst.len());
    let step = planes as usize;
    let mut i = 0;
    while i + step <= src.len() {
        dst[i] = 255 - src[i];
        for ch in 1..step {
            dst[i + ch] = src[i + ch];
        }
        i += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_buffers(width: i32, height: i32, planes: i16, in_row_bytes: i32) -> (Vec<u8>, Vec<u8>) {
        let h = height as usize;
        let rb = in_row_bytes as usize;
        let in_buf  = (0..(rb * h) as usize).map(|i| (i % 251) as u8).collect();
        let out_buf = vec![0u8; rb * h];
        let _ = (width, planes);
        (in_buf, out_buf)
    }

    #[test]
    fn invert_red_rgb_one_row() {
        let src = vec![10u8, 20, 30, 40, 50, 60];
        let mut dst = vec![0u8; 6];
        invert_red_into(&src, &mut dst, 3);
        assert_eq!(dst, vec![245, 20, 30, 215, 50, 60]);
    }

    #[test]
    fn invert_red_rgba_preserves_alpha() {
        let src = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let mut dst = vec![0u8; 8];
        invert_red_into(&src, &mut dst, 4);
        assert_eq!(dst, vec![254, 2, 3, 4, 250, 6, 7, 8]);
    }

    #[test]
    fn passthrough_round_trip_inverts_twice_to_identity() {
        // Two invocations of run_passthrough should return the red channel
        // to its original value (255 - (255 - r) == r).
        let width  = 4i32;
        let height = 2i32;
        let planes = 3i16;
        let row_bytes_in  = (width * planes as i32) + 1; // include stride padding
        let row_bytes_out = row_bytes_in;
        let (input, mut buf_a) = make_buffers(width, height, planes, row_bytes_in);
        let mut buf_b = vec![0u8; buf_a.len()];

        // First pass: invert red into buf_a.
        unsafe {
            let mut fr = std::mem::zeroed::<FilterRecord>();
            fr.filter_rect = crate::ps_types::VRect { top: 0, left: 0, bottom: height as i16, right: width as i16 };
            fr.planes        = planes;
            fr.in_data       = input.as_ptr() as *mut u8;
            fr.in_row_bytes  = row_bytes_in;
            fr.out_data      = buf_a.as_mut_ptr();
            fr.out_row_bytes = row_bytes_out;
            run_passthrough(&mut fr).unwrap();
        }

        // Second pass: invert buf_a's red back into buf_b.
        unsafe {
            let mut fr = std::mem::zeroed::<FilterRecord>();
            fr.filter_rect = crate::ps_types::VRect { top: 0, left: 0, bottom: height as i16, right: width as i16 };
            fr.planes        = planes;
            fr.in_data       = buf_a.as_ptr() as *mut u8;
            fr.in_row_bytes  = row_bytes_in;
            fr.out_data      = buf_b.as_mut_ptr();
            fr.out_row_bytes = row_bytes_out;
            run_passthrough(&mut fr).unwrap();
        }

        // Compare only the live pixel area, ignoring row padding.
        for y in 0..height as usize {
            let row_start = y * row_bytes_in as usize;
            let pixel_len = (width * planes as i32) as usize;
            assert_eq!(
                &buf_b[row_start..row_start + pixel_len],
                &input[row_start..row_start + pixel_len],
                "row {y} not identity after double invert"
            );
        }
    }

    #[test]
    fn rejects_zero_dimensions() {
        unsafe {
            let mut fr = std::mem::zeroed::<FilterRecord>();
            fr.filter_rect = crate::ps_types::VRect { top: 0, left: 0, bottom: 0, right: 0 };
            fr.planes        = 3;
            fr.in_data       = 1 as *mut u8;
            fr.in_row_bytes  = 1;
            fr.out_data      = 1 as *mut u8;
            fr.out_row_bytes = 1;
            assert!(matches!(run_passthrough(&mut fr), Err(FilterError::BadDimensions { .. })));
        }
    }

    #[test]
    fn rejects_absurd_dimensions() {
        unsafe {
            let mut fr = std::mem::zeroed::<FilterRecord>();
            // Width = i16::MAX - (-i16::MAX) = 65534, comfortably > MAX_EDGE.
            fr.filter_rect = crate::ps_types::VRect {
                top:    -i16::MAX,
                left:   -i16::MAX,
                bottom: i16::MAX,
                right:  i16::MAX,
            };
            fr.planes        = 3;
            fr.in_data       = std::ptr::dangling_mut();
            fr.in_row_bytes  = i32::MAX;
            fr.out_data      = std::ptr::dangling_mut();
            fr.out_row_bytes = i32::MAX;
            let err = run_passthrough(&mut fr).unwrap_err();
            assert!(matches!(err, FilterError::BadDimensions { .. }), "got {err:?}");
        }
    }

    #[test]
    fn rejects_null_in_data() {
        unsafe {
            let mut fr = std::mem::zeroed::<FilterRecord>();
            fr.filter_rect = crate::ps_types::VRect { top: 0, left: 0, bottom: 1, right: 1 };
            fr.planes        = 3;
            fr.in_data       = std::ptr::null_mut();
            fr.in_row_bytes  = 3;
            fr.out_data      = 1 as *mut u8;
            fr.out_row_bytes = 3;
            assert!(matches!(run_passthrough(&mut fr), Err(FilterError::NullPointer("in_data"))));
        }
    }

    #[test]
    fn rejects_small_row_stride() {
        unsafe {
            let mut fr = std::mem::zeroed::<FilterRecord>();
            fr.filter_rect = crate::ps_types::VRect { top: 0, left: 0, bottom: 1, right: 4 };
            fr.planes        = 3;
            fr.in_data       = 1 as *mut u8;
            fr.in_row_bytes  = 5; // need 12
            fr.out_data      = 1 as *mut u8;
            fr.out_row_bytes = 12;
            assert!(matches!(run_passthrough(&mut fr), Err(FilterError::RowStrideTooSmall { .. })));
        }
    }
}
