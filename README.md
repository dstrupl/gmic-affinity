# gmic-affinity

A Rust-based Photoshop-compatible filter plugin (`.plugin`) for macOS that bridges
[G'MIC](https://gmic.eu/) into [Affinity Photo 2](https://affinity.serif.com/photo/)
and later versions (including Affinity by Canva v3).

Status: **v0.1 release in progress.** The plugin loads in Affinity Photo 2,
appears as `Filters → Plugins → G'MIC → G'MIC...`, hands pixels through gmic
end-to-end and writes the result back inline. v0.1 ships universal,
ad-hoc-signed, for both Affinity Photo 2 and Affinity Photo v3.

Documentation set:

- [PRD.md](./PRD.md) — product spec: requirements, goals, status.
- [IMPLEMENTATION_NOTES.md](./IMPLEMENTATION_NOTES.md) — engineering
  detail: plugin format, `FilterRecord` layout, build pipeline, and the
  post-mortem of the five surprises that ate most of the bring-up.
- [docs/design/2026-05-18-release-v0.1-distribution.md](./docs/design/2026-05-18-release-v0.1-distribution.md)
  — v0.1 release / distribution design.
- This file — install + day-to-day troubleshooting.

## Install

### Manual install (zip)

This is the only install path for v0.1 — see "Why no Homebrew yet?"
below.

1. Make sure the `gmic` CLI is installed: `brew install gmic`. The
   plugin shells out to it; the Cocoa picker dialog is built in.
   (G'MIC-Qt — the standalone GUI / GIMP plugin — is not on Homebrew
   and is not needed for this plugin. If you want it anyway, grab it
   from <https://gmic.eu/download.html>.)
2. Download the latest `GmicFilter-vX.Y.Z.zip` from the
   [Releases page](https://github.com/dstrupl/gmic-affinity/releases/latest).
3. Unzip. Double-click `install.command` in the unzipped folder.
   - First time only: macOS may say "install.command cannot be opened
     because it is from an unidentified developer." Right-click →
     **Open** → **Open**, or in Terminal:
     `xattr -dr com.apple.quarantine .` then `./install.command`.
   - The script copies `GmicFilter.plugin` into every Affinity plugin
     folder that exists on your machine (Affinity Photo 2 and/or
     Affinity Photo v3) and strips the macOS quarantine bit so
     Affinity will load it.
4. Restart Affinity Photo and look for **Filters → Plugins → G'MIC →
   G'MIC…**.

If the plugin is not detected, open
**Affinity → Settings → Photoshop Plugins** and tick
*"Allow unknown plugins to be used"*.

### Why no Homebrew yet?

The original v0.1 plan included a private Homebrew tap
(`brew install --cask gmic-affinity`) as a one-liner alternative.
Homebrew is in the middle of a security tightening that retired the
cask-side workaround unsigned plugins like this one needed, and is
ending support for any cask that fails Apple Gatekeeper checks on
**2026-09-01** (see [Homebrew/brew#20755](https://github.com/homebrew/brew/issues/20755)).

The project now has the required signed-release path: a signing
collaborator with an Apple Developer Program membership runs the
Developer ID signing, notarisation, GitHub Release publishing, and
Homebrew tap bump from their Mac. The setup and per-release runbook
live in [`release/notarisation/SIGNING.md`](./release/notarisation/SIGNING.md).

Until the first signed stable release is cut and the tap is published,
the manual zip + `install.command` path lands the same files in the
same places that brew would.

## What it does

When installed into Affinity Photo's plugins folder, this adds a `G'MIC...` entry
to the **Filters → Plugins → G'MIC** submenu. Invoking it:

1. Opens a native macOS picker dialog (Cocoa) with the full G'MIC filter
   catalogue (~1500 filters, drawn from the bundled annotated
   `update*.gmic` snapshot). Search by name; pick a leaf; the right pane
   builds a parameter form for that filter (sliders, popups, color
   wells, …) pre-filled with either remembered or stdlib defaults.
2. On OK, receives the current 8-bit RGB / RGBA / Greyscale pixel data
   from Affinity via the Photoshop SDK's `FilterRecord` + `advanceState`
   protocol.
3. Writes the input region to a temporary TIFF in a per-call 0700 tempdir.
4. Runs the Homebrew-installed `gmic` binary with the chosen filter
   command and argument vector, in a cleared environment with a tight
   argv allow-list.
5. Reads the (possibly float, u16, u32) result TIFF back, quantises to
   8-bit, resamples to the host's filter rect if the filter changed
   dimensions (T12-C, e.g. rotate / crop), and copies it into
   Affinity's output buffer.

`Filter → Last Filter` (`Cmd-F`) replays the last picked filter without
re-opening the dialog.

## Configuration

The picker (v2) supersedes the older `~/.config/gmic-affinity/filter.txt`
mechanism. User state — the last filter, the recents MRU, and
per-filter remembered argument values — lives in:

```
~/Library/Application Support/gmic-affinity/settings.json
```

It is written atomically (temp + rename) after every OK from the
picker. If the file is unreadable on next open it is renamed to
`settings.json.broken-<ts>` and a fresh empty one is written; the
user-visible effect is loss of recents / remembered values, never a
crash.

`Filter → Last Filter` (Cmd-F) reads `settings.last` directly: even
across an Affinity restart, Cmd-F replays the last filter you picked
without opening the dialog.

## Troubleshooting

- Plugin doesn't appear in the Filters menu / "Detected Plugins" is
  empty: check the three things Affinity Photo silently rejects on
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
  Then open `Affinity Photo 2 -> Settings -> Photoshop Plugins`, ensure
  "Allow unknown plugins to be used" is ticked, and restart Affinity.
- Filter does nothing visible: open `Console.app`, filter on
  `gmic-affinity`. Each `PluginMain` call logs its selector.
- `gmic exited with status N`: try the same filter directly from a shell
  on a small TIFF; verify it works.

## License

MIT. See [LICENSE](./LICENSE).

---

# For developers

The rest of this file is for contributors building the plugin from
source. End users should follow the *Install* section above.

## Requirements

- macOS (Apple Silicon or Intel)
- [Rust](https://rustup.rs/) (stable, with `aarch64-apple-darwin` and
  `x86_64-apple-darwin` targets for a universal build)
- `gmic` CLI via Homebrew: `brew install gmic` (G'MIC-Qt is not on
  Homebrew and is not needed by this plugin)
- Affinity Photo 2 or Affinity Photo v3
- Git LFS — one-time per machine: `git lfs install`. After cloning,
  `git lfs pull` once. The bundled catalogue snapshot
  (`assets/gmic-catalogue.gmic.gz`) is LFS-tracked; without hydration
  the build fails fast with a `check-lfs` error from the Makefile.

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
make                    # ARM-only no-op bundle (fast iteration)
make universal          # ARM + x86_64 lipo'd bundle
make install            # copy GmicFilter.plugin into every detected Affinity Plugins folder
make uninstall          # remove the installed plugin
make picker-example     # open the picker standalone (no Affinity install needed)
make refresh-catalogue  # regenerate assets/gmic-catalogue.* from local gmic
make test               # cargo test under both default and --features live
make clippy             # cargo clippy --all-targets --all-features -D warnings
make quality-metrics    # report Rust function length + complexity guardrails
make help               # full list of targets
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

## Releasing

Pre-release tags (`vX.Y.Z-rc.N`, `vX.Y.Z-beta.N`) are CI-driven: push
the tag and the `release` GitHub Actions workflow builds an
ad-hoc-signed universal zip for testing.

Stable tags (`vX.Y.Z`) are collaborator-driven: ask the signing
collaborator to run `make release RELEASE_VERSION=vX.Y.Z` from a clean
checkout. That local pipeline signs with Developer ID, notarises,
staples, verifies Gatekeeper acceptance, publishes the GitHub Release,
and bumps the Homebrew tap cask. Full runbook:
[IMPLEMENTATION_NOTES.md](./IMPLEMENTATION_NOTES.md) §11 and
[`release/notarisation/SIGNING.md`](./release/notarisation/SIGNING.md).
