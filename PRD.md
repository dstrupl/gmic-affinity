# PRD: gmic-affinity — A Rust Photoshop Filter Plugin Bridging G'MIC and Affinity Photo

**Version:** 0.1  
**Status:** Draft  
**Target platform:** macOS (Apple Silicon + Intel), Affinity Photo 2 and later (including Affinity by Canva v3)

---

## 1. Background and Motivation

G'MIC (GREYC's Magic for Image Computing) is a powerful open-source image processing framework with hundreds of filters, available on macOS via Homebrew (`brew install gmic-qt`). Affinity Photo is a professional image editor that supports Photoshop-compatible filter plugins (`.plugin` bundles) on macOS.

As of 2026, no native macOS Photoshop-compatible plugin exists for G'MIC. The `gmic-8bf` project (Windows only) provides this on Windows via a `.8bf` DLL, but its author confirmed that a macOS port requires substantial OS-specific rewriting. This project fills that gap.

The goal is a minimal, self-contained Rust crate that compiles into a macOS `.plugin` bundle, appears in Affinity Photo's **Filters** menu, and when invoked:

1. Receives the current pixel data from Affinity
2. Writes it to a temporary TIFF
3. Invokes the Homebrew-installed `gmic` binary with a configurable filter
4. Reads the result back
5. Returns the modified pixels to Affinity — all inline, with no manual file roundtrip

This works with **Affinity Photo 2** and all later versions including **Affinity by Canva (v3)**, because the Photoshop plugin interface is stable across those versions.

---

## 2. Goals

- **G1** — Appears as a filter entry in Affinity Photo's Filters menu without requiring a Canva account or Claude Desktop
- **G2** — Works on both Apple Silicon (ARM64) and Intel (x86\_64) Macs as a universal binary
- **G3** — Written entirely in Rust; no C++ compilation required
- **G4** — Invokes the system `gmic` binary installed by Homebrew at `/opt/homebrew/bin/gmic` (ARM) or `/usr/local/bin/gmic` (Intel)
- **G5** — Operates inline: Affinity hands in pixels, plugin returns modified pixels, no separate open-file step
- **G6** — Self-contained: no additional runtime dependencies beyond `gmic` itself

## 3. Non-Goals

- A graphical parameter UI (the initial version will use a hardcoded or config-file-based filter command)
- Windows support
- Support for tiled/chunked processing (whole-image only in v1)
- Distribution via any plugin marketplace
- Integration with G'MIC-Qt's interactive GUI (future work)

---

## 4. How Photoshop-Compatible Filter Plugins Work on macOS

### 4.1 Plugin format

On macOS, a Photoshop-compatible filter plugin is a **bundle directory** with the extension `.plugin`:

```
GmicFilter.plugin/
  Contents/
    MacOS/
      GmicFilter          ← compiled universal dylib (no .dylib extension)
    Resources/
      GmicFilter.rsrc     ← (optional) resource fork for older hosts
    Info.plist            ← declares plugin type and metadata
```

The `Info.plist` must include:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>GmicFilter</string>
  <key>CFBundleIdentifier</key>
  <string>com.yourname.gmic-affinity</string>
  <key>CFBundleName</key>
  <string>G'MIC</string>
  <key>CFBundlePackageType</key>
  <string>8BFM</string>      <!-- 8BFM = filter plugin -->
  <key>CFBundleShortVersionString</key>
  <string>1.0.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
</dict>
</plist>
```

The `CFBundlePackageType` value `8BFM` identifies this as a filter plugin. Affinity Photo reads this to discover plugins in its plugins folder.

### 4.2 Entry point

The host (Affinity) calls a single exported C function:

```c
void PluginMain(
    const int16_t  selector,
    FilterRecord  *filterRecord,
    intptr_t      *data,
    int16_t       *result
);
```

`selector` is an integer that tells the plugin what phase of execution is being requested. The key selectors are:

| Selector value | Name | Purpose |
|---|---|---|
| 0 | `filterSelectorParameters` | Show parameter UI (optional) |
| 1 | `filterSelectorPrepare` | Declare memory requirements |
| 2 | `filterSelectorStart` | Request image data from host |
| 3 | `filterSelectorContinue` | Process the data (main work) |
| 4 | `filterSelectorFinish` | Clean up |

`result` is set to `0` (noErr) on success, or a non-zero error code on failure.

### 4.3 The FilterRecord

The host passes a pointer to a `FilterRecord` struct containing all image metadata and pixel data pointers. The fields you need for a simple whole-image filter are:

| Field | Type | Meaning |
|---|---|---|
| `image_mode` | `i16` | Colour mode (3 = RGB, 1 = Greyscale, etc.) |
| `image_size` | `VRect` | Full image dimensions |
| `filter_rect` | `VRect` | Region the filter should process |
| `planes` | `i16` | Number of channels (3 = RGB, 4 = RGBA) |
| `in_rect` | `VRect` | ← set by plugin: what rect to request |
| `in_lo_plane` | `i16` | ← set by plugin: first channel to request |
| `in_hi_plane` | `i16` | ← set by plugin: last channel to request |
| `in_data` | `*mut u8` | → filled by host: pointer to input pixels |
| `in_row_bytes` | `i32` | → filled by host: bytes per input row |
| `out_rect` | `VRect` | ← set by plugin: what rect to write |
| `out_lo_plane` | `i16` | ← set by plugin: first output channel |
| `out_hi_plane` | `i16` | ← set by plugin: last output channel |
| `out_data` | `*mut u8` | → filled by host: pointer to output buffer |
| `out_row_bytes` | `i32` | → filled by host: bytes per output row |

The pixel layout is **interleaved**: for RGB, each row is `[R G B R G B R G B ...]`. Row stride is `in_row_bytes` (may be larger than `width * planes` due to alignment).

**Important:** The `FilterRecord` struct in the full Adobe SDK has approximately 80 fields. You only need to define up to the fields you use. Pad the rest with a `[u8; N]` array. See Section 6.1 for the complete Rust definition.

### 4.4 Plugin installation in Affinity Photo

Copy the `.plugin` bundle to:

```
~/Library/Application Support/Affinity Photo 2/Plugins/
```

For Affinity v3 (by Canva):

```
~/Library/Application Support/Affinity/Plugins/
```

In Affinity Photo, go to **Edit → Preferences → Photoshop Plugins** (v2) or the equivalent in v3, and ensure **"Allow unknown plugins to be used"** is checked. The filter then appears under **Filters → Plugins → G'MIC** (the category is set via the resource fork or pipl resource — see Section 6.5).

### 4.5 Code signing

macOS requires all executables loaded into other processes to be signed. For personal/local use, an ad-hoc signature is sufficient and avoids the $99/year Apple Developer Program requirement:

```bash
codesign --force --deep --sign - GmicFilter.plugin
```

If Affinity refuses an ad-hoc-signed plugin, you will need a proper Apple Developer ID certificate and notarization. Test ad-hoc first.

---

## 5. G'MIC Integration

### 5.1 Homebrew binary locations

| Architecture | Path |
|---|---|
| Apple Silicon (ARM64) | `/opt/homebrew/bin/gmic` |
| Intel (x86\_64) | `/usr/local/bin/gmic` |

The plugin should detect which path exists at runtime, or accept a path override from a config file.

Detection logic (Rust pseudocode):
```rust
fn gmic_path() -> &'static str {
    if std::path::Path::new("/opt/homebrew/bin/gmic").exists() {
        "/opt/homebrew/bin/gmic"
    } else {
        "/usr/local/bin/gmic"
    }
}
```

### 5.2 Invocation

G'MIC is invoked as a subprocess. A typical call:

```bash
gmic /tmp/gmic_in.tif \
     -your_filter param1 param2 \
     -output /tmp/gmic_out.tif
```

In Rust:
```rust
std::process::Command::new(gmic_path())
    .args([
        "/tmp/gmic_in.tif",
        "-your_filter", "param1",
        "-output", "/tmp/gmic_out.tif",
    ])
    .status()?;
```

**Note:** `std::process::Command` correctly inherits the environment. Use `.env_clear()` with explicit paths if you need a clean environment.

### 5.3 Temporary files

Use a unique temp path per invocation to avoid collisions:

```rust
use std::time::{SystemTime, UNIX_EPOCH};
let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
let tmp_in  = format!("/tmp/gmic_affinity_in_{}.tif", ts);
let tmp_out = format!("/tmp/gmic_affinity_out_{}.tif", ts);
```

Clean up both files in `filterSelectorFinish` or after a successful read.

### 5.4 Filter configuration (v1)

For the initial version, hardcode a single filter command defined as a Rust constant, or read from `~/.config/gmic-affinity/filter.txt` if it exists. A proper parameter UI (passed via `filterSelectorParameters`) is deferred to v2.

---

## 6. Rust Implementation

### 6.1 Cargo.toml

```toml
[package]
name    = "gmic-affinity"
version = "0.1.0"
edition = "2021"

[lib]
name      = "GmicFilter"
crate-type = ["cdylib"]

[dependencies]
libc = "0.2"
tiff = "0.9"
```

### 6.2 FilterRecord and supporting types (src/ps_types.rs)

Define only the fields needed, padded to cover the full struct size. The full `FilterRecord` in the 2021 SDK is 796 bytes on 64-bit macOS. Pad with a trailing `[u8; N]` to ensure pointer arithmetic by the host is correct.

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VRect {
    pub top:    i16,
    pub left:   i16,
    pub bottom: i16,
    pub right:  i16,
}

// Callback function types
pub type TestAbortProc  = Option<unsafe extern "C" fn() -> i16>;
pub type ProgressProc   = Option<unsafe extern "C" fn(i32, i32)>;

/// Partial FilterRecord — fields up to out_row_bytes only.
/// The remaining ~600 bytes are covered by the trailing pad.
/// IMPORTANT: verify struct size against SDK if behaviour is wrong.
#[repr(C)]
pub struct FilterRecord {
    pub serial_number:      i32,
    pub abort_proc:         TestAbortProc,
    pub progress_proc:      ProgressProc,
    pub parameters:         *mut std::ffi::c_void,
    pub background:         [u8; 6],      // RGBColor
    pub foreground:         [u8; 6],      // RGBColor
    pub buffer_space:       i32,
    pub max_space:          i32,
    pub image_modes_allowed: i32,
    pub filter_rect:        VRect,
    pub image_mode:         i16,
    pub image_size:         VRect,
    pub planes:             i16,
    pub in_rect:            VRect,
    pub in_lo_plane:        i16,
    pub in_hi_plane:        i16,
    pub out_rect:           VRect,
    pub out_lo_plane:       i16,
    pub out_hi_plane:       i16,
    pub in_data:            *mut u8,
    pub in_row_bytes:       i32,
    pub out_data:           *mut u8,
    pub out_row_bytes:      i32,
    // Remaining SDK fields — do not access, just pad for correct size
    _pad: [u8; 700],
}
```

> **Verify struct layout:** Open the Adobe Photoshop SDK header `PIFilter.h` and check the field offsets of `inData` and `outData` against your Rust struct using `std::mem::offset_of!()` in a test. Misalignment here is the most common source of crashes.

### 6.3 Main plugin entry point (src/lib.rs)

```rust
mod ps_types;
mod filter;
mod tiff_io;

use ps_types::FilterRecord;

const SELECTOR_ABOUT:      i16 = 0;
const SELECTOR_PARAMETERS: i16 = 1;
const SELECTOR_PREPARE:    i16 = 2;
const SELECTOR_START:      i16 = 3;
const SELECTOR_CONTINUE:   i16 = 4;
const SELECTOR_FINISH:     i16 = 5;

const NO_ERR:   i16 = 0;
const USER_CANCEL: i16 = -1;

#[no_mangle]
pub unsafe extern "C" fn PluginMain(
    selector:      i16,
    filter_record: *mut FilterRecord,
    _data:         *mut isize,
    result:        *mut i16,
) {
    let fr = &mut *filter_record;

    *result = match selector {
        SELECTOR_ABOUT => {
            // Optionally show an about dialog via NSAlert or just do nothing
            NO_ERR
        }
        SELECTOR_PARAMETERS => {
            // No UI in v1 — just return OK
            NO_ERR
        }
        SELECTOR_PREPARE => {
            fr.buffer_space = 0;
            fr.max_space    = 0;
            NO_ERR
        }
        SELECTOR_START => {
            // Tell host: we want the whole image, all planes
            fr.in_rect      = fr.filter_rect;
            fr.in_lo_plane  = 0;
            fr.in_hi_plane  = fr.planes - 1;
            fr.out_rect     = fr.filter_rect;
            fr.out_lo_plane = 0;
            fr.out_hi_plane = fr.planes - 1;
            NO_ERR
        }
        SELECTOR_CONTINUE => {
            match filter::run(fr) {
                Ok(()) => {
                    // Signal to host that we are done (no more tiles)
                    fr.in_rect  = ps_types::VRect { top: 0, left: 0, bottom: 0, right: 0 };
                    fr.out_rect = ps_types::VRect { top: 0, left: 0, bottom: 0, right: 0 };
                    NO_ERR
                }
                Err(_) => USER_CANCEL,
            }
        }
        SELECTOR_FINISH => NO_ERR,
        _ => NO_ERR,
    };
}
```

### 6.4 Filter logic (src/filter.rs)

```rust
use crate::ps_types::FilterRecord;
use crate::tiff_io;
use std::process::Command;

pub fn run(fr: &mut FilterRecord) -> Result<(), Box<dyn std::error::Error>> {
    let ts  = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .subsec_nanos();
    let tmp_in  = format!("/tmp/gmic_affinity_in_{}.tif",  ts);
    let tmp_out = format!("/tmp/gmic_affinity_out_{}.tif", ts);

    // Extract image dimensions from the filter rect
    let rect   = fr.filter_rect;
    let width  = (rect.right  - rect.left) as u32;
    let height = (rect.bottom - rect.top)  as u32;
    let planes = fr.planes as u32;

    // Safety: host guarantees in_data is valid for this rect
    let in_buf = unsafe {
        std::slice::from_raw_parts(
            fr.in_data,
            (fr.in_row_bytes as u32 * height) as usize,
        )
    };

    tiff_io::write_tiff(&tmp_in, in_buf, width, height, planes, fr.in_row_bytes as u32)?;

    // Run gmic
    let gmic = gmic_path();
    let status = Command::new(gmic)
        .args(gmic_args(&tmp_in, &tmp_out))
        .status()?;

    if !status.success() {
        let _ = std::fs::remove_file(&tmp_in);
        return Err("gmic exited with non-zero status".into());
    }

    // Read result back into out_data
    let out_buf = unsafe {
        std::slice::from_raw_parts_mut(
            fr.out_data,
            (fr.out_row_bytes as u32 * height) as usize,
        )
    };

    tiff_io::read_tiff(&tmp_out, out_buf, width, height, planes, fr.out_row_bytes as u32)?;

    let _ = std::fs::remove_file(&tmp_in);
    let _ = std::fs::remove_file(&tmp_out);

    Ok(())
}

fn gmic_path() -> &'static str {
    if std::path::Path::new("/opt/homebrew/bin/gmic").exists() {
        "/opt/homebrew/bin/gmic"
    } else {
        "/usr/local/bin/gmic"
    }
}

/// Build the gmic argument list.
/// Reads from ~/.config/gmic-affinity/filter.txt if present,
/// otherwise falls back to a default filter.
fn gmic_args<'a>(input: &'a str, output: &'a str) -> Vec<String> {
    let filter_cmd = read_filter_config()
        .unwrap_or_else(|| "-fx_rodilius 10,2,200,20,3,0".to_string()); // default

    let mut args: Vec<String> = vec![input.to_string()];
    args.extend(filter_cmd.split_whitespace().map(String::from));
    args.push("-output".to_string());
    args.push(output.to_string());
    args
}

fn read_filter_config() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = format!("{}/.config/gmic-affinity/filter.txt", home);
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}
```

### 6.5 TIFF I/O (src/tiff_io.rs)

```rust
use tiff::encoder::{TiffEncoder, colortype};
use tiff::decoder::{Decoder, DecodingResult};
use std::io::Cursor;

pub fn write_tiff(
    path:       &str,
    buf:        &[u8],
    width:      u32,
    height:     u32,
    planes:     u32,
    row_bytes:  u32,
) -> Result<(), Box<dyn std::error::Error>> {
    // Re-pack rows removing any stride padding
    let row_len = (width * planes) as usize;
    let mut packed = Vec::with_capacity((row_len * height as usize) as usize);
    for row in 0..height as usize {
        let start = row * row_bytes as usize;
        packed.extend_from_slice(&buf[start..start + row_len]);
    }

    let file = std::fs::File::create(path)?;
    let mut tiff = TiffEncoder::new(file)?;

    match planes {
        1 => tiff.write_image::<colortype::Gray8>(width, height, &packed)?,
        3 => tiff.write_image::<colortype::RGB8>(width, height, &packed)?,
        4 => tiff.write_image::<colortype::RGBA8>(width, height, &packed)?,
        _ => return Err(format!("Unsupported plane count: {}", planes).into()),
    }
    Ok(())
}

pub fn read_tiff(
    path:      &str,
    out_buf:   &mut [u8],
    width:     u32,
    height:    u32,
    planes:    u32,
    row_bytes: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let file    = std::fs::File::open(path)?;
    let mut dec = Decoder::new(file)?;
    let result  = dec.read_image()?;

    let pixels: Vec<u8> = match result {
        DecodingResult::U8(v) => v,
        _ => return Err("Expected 8-bit TIFF output from gmic".into()),
    };

    // Write back into out_buf, respecting row stride
    let row_len = (width * planes) as usize;
    for row in 0..height as usize {
        let src_start = row * row_len;
        let dst_start = row * row_bytes as usize;
        out_buf[dst_start..dst_start + row_len]
            .copy_from_slice(&pixels[src_start..src_start + row_len]);
    }
    Ok(())
}
```

### 6.6 pipl resource (filter name and category in Affinity's menu)

Affinity Photo reads the filter's name and menu category from a `pipl` resource embedded in the binary. The `pipl` is a binary blob defined in the Adobe SDK. For a macOS plugin, the minimal approach is to embed it as a raw byte array in the binary via a linker section, or to ship a `.rsrc` file in `Contents/Resources/`.

The easiest path for v1: ship a pre-built `GmicFilter.rsrc` compiled from this Rez source:

```
#include "PIDefines.h"
#include "PITypes.r"
#include "PIGeneral.r"

resource 'pipl' (ResourceID, purgeable) {
    {
        Kind { Filter },
        Name { "G'MIC..." },
        Category { "G'MIC" },
        Version { (latestFilterVersion, latestFilterSubVersion) },
        SupportedModes { noBitmap, noGrayScale, noIndexedColor,
                         doesRGB, noHSL, noHSB, noCMYK,
                         noLab, noMultichannel, noDuotone,
                         noRGB48, noGray16 },
        HasTerminology { plugInClassFilter, plugInEventFilter,
                         ResourceID, "" },
    }
};
```

Compile with the `Rez` tool from Xcode's command-line tools:

```bash
Rez -o GmicFilter.rsrc GmicFilter.r -i /path/to/PhotoshopSDK/Resources/
```

If you want to skip the Rez step entirely for testing, Affinity may still load and run the plugin but it may appear under a default/unnamed category. Experiment to confirm.

---

## 7. Build Process

### 7.1 Compile for both architectures

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin

cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin
```

### 7.2 Create universal binary and bundle

```bash
# Create bundle structure
mkdir -p GmicFilter.plugin/Contents/MacOS
mkdir -p GmicFilter.plugin/Contents/Resources

# Merge into universal binary
lipo -create \
  target/aarch64-apple-darwin/release/libGmicFilter.dylib \
  target/x86_64-apple-darwin/release/libGmicFilter.dylib \
  -output GmicFilter.plugin/Contents/MacOS/GmicFilter

# Copy resources
cp Info.plist          GmicFilter.plugin/Contents/
cp GmicFilter.rsrc     GmicFilter.plugin/Contents/Resources/  # if built

# Ad-hoc sign (for local use; replace with Developer ID for distribution)
codesign --force --deep --sign - GmicFilter.plugin
```

Alternatively, use `cargo-lipo` to simplify the two-target build:

```bash
cargo install cargo-lipo
cargo lipo --release
# output: target/universal/release/libGmicFilter.dylib
```

### 7.3 Install

```bash
# Affinity Photo 2
cp -r GmicFilter.plugin \
  ~/Library/Application\ Support/Affinity\ Photo\ 2/Plugins/

# Affinity v3 (by Canva)
cp -r GmicFilter.plugin \
  ~/Library/Application\ Support/Affinity/Plugins/
```

Restart Affinity Photo. Enable unknown plugins in preferences if prompted. The filter appears under **Filters → Plugins → G'MIC → G'MIC...**.

### 7.4 Suggested Makefile

```makefile
BUNDLE   = GmicFilter.plugin
BINARY   = $(BUNDLE)/Contents/MacOS/GmicFilter
PLIST    = Info.plist

.PHONY: all bundle install clean

all: bundle

bundle:
	cargo build --release --target aarch64-apple-darwin
	cargo build --release --target x86_64-apple-darwin
	mkdir -p $(BUNDLE)/Contents/MacOS $(BUNDLE)/Contents/Resources
	lipo -create \
		target/aarch64-apple-darwin/release/libGmicFilter.dylib \
		target/x86_64-apple-darwin/release/libGmicFilter.dylib \
		-output $(BINARY)
	cp $(PLIST) $(BUNDLE)/Contents/
	codesign --force --deep --sign - $(BUNDLE)

install: bundle
	cp -r $(BUNDLE) \
		"$(HOME)/Library/Application Support/Affinity Photo 2/Plugins/"

clean:
	cargo clean
	rm -rf $(BUNDLE)
```

---

## 8. Struct Alignment Verification

Before doing any real work, write a test that asserts the byte offsets of `in_data` and `out_data` in your `FilterRecord` match the SDK header. Add to `src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::ps_types::FilterRecord;
    use std::mem::{offset_of, size_of};

    #[test]
    fn filter_record_layout() {
        // These values must match PIFilter.h from the Adobe Photoshop SDK.
        // Check the SDK header for your target SDK version.
        // The values below are for the 2021 macOS 64-bit SDK.
        assert_eq!(offset_of!(FilterRecord, in_data),      88,  "in_data offset");
        assert_eq!(offset_of!(FilterRecord, in_row_bytes), 96,  "in_row_bytes offset");
        assert_eq!(offset_of!(FilterRecord, out_data),     104, "out_data offset");
        assert_eq!(offset_of!(FilterRecord, out_row_bytes),112, "out_row_bytes offset");
    }

    #[test]
    fn vect_is_8_bytes() {
        assert_eq!(size_of::<super::ps_types::VRect>(), 8);
    }
}
```

> **Run `cargo test` and fix any offset mismatches before proceeding.**  
> Wrong offsets produce crashes or silent pixel corruption inside Affinity.

---

## 9. Known Limitations and Future Work

| Item | Notes |
|---|---|
| 8-bit RGB/RGBA only | 16-bit and 32-bit images are not handled in v1 |
| No parameter UI | Filter command is hardcoded or read from config file |
| Whole-image only | No tiled processing; may be slow or fail on very large images |
| Single filter only | One gmic command per plugin invocation |
| pipl resource | Optional for loading; required for correct menu category name |
| Code signing | Ad-hoc signing works locally; Apple Developer ID required for distribution |
| gmic-qt GUI | Could launch `gmic_qt` standalone for interactive filter selection (v2 idea) |
| Affinity v3 scripting | The v3 MCP/Scripts panel could invoke this plugin programmatically |

---

## 10. Related Resources

| Resource | URL |
|---|---|
| Adobe Photoshop SDK | https://console.adobe.io → Downloads → Creative Cloud → Photoshop C++ SDK |
| G'MIC documentation | https://gmic.eu/reference/ |
| G'MIC Homebrew formula | `brew info gmic-qt` |
| gmic-8bf (Windows reference implementation) | https://github.com/0xC0000054/gmic-8bf |
| Affinity plugin preferences | Affinity Photo → Edit → Preferences → Photoshop Plugins |
| `tiff` crate | https://crates.io/crates/tiff |
| `cargo-lipo` | https://crates.io/crates/cargo-lipo |
| `bindgen` (optional, for full SDK bindings) | https://crates.io/crates/bindgen |
| rabidgremlin/affinity-scripting | https://github.com/rabidgremlin/affinity-scripting |
| tacyan/AffinityMCP | https://github.com/tacyan/AffinityMCP |

---

## 11. Recommended Project Structure

```
gmic-affinity/
├── Cargo.toml
├── Makefile
├── Info.plist
├── GmicFilter.r          # Rez source for pipl resource (optional)
├── src/
│   ├── lib.rs            # PluginMain entry point
│   ├── ps_types.rs       # FilterRecord and VRect definitions
│   ├── filter.rs         # gmic invocation logic
│   └── tiff_io.rs        # TIFF read/write via the tiff crate
├── tests/
│   └── layout.rs         # Struct offset verification tests
└── README.md
```

---

*End of PRD*
