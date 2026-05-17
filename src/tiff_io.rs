//! Minimal TIFF read/write helpers for the gmic round-trip.
//!
//! Affinity hands us pixels in interleaved 8-bit form with arbitrary row
//! stride padding. G'MIC reads/writes its own TIFFs. These helpers move
//! pixels between the two representations.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use tiff::decoder::{Decoder, DecodingResult};
use tiff::encoder::{colortype, TiffEncoder};

use crate::filter::MAX_EDGE;

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
            TiffError::Io(e)                    => write!(f, "I/O error: {e}"),
            TiffError::Tiff(e)                  => write!(f, "TIFF error: {e}"),
            TiffError::UnsupportedPlanes(p)     => write!(f, "unsupported plane count {p}"),
            TiffError::UnsupportedBitDepth      => write!(f, "unsupported bit depth (only 8-bit in v1)"),
            TiffError::UnexpectedDimensions { got, want } => write!(
                f,
                "TIFF dimensions {got:?} do not match expected {want:?}"
            ),
            TiffError::DimensionTooLarge(d)     => write!(f, "TIFF dimension {d} exceeds MAX_EDGE"),
            TiffError::SizeMismatch { got, want } => {
                write!(f, "TIFF pixel buffer size {got} does not match expected {want}")
            }
            TiffError::BufferTooSmall { have, need } => {
                write!(f, "destination buffer too small ({have} < {need})")
            }
        }
    }
}

impl std::error::Error for TiffError {}

impl From<std::io::Error>  for TiffError { fn from(e: std::io::Error)  -> Self { TiffError::Io(e) } }
impl From<tiff::TiffError> for TiffError { fn from(e: tiff::TiffError) -> Self { TiffError::Tiff(e) } }

/// Write an 8-bit interleaved buffer to a TIFF file, removing any host
/// row-stride padding.
pub fn write_tiff(
    path:      &Path,
    buf:       &[u8],
    width:     u32,
    height:    u32,
    planes:    u32,
    row_bytes: u32,
) -> Result<(), TiffError> {
    if width == 0 || height == 0 || width > MAX_EDGE as u32 || height > MAX_EDGE as u32 {
        return Err(TiffError::DimensionTooLarge(width.max(height)));
    }
    if !(planes == 1 || planes == 3 || planes == 4) {
        return Err(TiffError::UnsupportedPlanes(planes));
    }

    let row_len    = (width as usize) * (planes as usize);
    let row_stride = row_bytes as usize;
    if row_stride < row_len {
        return Err(TiffError::SizeMismatch { got: row_stride, want: row_len });
    }
    let expected_len = row_stride
        .checked_mul(height as usize)
        .ok_or(TiffError::DimensionTooLarge(width.max(height)))?;
    if buf.len() < expected_len {
        return Err(TiffError::BufferTooSmall { have: buf.len(), need: expected_len });
    }

    let mut packed: Vec<u8> = Vec::with_capacity(row_len * height as usize);
    for y in 0..height as usize {
        let start = y * row_stride;
        packed.extend_from_slice(&buf[start..start + row_len]);
    }

    let file     = File::create(path)?;
    let mut enc  = TiffEncoder::new(file)?;
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
    path:      &Path,
    out_buf:   &mut [u8],
    width:     u32,
    height:    u32,
    planes:    u32,
    row_bytes: u32,
) -> Result<(), TiffError> {
    if width == 0 || height == 0 || width > MAX_EDGE as u32 || height > MAX_EDGE as u32 {
        return Err(TiffError::DimensionTooLarge(width.max(height)));
    }
    if !(planes == 1 || planes == 3 || planes == 4) {
        return Err(TiffError::UnsupportedPlanes(planes));
    }

    let file     = File::open(path)?;
    let mut dec  = Decoder::new(BufReader::new(file))?;
    let (w, h)   = dec.dimensions()?;
    if w > MAX_EDGE as u32 || h > MAX_EDGE as u32 {
        return Err(TiffError::DimensionTooLarge(w.max(h)));
    }
    if (w, h) != (width, height) {
        return Err(TiffError::UnexpectedDimensions { got: (w, h), want: (width, height) });
    }

    let pixels: Vec<u8> = match dec.read_image()? {
        DecodingResult::U8(v) => v,
        _                     => return Err(TiffError::UnsupportedBitDepth),
    };

    let row_len = (width as usize) * (planes as usize);
    let want    = row_len * height as usize;
    if pixels.len() != want {
        return Err(TiffError::SizeMismatch { got: pixels.len(), want });
    }
    let need_out = (row_bytes as usize) * (height as usize);
    if out_buf.len() < need_out {
        return Err(TiffError::BufferTooSmall { have: out_buf.len(), need: need_out });
    }

    for y in 0..height as usize {
        let src = y * row_len;
        let dst = y * (row_bytes as usize);
        out_buf[dst..dst + row_len].copy_from_slice(&pixels[src..src + row_len]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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
        a: &[u8], b: &[u8],
        width: u32, height: u32, planes: u32,
        a_stride: u32, b_stride: u32,
    ) {
        let row_len = (width * planes) as usize;
        for y in 0..height as usize {
            let as_ = y * a_stride as usize;
            let bs  = y * b_stride as usize;
            assert_eq!(
                &a[as_..as_ + row_len],
                &b[bs..bs + row_len],
                "row {y} differs"
            );
        }
    }

    #[test]
    fn round_trip_rgb_with_padding() {
        let dir   = tempdir().unwrap();
        let path  = dir.path().join("rgb.tif");
        let (w, h, p) = (10u32, 4u32, 3u32);
        let in_stride  = w * p + 2; // padding
        let out_stride = w * p + 5; // different padding on read side
        let in_buf = make_padded_buffer(w, h, p, in_stride);

        write_tiff(&path, &in_buf, w, h, p, in_stride).unwrap();

        let mut out_buf = vec![0xCCu8; (out_stride * h) as usize];
        read_tiff(&path, &mut out_buf, w, h, p, out_stride).unwrap();

        assert_pixel_area_equal(&in_buf, &out_buf, w, h, p, in_stride, out_stride);
    }

    #[test]
    fn round_trip_rgba_no_padding() {
        let dir  = tempdir().unwrap();
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
        let dir  = tempdir().unwrap();
        let path = dir.path().join("bad.tif");
        let buf  = vec![0u8; 8];
        assert!(matches!(
            write_tiff(&path, &buf, 2, 2, 2, 4),
            Err(TiffError::UnsupportedPlanes(2))
        ));
    }

    #[test]
    fn rejects_dimension_mismatch_on_read() {
        let dir  = tempdir().unwrap();
        let path = dir.path().join("mismatch.tif");
        let buf  = make_padded_buffer(4, 4, 3, 12);
        write_tiff(&path, &buf, 4, 4, 3, 12).unwrap();

        let mut out = vec![0u8; 8 * 8 * 3];
        let err = read_tiff(&path, &mut out, 8, 8, 3, 24).unwrap_err();
        assert!(matches!(err, TiffError::UnexpectedDimensions { .. }), "got {err}");
    }
}
