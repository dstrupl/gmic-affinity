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
`FilterRecord` struct, and the layout in `src/ps_types.rs` has not yet been
reconciled byte-for-byte against `PIFilter.h` from the Adobe Photoshop SDK
(see [SETUP.md](./SETUP.md)). The default no-op build never touches the
struct, so it is safe to install even before that reconciliation lands. Once
`cargo test -- --ignored` passes, switch to `FEATURES=live`.

Common targets:

```bash
make             # ARM-only no-op bundle (fast iteration)
make universal   # ARM + x86_64 lipo'd bundle
make pipl        # build GmicFilter.rsrc from GmicFilter.r (needs SDK)
make install     # copy GmicFilter.plugin into Affinity Photo 2 plugins folder
make uninstall   # remove the installed plugin
make test        # cargo test under both default and --features live
make clippy      # cargo clippy --all-targets --all-features -D warnings
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

- Plugin doesn't appear in the Filters menu: open
  `Affinity Photo 2 -> Settings -> Photoshop Plugins`, ensure
  "Allow unknown plugins to be used" is ticked, then restart Affinity.
- Filter does nothing visible: open `Console.app`, filter on
  `gmic-affinity`. Each `PluginMain` call logs its selector.
- `gmic exited with status N`: try the same filter directly from a shell
  on a small TIFF; verify it works.

## License

TBD.
