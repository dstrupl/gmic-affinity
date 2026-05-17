# gmic-affinity

A Rust-based Photoshop-compatible filter plugin (`.plugin`) for macOS that bridges
[G'MIC](https://gmic.eu/) into [Affinity Photo 2](https://affinity.serif.com/photo/)
and later versions (including Affinity by Canva v3).

Status: **MVP in progress** — see [PRD.md](./PRD.md) for the full product spec.

## What it does

When installed into Affinity Photo's plugins folder, this adds a `G'MIC...` entry
to the **Filters → Plugins** menu. Invoking it:

1. Receives the current pixel data from Affinity.
2. Writes it to a temporary TIFF.
3. Runs the Homebrew-installed `gmic` binary with a configurable filter command.
4. Reads the result back and returns the modified pixels to Affinity — no manual
   file roundtrip required.

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

The pipl resource that tells Affinity our menu name, category, and
supported image modes lives in [PiPLs.json](./PiPLs.json) and is copied
verbatim into `Contents/Resources/PiPLs.json` by every bundle target. This
is the modern Adobe Photoshop SDK 2026+ format and supersedes the legacy
Rez/.rsrc workflow.

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
  empty: first run `otool -h
  "$HOME/Library/Application Support/Affinity Photo 2/Plugins/GmicFilter.plugin/Contents/MacOS/GmicFilter"`
  and confirm `filetype` is `8` (MH_BUNDLE). If it is `6` (MH_DYLIB) the
  host will silently reject it — rebuild with `make universal-install`.
  Then open `Affinity Photo 2 -> Settings -> Photoshop Plugins`, ensure
  "Allow unknown plugins to be used" is ticked, and restart Affinity.
- Filter does nothing visible: open `Console.app`, filter on
  `gmic-affinity`. Each `PluginMain` call logs its selector.
- `gmic exited with status N`: try the same filter directly from a shell
  on a small TIFF; verify it works.

## License

TBD.
