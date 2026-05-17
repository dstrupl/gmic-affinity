# Implementation Notes

Engineering reference for `gmic-affinity`. Captures *how* the plugin is
built, *why* the unusual bits are the way they are, and what we learned
making Affinity Photo 2 actually load it.

Product spec (requirements, goals, status table) lives in [PRD.md](./PRD.md).
Day-to-day install / troubleshooting lives in [README.md](./README.md).

---

## 1. Plugin format on macOS

A Photoshop-compatible filter plugin is a macOS bundle directory with a
`.plugin` extension. The shipped layout:

```
GmicFilter.plugin/
├── Contents/
│   ├── Info.plist
│   ├── PkgInfo                          # 8 bytes: "8BFM8BIM"
│   ├── MacOS/
│   │   └── GmicFilter                   # universal Mach-O, filetype MH_BUNDLE
│   └── Resources/
│       ├── GmicFilter.rsrc              # legacy binary PiPL (mandatory)
│       ├── PiPLs.json                   # modern SDK-2026 metadata (forward compat)
│       └── en.lproj/
│           └── InfoPlist.strings
```

`Info.plist` declares `CFBundlePackageType=8BFM` (filter plugin) plus the
Carbon-era hint `CSResourcesFileMapped=yes` that every working
Affinity-compatible plugin we surveyed (e.g. AKVIS Coloriage) also sets.

### Why the executable must be `MH_BUNDLE`, not `MH_DYLIB`

Photoshop hosts (including Affinity) load `.plugin` bundles via
`CFBundleLoadExecutable`, which only accepts Mach-O files with filetype
**`MH_BUNDLE` (8)**. Rust's default `crate-type = "cdylib"` invokes the
linker with `-dynamiclib` and produces **`MH_DYLIB` (6)**, which the host
silently rejects — the plugin does not even appear as "unknown" in the
Detected Plugins list.

The fix is in the Makefile:

1. `crate-type = ["staticlib", "rlib"]` in `Cargo.toml` so cargo emits
   `libGmicFilter.a` plus an `rlib` for unit tests.
2. Relink with
   `clang -bundle -Wl,-force_load,libGmicFilter.a -Wl,-exported_symbol,_PluginMain -Wl,-dead_strip`
   per architecture.
3. `lipo -create` the per-arch results.
4. `make verify-bundle` runs `otool -h` on every slice and asserts
   filetype 8 before installation. This has already caught one regression.

---

## 2. The PiPL metadata story (the part that hurt the most)

Adobe SDK 2026 ships a modern JSON pipl format
(`Contents/Resources/PiPLs.json`). We initially shipped only that and
spent considerable time chasing red herrings — sandbox entitlements,
`Info.plist` locale strings, `en.lproj` localization — when Affinity
refused to detect the bundle.

The actual answer came from downloading the **AKVIS Coloriage demo** (a
confirmed-working Affinity Photo 2 plugin), mounting its DMG and diffing
its bundle against ours. AKVIS ships a 959-byte
`Contents/Resources/ColoriagePlugin.rsrc` file (named to match
`CFBundleExecutable`) containing a binary `PiPL` resource with the
classic `8BIMkind` / `8BIMname` / `8BIMcatg` / `8BIMma64` / `8BIMmi64` /
`8BIMmode` / `8BIMenbl` / `8BIMpmsa` properties.

Without a `.rsrc`, Affinity silently rejects the bundle during
enumeration. With one, it shows up immediately.

We compile ours from [`GmicFilter.r`](./GmicFilter.r) with `/usr/bin/Rez`,
the Carbon-era resource compiler that is, mercifully, still part of Xcode
Command Line Tools:

```bash
Rez -i $(PHOTOSHOP_SDK)/pluginsdk/photoshopapi/resources \
    -i $(PHOTOSHOP_SDK)/pluginsdk/photoshopapi/photoshop \
    -d "__PIMac__=1" -d "PRAGMA_ONCE=0" -useDF \
    -o GmicFilter.rsrc GmicFilter.r
```

Two non-obvious Rez flags matter:

- `-d "PRAGMA_ONCE=0"` — Adobe's `PIResDefines.h` uses a `PRAGMA_ONCE`
  macro that is never defined.
- We deliberately do **not** `#include "PIUtilities.r"`, because it
  pulls in Carbon `Types.r` / `SysTypes.r` headers for the classic
  `'STR '` and `'vers'` resource types that Apple removed from the
  CommandLineTools SDKs years ago. We do not need either resource for
  plugin discovery, so excluding the file lets Rez succeed on a stock
  Xcode-CLT install.

The compiled 610-byte binary `GmicFilter.rsrc` is **committed to the
repository** so CI environments without the Adobe SDK can still produce
a complete bundle. The Makefile re-runs Rez automatically when
`PHOTOSHOP_SDK` is set in the environment and the `.r` source is newer
than the committed binary.

`PiPLs.json` is also still committed — for forward compatibility with
SDK-2026-only hosts that *don't* read the legacy `.rsrc`. Both can live
in the same bundle.

---

## 3. `PluginMain` and the `FilterRecord` protocol

### Entry point

The host calls a single exported C function whose signature is:

```
void PluginMain(int16_t selector,
                FilterRecord *fr,
                intptr_t *data,
                int16_t *result);
```

`selector` is an integer phase code: `ABOUT=0`, `PARAMETERS=1`,
`PREPARE=2`, `START=3`, `CONTINUE=4`, `FINISH=5`. `result` is set to
`0` (`noErr`) on success or a non-zero error code on failure. The
function lives in [`src/lib.rs`](./src/lib.rs).

### The `advanceState` pull model

Adobe's `FilterRecord` is **pull-based**, not push-based. At
`SELECTOR_START` the plugin only *declares* which rectangle and which
channels it wants:

- set `in_rect`, `in_lo_plane`, `in_hi_plane`
- set `out_rect`, `out_lo_plane`, `out_hi_plane`

…and then **must invoke `FilterRecord.advance_state()`** (a function
pointer field at struct offset 296) to make the host actually allocate
buffers and populate `in_data`, `in_row_bytes`, `out_data`,
`out_row_bytes`. Only then does the host call `SELECTOR_CONTINUE`.

We initially expected `CONTINUE` to be called with pixels already in
place. That worked under naive `dlopen`-based testing but Affinity hit
us with `in_row_bytes == 0` on the first real invocation. Wiring up
`advance_state` from `START` fixed it.

### Struct layout is pinned by tests

The `FilterRecord` mirror in [`src/ps_types.rs`](./src/ps_types.rs) only
spells out the fields we use and pads the rest with `[u8; N]` to reach
the right total size. Total `sizeof(FilterRecord)` on macOS arm64 is
**648 bytes**. The offsets that matter — `in_data`, `in_row_bytes`,
`out_data`, `out_row_bytes`, `advance_state`, and the size of the whole
struct — are pinned by [`tests/layout.rs`](./tests/layout.rs) using
`std::mem::offset_of!()` so a future SDK header change will fail loudly
at `cargo test` rather than silently corrupting memory inside Affinity.

If you regenerate the struct against a new SDK, run that test first; it
is the most important guard rail in the codebase.

---

## 4. G'MIC invocation

### Binary detection

The plugin probes the two Homebrew install locations and uses whichever
exists:

| Architecture            | Path                       |
|-------------------------|----------------------------|
| Apple Silicon (ARM64)   | `/opt/homebrew/bin/gmic`   |
| Intel (x86\_64)         | `/usr/local/bin/gmic`      |

If neither exists, the filter returns a non-zero result code and the
file logger explains why; Affinity surfaces a generic "filter failed"
dialog.

### Subprocess hardening

[`src/gmic.rs`](./src/gmic.rs) shells out to `gmic` with:

- **A cleared environment** (`Command::env_clear()` followed by a small
  allow-list — `PATH`, `HOME`, `TMPDIR`, `LANG`). This eliminates an
  entire class of injection / surprise-config attacks from a malicious
  user account.
- **A per-call 0700 tempdir** (`tempfile::tempdir`) holding the input
  and output TIFFs. Files inside are named with random suffixes; the
  dir is removed on drop regardless of success / failure / panic.
- **A tight `argv` allow-list** — only `<input>`, the configured
  filter command split on whitespace, and `-output <path>` go on the
  command line. The filter command itself is read at every invocation
  from `~/.config/gmic-affinity/filter.txt` (fallback: a compile-time
  default).
- **Output capture caps** at 8 KB of stdout / 8 KB of stderr written to
  the file log; this is enough to debug typos in the filter command
  without OOM'ing on a chatty filter.

### Why the TIFF reader accepts five pixel formats

Almost every non-trivial gmic command (blur, convolution, FFT,
colour-space conversions, …) promotes the working image to gmic's
internal `float` representation and writes a float TIFF back, **even
when the input was 8-bit**. The set of `-output filename,<type>` strings
that force a specific output type is undocumented and varies between
gmic versions — gmic 3.7.6 accepts `,uchar` at parse time but rejects it
at write time with:

```
*** Error *** Command 'output': File '…', invalid specified pixel type 'uchar'
```

Rather than chase per-version syntax, [`src/tiff_io.rs`](./src/tiff_io.rs)
accepts `U8`, `U16`, `U32`, `F32` and `F64` decoded results and quantises
each to `u8` (clamp + round for floats, high-byte for unsigned ints).
This is bulletproof across gmic versions and the entire filter catalogue,
at the cost of being lossy if the user ever sends a 16-bit document
through — which is fine because the PiPL `EnableInfo` greys the filter
out for non-8-bit documents anyway.

The `tiff` crate's default `Limits` are also lifted to
`Limits::unlimited()`; the defaults are too strict for full-resolution
camera images (a 6000×4000 RGBA buffer is already 96 MB).

---

## 5. Build pipeline

### Toolchain

- Rust stable, with both `aarch64-apple-darwin` and
  `x86_64-apple-darwin` targets installed via `rustup`.
- Xcode Command Line Tools (provides `clang`, `lipo`, `codesign`, `Rez`).
- Optional: Adobe Photoshop SDK (only required to *regenerate*
  `GmicFilter.rsrc` from `GmicFilter.r`; a pre-built copy is committed).
- Homebrew `gmic-qt` at runtime.

### Make targets

| Target               | What it does                                                              |
|----------------------|---------------------------------------------------------------------------|
| `make bundle`        | Single-arch (host arm64) `.plugin` for fast dev iteration.                |
| `make universal`     | Universal `.plugin` (arm64 + x86\_64), Rez'd PiPL, lipo'd, ad-hoc signed. |
| `make install`       | `make bundle` + copy into Affinity Photo 2 plugins dir.                   |
| `make universal-install` | `make universal` + copy. **This is the one you usually want.**        |
| `make verify-bundle` | `otool -h` every slice, assert filetype `MH_BUNDLE` (8).                  |
| `make pipl`          | Re-run Rez if `PHOTOSHOP_SDK` is set; otherwise reuse committed `.rsrc`.  |
| `make clean`         | `cargo clean` and remove the bundle.                                      |

The default `cargo build` produces a **no-op `PluginMain`** that never
dereferences `FilterRecord`. The real filter logic is gated behind
`--features live`; the `Makefile` passes `FEATURES=live` on its build
recipes by default. The cargo-feature gate means a freshly cloned repo
can build a safely-installable (does-nothing) plugin without first
reconciling struct offsets — useful for someone bringing the project up
on a new SDK version.

### Code signing

Local development uses ad-hoc signing (`codesign --force --deep --sign -`).
Affinity Photo 2 accepts ad-hoc-signed bundles for local use; no Apple
Developer ID is required. Distribution will need a real Developer ID
cert and notarisation via `notarytool`.

### Cargo crate type recap

```toml
[lib]
name       = "GmicFilter"
crate-type = ["staticlib", "rlib"]
```

`staticlib` → re-linked to a true `MH_BUNDLE` by `clang -bundle`.
`rlib` → so the same crate can still be linked into integration tests
under `tests/`.

---

## 6. Logging: why there is a separate file log

Affinity routes plugin stderr to its own internal sink, so `eprintln!`
and `dbg!` output never reaches `Console.app` or `log show`. During
bring-up there was no way to tell whether `PluginMain` was even being
called.

The whole bring-up was unblocked by [`src/logging.rs`](./src/logging.rs),
which appends a single structured line per interesting event to:

```
~/Library/Logs/gmic-affinity.log
```

Format: `<ISO-8601 UTC> pid=<pid> <message>`. Future hosts (a Pixelmator
port, say) will almost certainly have the same stderr-swallowing
problem; keep that file logger around.

---

## 7. Post-mortem: the five surprises that ate most of the bring-up

In order of how much time they cost:

1. **`cdylib` produces `MH_DYLIB`, Photoshop wants `MH_BUNDLE`.** Hosts
   silently reject `MH_DYLIB`; the plugin doesn't even surface as
   "unknown". Fixed by `staticlib` + `clang -bundle`. See §1.
2. **Affinity ignores `PiPLs.json` and requires a legacy `.rsrc`.** No
   amount of `Info.plist` tweaking fixed detection until we shipped a
   Rez-compiled `<CFBundleExecutable>.rsrc`. AKVIS Coloriage was the
   smoking-gun reference. See §2.
3. **`SELECTOR_START` is not where pixels arrive.** Must invoke
   `FilterRecord.advance_state()` from `START` for the host to populate
   `in_data` / `out_data` before `CONTINUE`. See §3.
4. **gmic always promotes to float.** Output TIFFs are usually `F32`
   regardless of input depth; per-version `,uchar` syntax is too
   flaky to rely on. Reader now accepts U8/U16/U32/F32/F64 and
   quantises to U8. See §4.
5. **Affinity sinks plugin stderr.** Added the file logger; never
   looked back. See §6.

And one minor one we noticed but didn't pay for in time:

6. **`Info.plist` `CSResourcesFileMapped=yes`** — every working
   Affinity-compatible plugin we surveyed sets it. We never proved it
   is strictly required (vs just sufficient), but it costs nothing and
   matches known-working bundles.

---

## 8. Recommended project structure

```
gmic-affinity/
├── Cargo.toml
├── Makefile
├── Info.plist
├── PkgInfo                          # 8 bytes: 8BFM8BIM
├── GmicFilter.r                     # Rez source for PiPL
├── GmicFilter.rsrc                  # compiled PiPL (committed for SDK-less CI)
├── PiPLs.json                       # modern SDK-2026 PiPL (forward compat)
├── en.lproj/
│   └── InfoPlist.strings
├── src/
│   ├── lib.rs                       # PluginMain + selector dispatch
│   ├── ps_types.rs                  # FilterRecord + VRect + AdvanceStateProc
│   ├── filter.rs                    # bridge from FilterRecord to gmic invocation
│   ├── gmic.rs                      # hardened subprocess wrapper
│   ├── tiff_io.rs                   # multi-bit-depth TIFF read, U8 write
│   └── logging.rs                   # file logger to ~/Library/Logs/
├── tests/
│   └── layout.rs                    # FilterRecord offset / size assertions
├── PRD.md
├── IMPLEMENTATION_NOTES.md          # ← this file
├── README.md
└── LICENSE
```

---

*End of Implementation Notes*
