# gmic-affinity

A Rust-based Photoshop-compatible filter plugin (`.plugin`) for macOS that bridges
[G'MIC](https://gmic.eu/) into [Affinity Photo 2](https://affinity.serif.com/photo/)
and later versions (including Affinity by Canva v3).

Status: **MVP shipped (2026-05-17).** The plugin loads in Affinity Photo 2,
appears as `Filters → Plugins → G'MIC → G'MIC...`, hands pixels through gmic
end-to-end and writes the result back inline.

Documentation set:

- [PRD.md](./PRD.md) — product spec: requirements, goals, status.
- [IMPLEMENTATION_NOTES.md](./IMPLEMENTATION_NOTES.md) — engineering
  detail: plugin format, `FilterRecord` layout, build pipeline, and the
  post-mortem of the five surprises that ate most of the bring-up.
- This file — install + day-to-day troubleshooting.

## What it does

When installed into Affinity Photo's plugins folder, this adds a `G'MIC...` entry
to the **Filters → Plugins → G'MIC** submenu. Invoking it:

1. Receives the current 8-bit RGB / RGBA / Greyscale pixel data from Affinity via the
   Photoshop SDK's `FilterRecord` + `advanceState` protocol.
2. Writes the input region to a temporary TIFF in a per-call 0700 tempdir.
3. Runs the Homebrew-installed `gmic` binary with a configurable filter command,
   in a cleared environment with a tight argv allow-list.
4. Reads the (possibly float, u16, u32) result TIFF back, quantises to 8-bit, and
   copies it into Affinity's output buffer — no manual file roundtrip required.

## Requirements

- macOS (Apple Silicon or Intel)
- [Rust](https://rustup.rs/) (stable, with `aarch64-apple-darwin` and
  `x86_64-apple-darwin` targets for a universal build)
- `gmic` via Homebrew: `brew install gmic-qt`
- Affinity Photo 2 or later

## Build

This repository builds two flavours of the plugin, controlled by the cargo
feature `live`:

| Build                                  | What you get                                          |
|----------------------------------------|-------------------------------------------------------|
| `make bundle`                          | No-op `PluginMain` (M1). Safe to install at any time. |
| `make bundle FEATURES=live`            | Real pixel pipeline (M3 pass-through + M4 G'MIC).     |

Why the split? The plugin entry point dereferences a host-supplied
`FilterRecord` struct. The default no-op build never touches it, so it is
useful for verifying that the bundle merely loads. The `live` build runs
the real M3 + M4 pipeline. `FilterRecord` offsets are pinned by
`tests/layout.rs` against the Adobe Photoshop SDK 2026 v2's `PIFilter.h`,
so the `live` build is safe to install whenever `cargo test` is green.

Common targets:

```bash
make             # ARM-only no-op bundle (fast iteration)
make universal   # ARM + x86_64 lipo'd bundle
make install     # copy GmicFilter.plugin into Affinity Photo 2 plugins folder
make uninstall   # remove the installed plugin
make test        # cargo test under both default and --features live
make clippy      # cargo clippy --all-targets --all-features -D warnings
make help        # full list of targets
```

### Plugin metadata: legacy `.rsrc` *and* modern `PiPLs.json`

Photoshop-compatible filter hosts learn about a plugin's menu name,
category, supported image modes and entry-point function name from a
**pipl resource**. The bundle here ships *two* copies of that
metadata:

| File                               | Used by                                | Source                |
|------------------------------------|----------------------------------------|-----------------------|
| `Contents/Resources/GmicFilter.rsrc` | **Affinity Photo 2**, Adobe Photoshop  | [`GmicFilter.r`](./GmicFilter.r) compiled by `Rez` |
| `Contents/Resources/PiPLs.json`      | Adobe Photoshop SDK 2026+ greenfield  | [`PiPLs.json`](./PiPLs.json) copied verbatim       |

The reason we keep both: Affinity Photo 2 (verified empirically against
every plugin its own host loads, including AKVIS) **only reads the legacy
`<CFBundleExecutable>.rsrc` file**. With no `.rsrc` Affinity silently
rejects the bundle during enumeration — it never opens the Mach-O and
the plugin never appears, not even as "unknown". The modern JSON pipl is
just kept so the same bundle could later be loaded by a future
SDK-2026+-only host without rebuilding.

The compiled `GmicFilter.rsrc` is committed to the repo (it is 610 bytes
and deterministic) so contributors and CI without the Adobe Photoshop
SDK can still build the bundle. When the SDK *is* present (env var
`PHOTOSHOP_SDK`, defaulting to `$HOME/SDKs/photoshop-sdk`) the Makefile
re-runs `/usr/bin/Rez` so edits to `GmicFilter.r` take effect.

### Why a `staticlib`, not a `cdylib`

Affinity (and Adobe Photoshop, and any other host that loads `.plugin`
bundles via `CFBundleLoadExecutable` / `dlopen`) requires the bundle's
Mach-O executable to be **`MH_BUNDLE`** (filetype `8`). Rust's `cdylib`
crate-type produces **`MH_DYLIB`** (filetype `6`), which hosts silently
reject — the plugin doesn't even appear as "unknown" in the Detected
Plugins list. To get an `MH_BUNDLE` we build the crate as a `staticlib`
and relink with `clang -bundle`. The `make bundle` / `make universal`
targets do this for you and the `verify-bundle` Make rule asserts
`filetype 8` on every slice before installation. If you ever see
"Plugin not detected" again, run:

```bash
otool -h GmicFilter.plugin/Contents/MacOS/GmicFilter   # filetype must be 8
nm    -gU GmicFilter.plugin/Contents/MacOS/GmicFilter  # must export _PluginMain
```

See [PRD.md](./PRD.md) §7 for the full PRD-side build and install procedure.

## Configuration

The argv passed to `gmic` is read from `~/.config/gmic-affinity/filter.txt`
if present, otherwise `-fx_oldphoto` is used. The file must be at most 4 KiB
and may not contain NUL or other control characters (tab, newline, and CR
are allowed). Examples:

```
# ~/.config/gmic-affinity/filter.txt
-blur 8
```

```
-fx_rodilius 10,2,200,20,3,0
```

A proper parameter UI is planned for v2.

## Troubleshooting

- Plugin doesn't appear in the Filters menu / "Detected Plugins" is
  empty: check the three things Affinity Photo 2 silently rejects on
  (this list is the result of dissecting AKVIS Coloriage, which is
  known-working under Affinity, and the matching `fs_usage` traces of
  Affinity's plugin scanner — see commit history):
  ```bash
  INST="$HOME/Library/Application Support/Affinity Photo 2/Plugins/GmicFilter.plugin"
  # 1) Mach-O type must be 8 (MH_BUNDLE), not 6 (MH_DYLIB).
  otool -h "$INST/Contents/MacOS/GmicFilter"
  # 2) PluginMain (or whatever name your pipl's CodeMac{ARM,Intel}64
  #    declares) must be in the dynamic symbol table.
  nm -gU "$INST/Contents/MacOS/GmicFilter"
  # 3) A legacy pipl resource file named after CFBundleExecutable must
  #    sit next to the modern PiPLs.json:
  ls -la "$INST/Contents/Resources/GmicFilter.rsrc"
  strings "$INST/Contents/Resources/GmicFilter.rsrc" | head
  ```
  All three are produced by `make universal-install`. Then open
  `Affinity Photo 2 -> Settings -> Photoshop Plugins`, ensure "Allow
  unknown plugins to be used" is ticked, and restart Affinity.
- Filter does nothing visible: open `Console.app`, filter on
  `gmic-affinity`. Each `PluginMain` call logs its selector.
- `gmic exited with status N`: try the same filter directly from a shell
  on a small TIFF; verify it works.

## License

TBD.
