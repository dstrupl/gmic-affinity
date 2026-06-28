# Feature request: import multi-frame G'MIC output (animations)

Status: **deferred** (captured 2026-06-28)

## Background

Some G'MIC filters leave **more than one image** on gmic's image list:

- **Animations** — the `fx_animate_*` family (e.g. `fx_animate_glow`,
  `fx_animate_cartoon`) produce a *sequence* of frames; with default
  parameters a single call emits ~10 images.
- **"Result + extra" filters** — e.g. `afre_details`, `cl_colorWheel`,
  `Triangles_Shades_Adjacents` leave 2 images (typically the processed
  result plus a secondary visualization or the untouched original).

When gmic is told `-output out.tif` but the list holds ≠1 image, it writes
`out_000000.tif`, `out_000001.tif`, … and no `out.tif`.

## Current behavior (after the multi-output frame-0 fix)

The plugin and the preview generator both resolve the output to **frame 0**
(`out_000000.*`) when the single expected file is absent (see
`gmic::resolve_output_path`). Frame 0 was empirically verified to be the
meaningful result for the "result + extra" class, and is a sensible still
for animations. Affinity's `FilterRecord` API accepts exactly one output
image, so a single frame is all the host can represent today.

This means **animations currently collapse to their first frame**. That is
correct and useful as a still, but it discards the rest of the sequence.

## The ask

Provide a way to import the *whole* animation rather than just frame 0,
when the user picks an `fx_animate_*` (or otherwise multi-frame) filter.

Open questions / options to explore:

- **Layers**: import each frame as a separate Affinity layer. Requires a
  host capability beyond the single-region `FilterRecord` filter
  protocol — investigate whether Affinity's plugin API (or a different
  plugin entry point) can create multiple layers / a layer group.
- **Animated export**: write the frames to an animated format (APNG / GIF)
  and surface the path, rather than returning pixels inline.
- **Frame picker**: let the user choose which frame to apply (default 0),
  or a "frames N..M as layers" range.
- **Detection + UX**: detect multi-frame filters up front (the catalogue /
  filter metadata may indicate this) and present the relevant option in the
  picker instead of silently applying frame 0.

## Why deferred

The single-frame behavior is correct for the current one-image host
contract and unblocks the static-previews feature. True animation import is
a larger, separate piece of work touching the host integration model and
the picker UX, and should be scoped on its own.

## Pointers

- `src/gmic.rs` — `resolve_output_path`, `run_with_tokens`,
  `render_with_tokens` (the frame-0 fallback).
- Investigation notes: filters confirmed multi-output include
  `fx_animate_*`, `afre_details`, `cl_colorWheel`,
  `Triangles_Shades_Adjacents`.
