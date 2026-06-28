# Static pre-computed filter previews — design

Date: 2026-06-28
Status: Approved (brainstorming), pending implementation plan

## Goal

Show the user a representative preview image for each supported G'MIC
filter while they browse the picker. Previews are **pre-computed at
build time** against a single bundled sample image — there is no
runtime image processing in this feature. The build only recomputes a
filter's preview when something that affects its output has changed
(per-filter up-to-date check).

## Scope

- **Filters covered:** all renderable catalogue filters — every filter
  whose default-argument command produces a valid `gmic` run. Filters
  that fail (error, timeout, need external data, too large) are skipped
  and recorded, not retried at runtime.
- **Source image:** one bundled medium preview, ~512px max edge.
- **No runtime computation.** The picker only loads a pre-rendered PNG.

Non-goals (explicitly out of scope for this iteration):

- Live/parameter-aware previews that re-render as the user edits params.
- Per-user or per-image previews.
- Animated or multi-frame previews.

## Architecture

Three loosely-coupled units:

1. **Generator** — `src/bin/gen-previews.rs`. Walks the catalogue,
   renders one PNG per renderable filter via the `gmic` CLI, writes the
   manifest. Depends on `catalogue`, the gmic-run path, the source
   image, and the `image` crate (PNG encode).
2. **Manifest / up-to-date check** — a small pure module shared by the
   generator. Hashes inputs, decides stale vs. fresh, records successes
   and skips. Independently unit-testable without invoking gmic.
3. **UI preview loader** — in `picker_form.rs` (live feature only).
   Given the selected filter's command, resolves the bundled PNG and
   displays it; shows a placeholder when absent.

Data flow:

```
catalogue::builtin()  ─┐
assets/preview-source.tiff ─┤
gmic --version ────────┤
                       ▼
              gen-previews (build time)
                       │  per filter: default args → gmic run → PNG
                       ▼
   previews/manifest.json + previews/<key>.png   (committed to repo)
                       │  Makefile copies *.png
                       ▼
   GmicFilter.plugin/Contents/Resources/previews/<key>.png
                       │  selection change
                       ▼
   picker_form preview pane  (NSImageView + description, else placeholder)
```

## Generator (`src/bin/gen-previews.rs`)

For each `Filter` in `catalogue::builtin()`:

1. Build default args from each `Param` using the catalogue's
   default-for-kind logic. This is exactly the argv the picker sends on
   OK with untouched defaults. `reconcile::default_for` (currently
   private) is refactored to be reachable so the generator and the
   reconcile path share one source of truth.
2. Run `gmic` against the sample image: load source → apply
   `command default_args` → encode PNG to a temp file. The
   locate-binary + sanitise-argv + timeout logic in `src/gmic.rs` is
   extracted into a reusable run path so the generator does not
   duplicate it.
3. **Success:** write `previews/<key>.png`, record hash + `status: ok`
   + `file` in the manifest.
4. **Failure** (non-zero exit, timeout, too-large, missing external
   data): emit no PNG, record hash + `status: skip` + `reason`. The
   build continues.

**Filename key.** The gmic `command` is the logical key but is not
guaranteed filesystem-safe. A shared `sanitise_key(command)` function
maps it to `[A-Za-z0-9_-]`, disambiguating collisions with a short hash
suffix. This same function is used by the UI loader so the filename is
re-derivable in code without parsing the manifest at runtime. The
manifest still records `command → file` for tooling/inspection.

**Determinism.** Filters with random elements differ run-to-run. We
accept this: the up-to-date hash is over *inputs*, so output is only
regenerated when inputs change, keeping git diffs stable.

**CLI flags.** `--source <img>`, `--out <dir>`, `--manifest <path>`,
`--jobs N` (parallel gmic processes, default = CPU count), `--only
<command>` (regenerate one), `--force` (ignore manifest).

**Concurrency.** Render up to `--jobs` filters in parallel; each gmic
run is an isolated subprocess. This is important across ~1200 filters.

## Manifest & up-to-date check

`previews/manifest.json`, committed to git:

```json
{
  "source_hash": "sha256:…",
  "gmic_version": "3.3.6",
  "entries": {
    "fx_oldphoto": {
      "input_hash": "sha256:…",
      "status": "ok",
      "file": "fx_oldphoto.png"
    },
    "some_external_filter": {
      "input_hash": "sha256:…",
      "status": "skip",
      "reason": "timeout after 60s"
    }
  }
}
```

**`input_hash` = sha256 of:** source image bytes + gmic version string
+ filter `command` + default args vector (canonically joined). These
are exactly the inputs that change the rendered output. Catalogue
display-name / folder moves do *not* trigger a recompute because the
output is identical.

**Decision per filter** (`decide(entry, computed_hash, file_exists) ->
Action`, pure & unit-tested):

- Entry missing, or `input_hash` differs → **recompute**.
- Entry `status: ok` but its PNG missing on disk → **recompute**.
- Otherwise → **skip, keep existing**.

A changed `source_hash` or `gmic_version` flips every filter's
`input_hash`, so swapping the sample image or upgrading gmic
regenerates everything — with no separate global-stamp branch in the
code. The generator prints `N recomputed, M unchanged, K skipped`.

## Source image, storage & bundle layout

- **Source image:** new CC0 / public-domain photo at
  `assets/preview-source.tiff`, downscaled to 512px max edge, with
  varied content (color, texture, edges, smooth gradients) so diverse
  filters all render something meaningful. Tracked via Git LFS like the
  catalogue. License + source URL recorded in
  `assets/preview-source.LICENSE.txt`.
- **Repo storage:** `previews/` at repo root holding `manifest.json` +
  `<key>.png`. Committed as normal git objects (not LFS) — small PNGs
  at this size diff well as additions/removals. Can move to LFS later
  without code changes if size becomes a problem.
- **Bundle layout:** the Makefile copies `previews/*.png` into
  `GmicFilter.plugin/Contents/Resources/previews/`. A `make previews`
  target runs the generator and is wired as a prerequisite ahead of
  `bundle` / `universal`. Because of the manifest check, a no-change
  build does near-zero work.

## UI integration (three-column layout — Approach A)

The picker's 2-pane `NSSplitView` (tree | form) becomes a 3-pane split:
**tree | form | preview**.

- Preview pane (rightmost) holds an `NSImageView` showing the
  pre-rendered PNG scaled proportionally to fit the pane width, with the
  filter's `description` text below it.
- New min widths so dragging can't crush any pane (tree ~200, form
  ~220, preview ~240) and the default window width is increased so all
  three fit comfortably; split divider positions are autosaved as today.
- On selection change, the loader resolves the PNG via the running
  plugin bundle's Resources path (`NSBundle` →
  `Resources/previews/<sanitise_key(command)>.png`). The filename is
  re-derived in code via the shared `sanitise_key`; no runtime JSON
  parse.
- When the file is absent (skipped filter or unknown command), show a
  neutral "No preview available" placeholder.
- Preview UI lives under the existing `--features live` gate, like the
  rest of `picker_form.rs`.

## Error handling

- Generator: per-filter failures are isolated, recorded in the
  manifest with a reason, and never abort the build.
- UI: a missing or unreadable PNG falls back to the placeholder; the
  picker never blocks on preview loading.

## Testing

- **Manifest module:** pure unit tests for `input_hash` composition and
  the `decide(...)` state machine (missing entry, hash drift, ok-but-
  file-gone, fresh).
- **`sanitise_key`:** unit tests for safe charset, collision
  disambiguation, and round-trip stability (same command → same key).
- **Generator:** a small smoke test over a tiny fixture catalogue (1–2
  filters) confirming PNG emission + manifest write + skip recording,
  guarded so it only runs where `gmic` is available.
- **UI:** preview-pane construction is exercised by the existing
  live-feature build; placeholder fallback verified for a known-absent
  key.

## Open questions / deferred

- Whether to eventually track `previews/` in Git LFS if the directory
  grows large — deferred; no code impact.
- Live parameter-aware previews — explicitly out of scope.
