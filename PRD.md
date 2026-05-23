# PRD: gmic-affinity — A Rust Photoshop Filter Plugin Bridging G'MIC and Affinity Photo

**Version:** 0.1
**Status:** **MVP shipped — 2026-05-17.** End-to-end pipeline working in
Affinity Photo 2 (Serif store build, Apple Silicon, macOS 14+). Universal
binary, ad-hoc signed, no Developer ID required for local use.
**Target platform:** macOS (Apple Silicon + Intel), Affinity Photo 2 and
later (including Affinity by Canva v3).

> This document captures the product requirements and current status only.
> The engineering deep-dive (plugin format, `FilterRecord` layout, build
> pipeline, post-mortem) lives in [IMPLEMENTATION_NOTES.md](./IMPLEMENTATION_NOTES.md).
> Day-to-day install / troubleshooting steps live in [README.md](./README.md).

---

## 1. Background and Motivation

[G'MIC](https://gmic.eu/) (GREYC's Magic for Image Computing) is a
powerful open-source image-processing framework with hundreds of filters,
available on macOS via Homebrew (`brew install gmic`). Affinity Photo
is a professional image editor that supports Photoshop-compatible filter
plugins (`.plugin` bundles) on macOS.

As of 2026, no native macOS Photoshop-compatible plugin exists for G'MIC.
The [`gmic-8bf`](https://github.com/0xC0000054/gmic-8bf) project (Windows
only) provides this on Windows via a `.8bf` DLL, but its author confirmed
that a macOS port requires substantial OS-specific rewriting. This project
fills that gap.

The result is a self-contained Rust crate that compiles into a macOS
`.plugin` bundle and runs inside Affinity Photo with no manual file
roundtrip and no Adobe Photoshop install on the user's machine.

This works with **Affinity Photo 2** and all later versions including
**Affinity by Canva (v3)**, because the Photoshop plugin interface is
stable across those versions.

---

## 2. Goals

- **G1** — Appears as a filter entry in Affinity Photo's Filters menu
  without requiring a Canva account or Claude Desktop.
- **G2** — Works on both Apple Silicon (ARM64) and Intel (x86\_64) Macs
  as a universal binary.
- **G3** — Written entirely in Rust; no C++ compilation required.
- **G4** — Invokes the system `gmic` binary installed by Homebrew at
  `/opt/homebrew/bin/gmic` (Apple Silicon) or `/usr/local/bin/gmic`
  (Intel).
- **G5** — Operates inline: Affinity hands in pixels, plugin returns
  modified pixels, no separate open-file step.
- **G6** — Self-contained: no additional runtime dependencies beyond
  `gmic` itself.

## 3. Non-Goals

- ~~A graphical parameter UI (the initial version uses a hardcoded or
  config-file-based filter command).~~ **Shipped in v2 — see
  `docs/design/2026-05-17-gmic-picker-dialog.md`.**
- Windows support.
- Tiled / chunked processing (whole-image only in v1).
- Distribution via any plugin marketplace.
- Integration with G'MIC-Qt's interactive GUI (future work).

---

## 4. User-Facing Behaviour & Acceptance Criteria

The product is considered shipped when, on a clean macOS install with
Homebrew `gmic` present:

| # | Acceptance criterion                                                                                                  | Status |
|---|------------------------------------------------------------------------------------------------------------------------|--------|
| 1 | Bundle copied to `~/Library/Application Support/Affinity Photo 2/Plugins/` and/or `~/Library/Application Support/Affinity/Plugins/` is discovered after Affinity Photo 2 / Affinity Photo v3 restart. | ✅ (v2 verified; v3 pending Phase 0 of [release design](./docs/design/2026-05-18-release-v0.1-distribution.md)) |
| 2 | Bundle appears in **Preferences → Photoshop Plugins → Detected** (Unknown status is acceptable).                       | ✅     |
| 3 | A filter entry `G'MIC...` exists under **Filters → Plugins → G'MIC**.                                                  | ✅     |
| 4 | Invoking the filter on an 8-bit RGB document produces a visibly transformed image in-place (no separate file dialog).  | ✅     |
| 5 | The filter command is chosen interactively from a native picker dialog (catalogue + parameter form) without rebuilding the plugin. The user's last pick is remembered and replayed by `Filter → Last Filter` (`Cmd-F`). | ✅     |
| 6 | Plugin runs natively on both Apple Silicon and Intel without Rosetta translation.                                      | ✅     |
| 7 | Plugin loads under ad-hoc code signing (no paid Apple Developer ID required for personal use).                         | ✅     |
| 8 | Plugin failure (gmic missing, gmic returns non-zero, malformed TIFF) reports a non-zero result code and does not crash Affinity. | ✅     |

---

## 5. System Requirements

| Component              | Minimum                                            |
|------------------------|----------------------------------------------------|
| OS                     | macOS 14 Sonoma (tested), should work on 12+      |
| Host                   | Affinity Photo 2 (Serif store build) or Affinity Photo v3 (Affinity by Canva) on macOS |
| CPU                    | Apple Silicon (ARM64) or Intel (x86\_64)          |
| External dependency    | Homebrew `gmic` formula (provides the `gmic` CLI; G'MIC-Qt is *not* a Homebrew package and is not needed) |
| Build-time (developer) | Rust stable, Xcode Command Line Tools, optional Adobe Photoshop SDK (only needed to *regenerate* `GmicFilter.rsrc`; a pre-built copy ships in the repo) |

---

## 6. Shipped vs Deferred Status

| Capability                                       | Status                  |
|---------------------------------------------------|-------------------------|
| Affinity Photo 2 loads and lists the plugin       | ✅ shipped              |
| Affinity Photo v3 loads and lists the plugin      | ✅ shipped (Phase 0 step 2 PASS on 2026-05-19; see release design doc §3) |
| Distributed via GitHub release zip + `install.command` | ✅ shipped (universal `.plugin` zip, double-clickable installer; see [release design doc](./docs/design/2026-05-18-release-v0.1-distribution.md)) |
| Distributed via Homebrew cask                     | 🟡 v0.2 (cask + tap repo scaffolded under [`release/homebrew-tap/`](./release/homebrew-tap/) but unpublished. Signed-release path is documented in [`release/notarisation/SIGNING.md`](./release/notarisation/SIGNING.md); remaining work is first collaborator-run release + tap publication) |
| Filter runs end-to-end on a real image            | ✅ shipped              |
| Universal binary (arm64 + x86\_64)                | ✅ shipped              |
| 8-bit RGB / RGBA / Greyscale                      | ✅ shipped              |
| `FilterRecord` layout pinned vs SDK by tests      | ✅ shipped              |
| CI build (no Adobe SDK required)                  | ✅ shipped              |
| 16-bit / 32-bit per-channel modes                 | ⛔ v2                   |
| Parameter UI (catalogue picker + dynamic form)    | ✅ shipped (v2 — see `docs/design/2026-05-17-gmic-picker-dialog.md`) |
| Tiled / chunked processing                        | ⛔ v2                   |
| Notarised distribution / Apple Developer ID       | 🟡 release path ready   |
| G'MIC-Qt interactive GUI launch                   | ⛔ post-v1              |

---

## 7. Known Limitations

- **Whole-image only.** No tiled processing; very large images consume a
  proportional amount of RAM during the temp-file round trip.
- **8-bit only.** The PiPL `EnableInfo` greys out the filter for 16- or
  32-bit documents.
- ~~**Single hardcoded filter per invocation.** No interactive parameter
  dialog; the command is either a compile-time default or whatever the
  user puts in `~/.config/gmic-affinity/filter.txt`.~~ Resolved in v2:
  the picker dialog now exposes the full catalogue with per-filter
  parameter editing; user state is persisted to
  `~/Library/Application Support/gmic-affinity/settings.json`.
- **Ad-hoc manual zips until the first signed stable release.** Local
  development and pre-release zips use ad-hoc signing; stable
  distribution uses the collaborator-run Developer ID + notarisation
  pipeline in `release/notarisation/SIGNING.md`.
- **External binary dependency.** The plugin shells out to a Homebrew-
  installed `gmic`; if the binary is missing the filter fails cleanly
  rather than embedding gmic itself.

---

## 8. Future Work (post-v1)

| Item                              | Notes |
|-----------------------------------|-------|
| ~~Parameter UI~~                  | ~~Either a native sheet via `filterSelectorParameters`, or launching `gmic_qt` standalone for filter picking.~~ Shipped in v2 via native Cocoa picker + dynamic parameter form. |
| 16/32-bit support                 | Promote `tiff_io` to those depths, update PiPL `EnableInfo`. |
| Tiled processing                  | Required to handle gigapixel images without OOM. |
| Notarised cask distribution       | Run the collaborator-driven `make release RELEASE_VERSION=vX.Y.Z` pipeline from [`release/notarisation/SIGNING.md`](./release/notarisation/SIGNING.md), then publish the cask in [`release/homebrew-tap/`](./release/homebrew-tap/) to `dstrupl/homebrew-gmic-affinity`. The signing/notarisation process exists; the remaining v0.2 work is the first signed release and tap publication. |
| Affinity v3 scripting integration | The v3 MCP/Scripts panel could invoke this plugin programmatically. |
| Bundled `gmic`                    | Optional — could ship a vendored gmic binary so users do not need Homebrew. |

---

## 9. Related Resources

| Resource                                   | URL |
|--------------------------------------------|-----|
| Adobe Photoshop SDK                        | https://console.adobe.io → Downloads → Creative Cloud → Photoshop C++ SDK |
| G'MIC documentation                        | https://gmic.eu/reference/ |
| G'MIC Homebrew formula                     | `brew info gmic` |
| G'MIC-Qt standalone GUI / GIMP plugin      | <https://gmic.eu/download.html> (not on Homebrew, not required by this plugin) |
| gmic-8bf (Windows reference implementation) | https://github.com/0xC0000054/gmic-8bf |
| Affinity plugin preferences                | Affinity Photo → Edit → Preferences → Photoshop Plugins |

---

## 10. Project Documents

- [README.md](./README.md) — install + day-to-day troubleshooting.
- [IMPLEMENTATION_NOTES.md](./IMPLEMENTATION_NOTES.md) — engineering
  detail: plugin format, `FilterRecord` layout, build pipeline,
  post-mortem of the bring-up.
- [LICENSE](./LICENSE) — MIT.

---

*End of PRD*
