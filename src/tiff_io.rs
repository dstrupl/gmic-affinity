//! Minimal TIFF read/write helpers for the gmic round-trip.
//!
//! Affinity hands us pixels in interleaved 8-bit form with arbitrary row
//! stride padding. G'MIC reads/writes its own TIFFs. These helpers move
//! pixels between the two representations.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::encoder::{colortype, TiffEncoder};

use crate::filter::MAX_EDGE;

/// Decoder limits sized for the images Affinity actually hands us.
///
/// The `tiff` crate's defaults cap an intermediate decoding buffer
/// (`decoding_buffer_size`) at ~256 MiB and `ifd_value_size` at ~1 MiB,
/// which is fine for thumbnails but rejects normal 24 MP+ photographs.
/// `validate_filter_record` already enforces our `MAX_EDGE = 30_000`
/// limit, so the largest legal buffer we'd ever decode is bounded above
/// by `MAX_EDGE * MAX_EDGE * 4 = ~3.6 GB` — way above any reasonable
/// crate default, and the host wouldn't have asked us to filter it in
/// the first place. Use the crate's documented `Limits::unlimited()`
/// (which still validates internal arithmetic for overflow) and rely on
/// our own up-front dimension/buffer-size checks for safety.
fn decoder_limits() -> Limits {
    Limits::unlimited()
}

#[derive(Debug)]
pub enum TiffError {
    Io(std::io::Error),
    Tiff(tiff::TiffError),
    UnsupportedPlanes(u32),
    UnsupportedBitDepth,
    UnexpectedDimensions { got: (u32, u32), want: (u32, u32) },
    DimensionTooLarge(u32),
    SizeMismatch { got: usize, want: usize },
    BufferTooSmall { have: usize, need: usize },
}

impl std::fmt::Display for TiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TiffError::Io(e) => write!(f, "I/O error: {e}"),
            TiffError::Tiff(e) => write!(f, "TIFF error: {e}"),
            TiffError::UnsupportedPlanes(p) => write!(f, "unsupported plane count {p}"),
            TiffError::UnsupportedBitDepth => write!(f, "unsupported bit depth (only 8-bit in v1)"),
            TiffError::UnexpectedDimensions { got, want } => {
                write!(f, "TIFF dimensions {got:?} do not match expected {want:?}")
            }
            TiffError::DimensionTooLarge(d) => write!(f, "TIFF dimension {d} exceeds MAX_EDGE"),
            TiffError::SizeMismatch { got, want } => {
                write!(
                    f,
                    "TIFF pixel buffer size {got} does not match expected {want}"
                )
            }
            TiffError::BufferTooSmall { have, need } => {
                write!(f, "destination buffer too small ({have} < {need})")
            }
        }
    }
}

impl std::error::Error for TiffError {}

impl From<std::io::Error> for TiffError {
    fn from(e: std::io::Error) -> Self {
        TiffError::Io(e)
    }
}
impl From<tiff::TiffError> for TiffError {
    fn from(e: tiff::TiffError) -> Self {
        TiffError::Tiff(e)
    }
}

/// Write an 8-bit interleaved buffer to a TIFF file, removing any host
/// row-stride padding.
pub fn write_tiff(
    path: &Path,
    buf: &[u8],
    width: u32,
    height: u32,
    planes: u32,
    row_bytes: u32,
) -> Result<(), TiffError> {
    if width == 0 || height == 0 || width > MAX_EDGE as u32 || height > MAX_EDGE as u32 {
        return Err(TiffError::DimensionTooLarge(width.max(height)));
    }
    if !(planes == 1 || planes == 3 || planes == 4) {
        return Err(TiffError::UnsupportedPlanes(planes));
    }

    let row_len = (width as usize) * (planes as usize);
    let row_stride = row_bytes as usize;
    if row_stride < row_len {
        return Err(TiffError::SizeMismatch {
            got: row_stride,
            want: row_len,
        });
    }
    let expected_len = row_stride
        .checked_mul(height as usize)
        .ok_or(TiffError::DimensionTooLarge(width.max(height)))?;
    if buf.len() < expected_len {
        return Err(TiffError::BufferTooSmall {
            have: buf.len(),
            need: expected_len,
        });
    }

    let mut packed: Vec<u8> = Vec::with_capacity(row_len * height as usize);
    for y in 0..height as usize {
        let start = y * row_stride;
        packed.extend_from_slice(&buf[start..start + row_len]);
    }

    let file = File::create(path)?;
    let mut enc = TiffEncoder::new(file)?;
    match planes {
        1 => enc.write_image::<colortype::Gray8>(width, height, &packed)?,
        3 => enc.write_image::<colortype::RGB8>(width, height, &packed)?,
        4 => enc.write_image::<colortype::RGBA8>(width, height, &packed)?,
        _ => unreachable!(),
    };
    Ok(())
}

/// Read an 8-bit TIFF file back into the host's interleaved buffer,
/// respecting row stride. Dimensions, channel count, and bit depth must
/// match what we wrote.
pub fn read_tiff(
    path: &Path,
    out_buf: &mut [u8],
    width: u32,
    height: u32,
    planes: u32,
    row_bytes: u32,
) -> Result<(), TiffError> {
    validate_image_shape(width, height, planes)?;

    let file = File::open(path)?;
    let mut dec = Decoder::new(BufReader::new(file))?.with_limits(decoder_limits());
    let (w, h) = dec.dimensions()?;
    validate_dimensions(w, h)?;

    let pixels = decode_image_to_u8(&mut dec)?;
    let pixels = fit_pixels_to_expected_size(pixels, (w, h), (width, height), planes);
    copy_pixels_to_strided_buffer(&pixels, out_buf, width, height, planes, row_bytes)
}

fn validate_image_shape(width: u32, height: u32, planes: u32) -> Result<(), TiffError> {
    validate_dimensions(width, height)?;
    if !(planes == 1 || planes == 3 || planes == 4) {
        return Err(TiffError::UnsupportedPlanes(planes));
    }
    Ok(())
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), TiffError> {
    if width == 0 || height == 0 || width > MAX_EDGE as u32 || height > MAX_EDGE as u32 {
        return Err(TiffError::DimensionTooLarge(width.max(height)));
    }
    Ok(())
}

fn decode_image_to_u8(dec: &mut Decoder<BufReader<File>>) -> Result<Vec<u8>, TiffError> {
    // gmic promotes images to its internal `float` type for almost any
    // non-trivial operation and then writes the result back in whatever
    // pixel type matches that internal representation. Accept anything
    // the tiff crate can decode and quantise it back to 8-bit ourselves.
    match dec.read_image()? {
        DecodingResult::U8(v) => Ok(v),
        DecodingResult::U16(v) => Ok(quantize_u16_to_u8(&v)),
        DecodingResult::U32(v) => Ok(quantize_u32_to_u8(&v)),
        DecodingResult::F32(v) => Ok(quantize_f32_to_u8(&v)),
        DecodingResult::F64(v) => Ok(quantize_f64_to_u8(&v)),
        _ => Err(TiffError::UnsupportedBitDepth),
    }
}

fn fit_pixels_to_expected_size(
    pixels: Vec<u8>,
    got: (u32, u32),
    want: (u32, u32),
    planes: u32,
) -> Vec<u8> {
    // Dimension-mismatch handling (T12-C): many gmic filters change
    // image dimensions. For v1, resize back to the input dims with
    // nearest-neighbour and log a one-line warning.
    if got == want {
        return pixels;
    }

    crate::logging::log(&format!(
        "tiff: gmic returned {}x{}, expected {}x{}; resampling nearest-neighbour",
        got.0, got.1, want.0, want.1,
    ));
    resample_nearest_to_u8(&pixels, got.0, got.1, want.0, want.1, planes)
}

fn copy_pixels_to_strided_buffer(
    pixels: &[u8],
    out_buf: &mut [u8],
    width: u32,
    height: u32,
    planes: u32,
    row_bytes: u32,
) -> Result<(), TiffError> {
    let row_len = (width as usize) * (planes as usize);
    let row_stride = row_bytes as usize;
    if row_stride < row_len {
        return Err(TiffError::SizeMismatch {
            got: row_stride,
            want: row_len,
        });
    }

    let want = row_len * height as usize;
    if pixels.len() != want {
        return Err(TiffError::SizeMismatch {
            got: pixels.len(),
            want,
        });
    }

    let need_out = row_stride * height as usize;
    if out_buf.len() < need_out {
        return Err(TiffError::BufferTooSmall {
            have: out_buf.len(),
            need: need_out,
        });
    }

    for y in 0..height as usize {
        let src = y * row_len;
        let dst = y * row_stride;
        out_buf[dst..dst + row_len].copy_from_slice(&pixels[src..src + row_len]);
    }
    Ok(())
}

/// Quantise a 16-bit unsigned channel buffer to 8-bit by right-shifting 8.
/// This is the standard "high byte" mapping (preserves perceived range
/// without scaling artefacts) and is what every image library does for
/// 16->8 downconversion.
fn quantize_u16_to_u8(v: &[u16]) -> Vec<u8> {
    v.iter().map(|&p| (p >> 8) as u8).collect()
}

fn quantize_u32_to_u8(v: &[u32]) -> Vec<u8> {
    v.iter().map(|&p| (p >> 24) as u8).collect()
}

/// Quantise a float channel buffer to 8-bit by clamping to `[0,255]`
/// and rounding. gmic's float representation of an 8-bit-source image
/// stays in `[0,255]`, so no rescaling is needed.
fn quantize_f32_to_u8(v: &[f32]) -> Vec<u8> {
    v.iter()
        .map(|&p| p.clamp(0.0, 255.0).round() as u8)
        .collect()
}

fn quantize_f64_to_u8(v: &[f64]) -> Vec<u8> {
    v.iter()
        .map(|&p| p.clamp(0.0, 255.0).round() as u8)
        .collect()
}

/// Nearest-neighbour resample of an interleaved 8-bit `src` image
/// from `src_w x src_h` to `dst_w x dst_h` with `planes` interleaved
/// channels. Used to fold a gmic output whose dimensions changed (e.g.
/// `-rotate`, `-crop`) back to the host's filter rect so the existing
/// FilterRecord pipeline can copy it into place without crashing. v2
/// of the plugin will negotiate a new image size with Affinity using
/// the `imageSize` selector instead and drop this helper.
fn resample_nearest_to_u8(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    planes: u32,
) -> Vec<u8> {
    let pl = planes as usize;
    let src_w_us = src_w as usize;
    let dst_w_us = dst_w as usize;
    let dst_h_us = dst_h as usize;
    let mut out = vec![0u8; dst_w_us * dst_h_us * pl];
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return out;
    }
    for y in 0..dst_h_us {
        // floor(y * src_h / dst_h) using u64 to avoid overflow on big
        // images (a 30k x 30k MAX_EDGE image multiplied by another
        // 30k dimension already exceeds u32).
        let sy = (y as u64 * src_h as u64 / dst_h as u64) as usize;
        for x in 0..dst_w_us {
            let sx = (x as u64 * src_w as u64 / dst_w as u64) as usize;
            let src_idx = (sy * src_w_us + sx) * pl;
            let dst_idx = (y * dst_w_us + x) * pl;
            out[dst_idx..dst_idx + pl].copy_from_slice(&src[src_idx..src_idx + pl]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn quantize_f32_clamps_and_rounds() {
        let v = quantize_f32_to_u8(&[-5.0, 0.4, 0.6, 127.5, 254.9, 1000.0]);
        assert_eq!(v, vec![0, 0, 1, 128, 255, 255]);
    }

    #[test]
    fn quantize_u16_takes_high_byte() {
        let v = quantize_u16_to_u8(&[0, 0x00FF, 0x0100, 0x8000, 0xFFFF]);
        assert_eq!(v, vec![0, 0, 1, 0x80, 0xFF]);
    }

    fn make_padded_buffer(width: u32, height: u32, planes: u32, row_bytes: u32) -> Vec<u8> {
        let stride = row_bytes as usize;
        let mut buf = vec![0u8; stride * height as usize];
        for y in 0..height as usize {
            for x in 0..width as usize {
                for p in 0..planes as usize {
                    buf[y * stride + x * planes as usize + p] =
                        ((x * 7 + y * 13 + p * 29) % 251) as u8;
                }
            }
        }
        buf
    }

    fn assert_pixel_area_equal(
        a: &[u8],
        b: &[u8],
        width: u32,
        height: u32,
        planes: u32,
        a_stride: u32,
        b_stride: u32,
    ) {
        let row_len = (width * planes) as usize;
        for y in 0..height as usize {
            let as_ = y * a_stride as usize;
            let bs = y * b_stride as usize;
            assert_eq!(
                &a[as_..as_ + row_len],
                &b[bs..bs + row_len],
                "row {y} differs"
            );
        }
    }

    #[test]
    fn round_trip_rgb_with_padding() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("rgb.tif");
        let (w, h, p) = (10u32, 4u32, 3u32);
        let in_stride = w * p + 2; // padding
        let out_stride = w * p + 5; // different padding on read side
        let in_buf = make_padded_buffer(w, h, p, in_stride);

        write_tiff(&path, &in_buf, w, h, p, in_stride).unwrap();

        let mut out_buf = vec![0xCCu8; (out_stride * h) as usize];
        read_tiff(&path, &mut out_buf, w, h, p, out_stride).unwrap();

        assert_pixel_area_equal(&in_buf, &out_buf, w, h, p, in_stride, out_stride);
    }

    #[test]
    fn round_trip_rgba_no_padding() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("rgba.tif");
        let (w, h, p) = (8u32, 8u32, 4u32);
        let stride = w * p;
        let in_buf = make_padded_buffer(w, h, p, stride);

        write_tiff(&path, &in_buf, w, h, p, stride).unwrap();

        let mut out_buf = vec![0u8; in_buf.len()];
        read_tiff(&path, &mut out_buf, w, h, p, stride).unwrap();

        assert_eq!(in_buf, out_buf);
    }

    #[test]
    fn rejects_unsupported_planes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.tif");
        let buf = vec![0u8; 8];
        assert!(matches!(
            write_tiff(&path, &buf, 2, 2, 2, 4),
            Err(TiffError::UnsupportedPlanes(2))
        ));
    }

    #[test]
    fn dimension_mismatch_resamples_to_target_size() {
        // Pre-T12-C this case returned `TiffError::UnexpectedDimensions`.
        // After T12-C we silently nearest-neighbour resample back to
        // the host's expected dims so gmic filters that change size
        // (rotate / crop / spread / …) don't crash the plugin.
        let dir = tempdir().unwrap();
        let path = dir.path().join("mismatch.tif");
        let buf = make_padded_buffer(4, 4, 3, 12);
        write_tiff(&path, &buf, 4, 4, 3, 12).unwrap();

        let mut out = vec![0xCC_u8; 8 * 8 * 3];
        // Reading a 4x4 source into an 8x8 destination must succeed
        // and produce *some* RGB pixel content (not the 0xCC sentinel).
        read_tiff(&path, &mut out, 8, 8, 3, 8 * 3).unwrap();
        assert!(
            out.iter().any(|&b| b != 0xCC),
            "resampled output must overwrite the sentinel"
        );
    }
}

#[cfg(test)]
mod resample_tests {
    use super::*;

    #[test]
    fn identity_passthrough() {
        let src = vec![1u8, 2, 3, 4];
        let out = resample_nearest_to_u8(&src, 2, 2, 2, 2, 1);
        assert_eq!(out, src);
    }

    #[test]
    fn shrink_by_half() {
        // 4x1 → 2x1, single channel, nearest picks indices 0,2
        let src = vec![10, 20, 30, 40];
        let out = resample_nearest_to_u8(&src, 4, 1, 2, 1, 1);
        assert_eq!(out, vec![10, 30]);
    }

    #[test]
    fn upsample_doubles() {
        let src = vec![5, 9];
        let out = resample_nearest_to_u8(&src, 2, 1, 4, 1, 1);
        assert_eq!(out, vec![5, 5, 9, 9]);
    }

    #[test]
    fn rgb_planes_are_preserved() {
        // 2x1 rgb, shrink to 1x1, expected first pixel
        let src = vec![1, 2, 3, 4, 5, 6];
        let out = resample_nearest_to_u8(&src, 2, 1, 1, 1, 3);
        assert_eq!(out, vec![1, 2, 3]);
    }

    #[test]
    fn zero_dimension_returns_empty() {
        let src = vec![1u8, 2, 3, 4];
        let out = resample_nearest_to_u8(&src, 0, 0, 2, 2, 1);
        assert_eq!(out, vec![0, 0, 0, 0]); // dst-sized but unwritten
    }
}
