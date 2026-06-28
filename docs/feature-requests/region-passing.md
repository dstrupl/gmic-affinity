# Feature request: pass the user's selection / region to the filter

Status: **deferred** (captured 2026-06-28)

## Background

The picker hides filters that cannot run as a single-image,
non-interactive transform (see
`docs/superpowers/specs/2026-06-28-filter-exclusion-design.md`). Some of
those excluded filters are excluded only because they need a *user
input* the plugin does not currently provide — in particular a
selection / region / mask:

- `fx_extract_foreground`, `fx_extract_objects` — expect the user to mark
  foreground/background.
- `fx_mask_color`, selection-driven fills, etc.

The plugin currently hands gmic the whole filter rect. If we passed the
host's active **selection** (Affinity's marquee/selection) through to the
filter — as a region, a mask layer, or coordinates — some of these
filters would become usable instead of hidden.

## The ask

Investigate handing the user's selection to gmic so selection-dependent
filters work:

- **What the host exposes:** does the Photoshop-plugin `FilterRecord`
  give us the selection region / mask (`maskData`, selection bounds)? If
  so, in what form?
- **How gmic wants it:** as a second image (mask), as coordinate
  parameters, or as a cropped input region?
- **Which filters benefit:** cross-reference the exclusion list for
  selection-dependent filters and estimate how many become usable.
- **UX:** how the picker communicates that a filter uses the current
  selection (and what happens when there is none).

## Why deferred

Exclusion ships first and is self-contained. Region/selection passing
changes how pixels/inputs are handed to gmic and intersects the
`FilterRecord` mask plumbing — a distinct effort with its own design.
When it lands, the exclusion heuristics/baked list should be revisited so
newly-viable filters are no longer hidden.

## Pointers

- `src/gmic.rs` — `run_with_tokens` (how the input region is written to
  TIFF today).
- `src/ps_types.rs` — `FilterRecord` layout (where selection/mask fields
  would live).
- Exclusion design: `docs/superpowers/specs/2026-06-28-filter-exclusion-design.md`.
