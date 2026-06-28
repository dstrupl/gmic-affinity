# Manual test plan — static filter previews

Two phases, as requested:
1. **Standalone** — the effect selector (picker) running on its own, no
   Affinity, via the `picker` example.
2. **In Affinity** — after installing the built `.plugin` into Affinity
   Photo.

This plan is written to be executed once the preview-pane UI (Group C,
Tasks 9–10) is implemented. Until then, only the "preview assets" checks
(Phase 0) can run.

## Preconditions

- `gmic` installed (`/opt/homebrew/bin/gmic`), `git lfs pull` done so
  `assets/preview-source.tiff` is a real TIFF (not a pointer).
- Previews generated and committed: `previews/manifest.json` +
  `previews/*.png` present (~1029 PNGs).
- Build understands where the UI loads previews from. In a non-installed
  run (the `picker` example) the loader resolves previews via the
  `GMIC_PREVIEWS_DIR` env override (per the plan); in an installed bundle
  it resolves `…/GmicFilter.plugin/Contents/Resources/previews/`.

## Phase 0 — preview assets sanity (can run now, pre-UI)

Goal: confirm the generated set itself is good before trusting the UI.

1. **Count + integrity**
   - `ls previews/*.png | wc -l` matches the manifest "ok" count.
   - No zero-byte PNGs: `find previews -name '*.png' -size 0` is empty.
   - No stray frame files: `ls previews/*_0000*.png` is empty.
2. **Spot-check representative filters visually** (open the PNGs):
   - A tonal/photographic filter (e.g. `fx_old_photo`) — recognisably the
     sample photo, processed.
   - A color-heavy filter (e.g. one with color params) — renders, colors
     look intentional (the color-encoding fix).
   - A multi-output filter (e.g. `cl_colorWheel`, `afre_details`) — shows
     frame 0 (the result), not a blank/duplicate.
3. **Manifest skips are plausible** — skim `manifest.json` skip reasons;
   confirm they're "needs external data / timeout / genuinely failing,"
   not a new systematic error.

Pass criteria: counts line up, no corrupt/stray files, spot-checked
images look like real processed previews.

## Phase 1 — standalone selector (picker example, no Affinity)

Goal: the preview pane shows the correct image for the selected filter,
updates on selection change, and degrades gracefully.

Launch:
```
GMIC_PREVIEWS_DIR="$PWD/previews" make picker-example
```
(Builds + runs `examples/picker.rs` under `--features live`; the env var
points the loader at the repo previews since this isn't an installed
bundle.)

Test cases:
1. **Three-column layout** — picker opens with tree | params | preview
   columns. Window is wide enough that all three are usable; dividers
   draggable; no pane collapses to unusable width.
2. **Preview shows on selection** — select `fx_old_photo` (or similar):
   the preview pane shows its PNG, with the filter description beneath.
3. **Preview updates** — select a different filter; the image + caption
   change to match. Select several in a row; no stale image, no flicker
   that persists.
4. **Folder / no-leaf selection** — select a folder row (not a filter):
   preview resets to placeholder (no leftover image from prior filter).
5. **Placeholder for skipped filters** — select a filter known to be
   skipped (no PNG; pick one from manifest skips, e.g. a `_animate_` or an
   external-data filter): pane shows "No preview available," not a broken
   image or a crash.
6. **Unknown/edge** — a filter whose command sanitises oddly still either
   shows its preview or the placeholder (never an error dialog).
7. **Resize** — resize the window; preview scales proportionally, stays
   within its pane, caption still readable.
8. **Selection + OK still works** — the preview pane doesn't interfere
   with the existing pick flow: select a filter, adjust a param, OK
   returns the expected ChosenFilter (watch the example's stdout).
9. **No console errors** — no panics / objc exceptions in the terminal
   during the above.

Pass criteria: correct preview per selection, correct placeholder
fallback, graceful resize, existing pick flow unaffected, no crashes.

## Phase 2 — in Affinity Photo

Goal: the feature works inside the real host, loading previews from the
installed bundle Resources.

Setup:
1. Build + install the bundle: `make install` (or `make universal-install`
   for a universal build). Confirm the bundle contains previews:
   `ls "GmicFilter.plugin/Contents/Resources/previews" | wc -l` > 0.
2. Restart Affinity Photo; open an image; **Filters → Plugins → G'MIC →
   G'MIC…**.

Test cases:
1. **Previews load from the bundle** — open the picker in Affinity; select
   filters; previews appear (proving the `NSBundle`/dladdr resolution to
   `Contents/Resources/previews` works in the host, not just via the env
   override).
2. **Same UX as standalone** — selection updates the preview; placeholder
   for skipped filters; resize behaves.
3. **End-to-end apply** — pick a filter with a visible effect, click OK,
   confirm Affinity applies it to the actual image (the preview matched
   what the filter does).
4. **A multi-output filter end-to-end** — pick e.g. `cl_colorWheel`;
   confirm it applies frame 0 (no "couldn't read back" error — the plugin
   fix), and the preview shown matched.
5. **A color filter end-to-end** — pick a color-param filter; confirm it
   applies without the old positional-arg error and the result resembles
   its preview (the color-encoding fix).
6. **No host instability** — opening/closing the picker repeatedly,
   switching filters, applying several in a row: no Affinity crash, no
   hang.

Pass criteria: previews load from the installed bundle, UX matches
standalone, applied results correspond to previews, multi-output and
color filters apply cleanly, host stays stable.

## Recording results

For each phase, note: pass/fail per case, screenshots of any wrong
preview or placeholder, and the exact filter command for any failure so
it can be reproduced via the generator/CLI.
