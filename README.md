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

```bash
make bundle      # produce GmicFilter.plugin (universal binary, ad-hoc signed)
make install     # copy bundle into Affinity Photo 2 plugins folder
```

See [PRD.md](./PRD.md) §7 for the full build and install procedure.

## Configuration

The filter command sent to `gmic` is read from
`~/.config/gmic-affinity/filter.txt` if present, otherwise a default is used.
A proper parameter UI is planned for v2.

## License

TBD.
