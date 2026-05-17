# Design: G'MIC Picker Dialog (catalogue tree + parameter form)

**Date:** 2026-05-17
**Status:** Approved, ready for implementation planning.
**Supersedes for menu/UX:** PRD.md §3 "Non-Goals" item *"A graphical parameter UI"*
(parameter UI is now in v1 scope).

> Related docs: [PRD.md](../../PRD.md) (product spec),
> [IMPLEMENTATION_NOTES.md](../../IMPLEMENTATION_NOTES.md) (engineering reference),
> [README.md](../../README.md) (install + troubleshooting).

---

## 0. Summary

Replace today's single hard-coded `G'MIC...` menu entry (which runs whatever
command sits in `~/.config/gmic-affinity/filter.txt`) with a tree-picker
dialog backed by G'MIC's own filter catalogue. The same `Filters → Plugins →
G'MIC → G'MIC...` menu item now opens an in-process Cocoa modal that shows
~500–700 filters extracted from G'MIC's `#@gui` annotations, organised in
the same folder structure gmic-qt uses, with a parameter form on the right
that defaults to the values gmic itself recommends. User picks a leaf,
optionally tweaks parameters, clicks OK; the existing `gmic.rs` pipeline
runs the chosen filter. Last-Filter and per-filter remembered values give
the dialog a sticky feel.

---

## 1. Decisions made during brainstorming

| # | Decision                                                  | Why                                                                 |
|---|------------------------------------------------------------|---------------------------------------------------------------------|
| 1 | **Real tree dialog** (not flat many-PiPLs menu)            | Photoshop categories are flat one level; deep tree needs a dialog.  |
| 2 | **Auto-extracted from G'MIC's `#@gui` catalogue**          | ~500–700 filters with names, folders, params for ~free.             |
| 3 | **In-process Cocoa UI via the `objc2` crate**              | Single bundle, single language, no sidecar process to install.      |
| 4 | **Bundled snapshot of the catalogue** (not download)       | No network from a plugin; deterministic; works offline.             |
| 5 | **Parameter editing in v1** (with defaults; richer later)  | Added explicitly by user during brainstorming.                      |
| 6 | **Lazy parse at runtime** via `OnceLock` (not `build.rs`)  | Keeps parser as ordinary debuggable runtime code.                   |
| 7 | **Compressed (gzip) snapshot stored in Git LFS**           | Bundle shrinks 10 MB → ~3 MB; large blobs stay out of git proper.   |

---

## 2. Architecture overview

The existing pipeline (`PluginMain` → `FilterRecord` → `gmic.rs` →
`tiff_io.rs`) stays. We bolt a picker on the front and feed its output
into the existing dispatcher instead of `filter.txt`.

New top-level modules:

- `catalogue` — data model: `Catalogue { Folder { children: Vec<Node> } }`,
  `Node = Folder | Filter`, `Filter { display_name, command, description, params: Vec<Param> }`,
  `Param { label, kind: ParamKind }`. Plus a `BUILTIN: OnceLock<Catalogue>`
  populated from the bundled snapshot.
- `catalogue::parser` — parses G'MIC's `#@gui` annotation format into
  `Catalogue`. Pure function over `&str`, fully unit-testable.
- `ui::picker` — `objc2`-based modal `NSPanel` hosting an `NSOutlineView` +
  `NSSearchField` + parameter form + OK/Cancel. Single public function
  `show_picker(catalogue, last_choice) -> Option<ChosenFilter>`.
- `ui::runloop` — thin helper for running a modal panel inside Affinity's
  existing AppKit run loop (the highest-risk surface; see §9).
- `ui::alert` — `NSAlert` wrapper for the error-handling matrix in §5.3.
- `settings` — JSON plist in `~/Library/Application Support/gmic-affinity/`
  storing last-picked filter, recents (for the dialog's Recent
  pseudo-folder), and per-filter remembered argument values.
- `ps_data` — typed Box-leak / raw-pointer-recover / drop helpers for the
  `*data` parameter that the host carries across selectors.

Data flow on one filter invocation:

```text
Affinity → PluginMain(PARAMETERS) → ui::picker → ChosenFilter
                                      ↓
                                    stash in plugin-private *data
Affinity → PluginMain(START)      → unchanged (advance_state etc.)
Affinity → PluginMain(CONTINUE)   → gmic::run_filter_with(fr, &chosen)
Affinity → PluginMain(FINISH)     → drop the Boxed ChosenFilter
```

The existing `~/.config/gmic-affinity/filter.txt` escape hatch is
deprecated by this design. Its read path remains in `gmic.rs` so an
out-of-band tool could still drive the pipeline, but no menu entry or
tree node surfaces it to end users; it is slated for removal once the
picker has shipped a release (§10, open question 8).

---

## 3. Catalogue: format, IR, parser, storage

### 3.1 What the source looks like

G'MIC's filter index is plain text with magic-comment lines that gmic-qt
parses. A representative slice:

```text
#@gui Artistic
#@gui Paint Brush : fx_paint_brush, fx_paint_brush_preview(0)
#@gui : Density (%)  = float(50,0,100)
#@gui : Opacity      = float(0.5,0,1)
#@gui : Radius (px)  = int(5,1,30)
#@gui : Smoothness   = float(2,0,10)
#@gui : note         = note("<small>Author: David Tschumperle.</small>")
#@gui : sep          = separator()
#@gui Lights & Shadows
#@gui Light Glow : fx_light_glow, fx_light_glow_preview(0)
#@gui : Density = int(30,1,100)
#@gui : Mode    = choice(3,"Burn","Dodge","Lighten","Multiply","Overlay","Screen","Soft Light")
```

Conventions we rely on:

- `#@gui Folder` opens or selects a folder; nested folders use `/`.
- `#@gui FilterName : command[, preview_command]` starts a filter under the
  current folder.
- `#@gui : <label> = <typed-default>` appends one row to the current
  filter's parameter list.
- `#@cli`, `#@version`, `#@author` etc. are ignored for v1.

### 3.2 Internal IR

```text
Catalogue { root: Folder }

Folder    { name: String, children: Vec<Node> }
Node      = Folder | Filter
Filter    { display_name, command, description: Option<String>, params: Vec<Param> }

Param     { label: String, kind: ParamKind }
ParamKind = Int       { default: i64, min: i64, max: i64 }
          | Float     { default: f64, min: f64, max: f64 }
          | Bool      { default: bool }
          | Choice    { choices: Vec<String>, default: usize }
          | Color     { default_rgb: [u8; 3] }
          | Text      { default: String }
          | Note(String)        // non-input: static label
          | Separator           // non-input: visual break
          | Link      { label: String, url: String }
          | Unknown(String)     // safety net for unrecognised gmic kinds
```

`ChosenFilter` is what the picker returns:

```text
ChosenFilter { command: String, args: Vec<String> }
// e.g. command = "fx_paint_brush", args = ["50", "0.5", "5", "2"]
```

### 3.3 Parser shape

Line-driven state machine, no external deps:

```text
state ParseState {
    folder_stack: Vec<Folder>,   // current path; root at index 0
    current_filter: Option<Filter>,
}
```

For each line we recognise one of: `folder`, `filter_header`, `param_row`,
`noise`. `folder` rebases the stack. `filter_header` flushes any in-flight
filter into its folder and starts a new one. `param_row` appends to
`current_filter.params`. On EOF we flush.

The `ParamKind` sub-parser is the bulk of the code — twelve string→struct
variants, each unit-testable in isolation. Unrecognised right-hand sides
become `ParamKind::Unknown(raw_string)` rather than aborting the parse;
the dialog renders them as a read-only label and they contribute nothing
to `args`.

### 3.4 Storage and runtime materialisation

```text
assets/gmic-catalogue.gmic.gz       Git LFS    ~3 MB     (the file we include_bytes!)
assets/gmic-catalogue.toc.txt       regular    ~50 KB    (one line per filter, for PR diffs)
assets/gmic-catalogue.version.txt   regular    ~80 bytes (gmic version + ISO refresh timestamp)
```

Runtime init in `catalogue::builtin()`:

```text
static BUILTIN: OnceLock<Catalogue> = OnceLock::new();

pub fn builtin() -> &'static Catalogue {
    BUILTIN.get_or_init(|| {
        const GZ: &[u8] = include_bytes!("../assets/gmic-catalogue.gmic.gz");
        let mut text = String::new();
        flate2::read::GzDecoder::new(GZ).read_to_string(&mut text).unwrap();
        catalogue::parser::parse(&text).unwrap()
    })
}
```

First call: ~30–50 ms (gzip decode) + a few ms parsing. Every subsequent
call: lookup-only, free.

---

## 4. Picker dialog (`ui::picker`)

### 4.1 Window shape

A single modal `NSPanel`, ~720×520 default, resizable, frame persisted
via AppKit's built-in `setFrameAutosaveName` (one-line opt-in, uses
NSUserDefaults under the hood — the only place we touch NSUserDefaults).

```text
┌─────────────────────────────────────────────────────────────┐
│  [search field..........................]                   │  ← NSSearchField
├──────────────────────────┬──────────────────────────────────┤
│ ▼ Recent                 │  Selected: Artistic > Paint Brush│
│   ▸ Artistic             │  ─────────────────────────────── │
│   ▸ Black & White        │  Density (%)   [───●─────] 50    │  ← dynamic param form
│   ▼ Lights & Shadows     │  Opacity       [──●──────] 0.5   │     (NSStackView,
│       Light Glow         │  Radius (px)   [──●──────] 5     │      rebuilt on
│       Lens Flare         │  Smoothness    [─●───────] 2     │      selection change)
│   ▸ Repair               │                                  │
│                          │  Author: David Tschumperlé.      │
├──────────────────────────┴──────────────────────────────────┤
│  Reset defaults                            [Cancel]  [ OK ] │
└─────────────────────────────────────────────────────────────┘
```

Split via `NSSplitView` (vertical), tree pane in an `NSScrollView` with
`NSOutlineView`, params pane in another `NSScrollView` with `NSStackView`.

### 4.2 Tree control

`NSOutlineView` backed by a custom Rust data-source class declared via
`objc2::declare_class!`. The data source returns nodes from a `&Catalogue`;
node identity is the stable index path so AppKit's expand/collapse memory
survives a search-refilter.

Search: on every keystroke we recompute a *visible* index over the
catalogue using a case-insensitive substring match against `display_name`,
expand matching folders, dim non-matches. Empty query restores the saved
expansion state.

A `Recent` pseudo-folder (top of the tree) lists up to ten most-recent
picks from `settings.json`.

### 4.3 Parameter form

One `NSStackView`, rebuilt whenever the outline selection changes to a
`Filter` leaf:

| `ParamKind`   | Control                                              |
|---------------|------------------------------------------------------|
| `Int`         | `NSSlider` + `NSTextField` (both bound to one value) |
| `Float`       | `NSSlider` (continuous) + `NSTextField`              |
| `Bool`        | `NSButton(checkbox)`                                 |
| `Choice`      | `NSPopUpButton`                                      |
| `Color`       | `NSColorWell`                                        |
| `Text`        | `NSTextField`                                        |
| `Note`        | `NSTextField` (label-style, multi-line, no border)   |
| `Separator`   | `NSBox(.separator)`                                  |
| `Link`        | `NSButton(bordered = false)` opening the URL         |
| `Unknown`     | `NSTextField` showing the raw declaration, read-only |

"Reset defaults" repopulates from `ParamKind` defaults without changing
the tree selection.

### 4.4 Buttons, keys, lifecycle

- **OK** only enabled when a leaf (not a folder) is selected. Action:
  read every form control into `Vec<String>`, return `Some(ChosenFilter)`,
  call `NSApp.stopModal(withCode: .OK)`, `orderOut`.
- **Cancel**: `stopModal(withCode: .cancel)`, return `None`.
- **Esc** = Cancel, **Return** = OK (if enabled), **double-click leaf** = OK.

### 4.5 Running modal inside Affinity (the hairy bit)

From `PluginMain` (always on the main thread per SDK contract) we call
`NSApp.runModal(for: panel)` — the standard pattern every Photoshop plugin
uses — and our button handlers call `NSApp.stopModal`. AppKit nests modals
gracefully so this composes correctly even when Affinity is showing its
own progress sheet.

This is the highest-risk piece of the whole feature: see §9 (risk #1) for
the fallback plan and milestone ordering.

---

## 5. PluginMain runtime flow

### 5.1 Selector dispatch

```text
PARAMETERS  selector 1
            ├─ load Catalogue (lazy OnceLock)
            ├─ load last-choice from settings.json
            ├─ ui::picker::show(catalogue, last_choice) -> Option<ChosenFilter>
            │     ├─ Some(chosen): Box::leak, store raw ptr in *data,
            │     │                update settings.json (last + recents + remembered_args),
            │     │                return NO_ERR
            │     └─ None:         return USER_CANCEL  (Affinity aborts silently)

PREPARE     selector 2  ── unchanged; buffer_space / max_space = 0

START       selector 3  ── unchanged; sets in_rect/out_rect, calls advance_state.
                          *data preserved by host across selectors.

CONTINUE    selector 4  ── recover ChosenFilter from *data; if null
                          (Last-Filter case) fall back to settings.json
                          last-choice; if also missing, alert and
                          USER_CANCEL. Call gmic::run_filter_with(fr, &chosen).

FINISH      selector 5  ── if *data is non-null, drop the Boxed
                          ChosenFilter, null *data.
```

### 5.2 `gmic.rs` change

Today `run_filter` reads `filter.txt`, splits on whitespace, builds argv.
We add an overload that takes a `ChosenFilter` directly:

```text
pub fn run_filter_with(fr: &mut FilterRecord, chosen: &ChosenFilter)
    -> Result<(), GmicError>;
```

The existing `MAX_FILTER_ARGS` / `MAX_ARG_BYTES` / NUL & control-char
checks all still apply — they move from "validate the parsed-from-file
string" to "validate the dialog output", and remain the only sanitiser
across both code paths.

### 5.3 Error handling matrix

A new tiny module `ui::alert` exposes:

```text
pub fn alert_error(title: &str, message: &str, log_hint: bool);
// Wraps NSAlert; if log_hint, message gets
// "\n\nSee ~/Library/Logs/gmic-affinity.log for details." appended.
```

Every failure returns `USER_CANCEL` (-1) so Affinity stays silent and
doesn't double-alert over the top of our message.

| Failure                                       | Selector   | NSAlert text                                                                                          |
|-----------------------------------------------|------------|--------------------------------------------------------------------------------------------------------|
| User clicks Cancel in the picker              | PARAMETERS | *(none — their own action)*                                                                            |
| Picker fails to open (AppKit / runloop error) | PARAMETERS | "Couldn't open the G'MIC dialog." + log hint                                                           |
| Catalogue parse fails on first use            | PARAMETERS | "G'MIC filter list is unreadable — this build of the plugin may be corrupted." + log hint              |
| Last-Filter pressed without a previous pick   | CONTINUE   | "No previous G'MIC filter to repeat. Pick one from Filters → Plugins → G'MIC → G'MIC…"                 |
| gmic binary not found                         | CONTINUE   | "G'MIC isn't installed. Install it with: `brew install gmic-qt` and try again."                        |
| gmic exited non-zero                          | CONTINUE   | "G'MIC reported an error running `<command>` (exit N)." + log hint                                     |
| TIFF read / write failure                     | CONTINUE   | "G'MIC produced an image we couldn't read back." + log hint                                            |
| Plugin panic anywhere                         | any        | *(no alert — alerting from a panicking thread is risky)*                                               |

`logging::log` still records full machine-readable detail to
`~/Library/Logs/gmic-affinity.log` for every event; the alert text stays
short and human-friendly. `alert_error` has a `cfg(test)` shim that
captures alert text into a buffer instead of opening AppKit, so the matrix
is table-tested without spawning a window.

### 5.4 Memory safety

Only one heap allocation crosses the FFI boundary per invocation — the
`Box<ChosenFilter>` we leak in PARAMETERS and reclaim in FINISH.
`ps_data` exposes typed `leak / recover / drop` helpers and is the *only*
module that does the pointer dance; everything else gets a `&ChosenFilter`.
A Miri test exercises the leak / recover / drop cycle without any AppKit.

---

## 6. Persistence

### 6.1 File location and format

A single JSON file at
`~/Library/Application Support/gmic-affinity/settings.json`. Plain JSON
(not NSUserDefaults) because the data is ours, lives in our directory, is
trivially inspectable with `cat`, easy to delete, and doesn't get tangled
in Affinity's preferences namespace. The dialog window frame is the one
exception — handled by AppKit's `setFrameAutosaveName` (NSUserDefaults
under the hood, one-line opt-in).

### 6.2 Schema (v1)

```text
{
  "version": 1,
  "last": {
    "command":      "fx_paint_brush",
    "args":         ["50", "0.5", "5", "2"],
    "display_path": "Artistic / Paint Brush",
    "ts":           "2026-05-17T14:58:00Z"
  },
  "recent": [
    { "command": "fx_paint_brush", "display_path": "Artistic / Paint Brush", "ts": "..." },
    { "command": "fx_light_glow",  "display_path": "Lights & Shadows / Light Glow", "ts": "..." }
    // … capped at 10, oldest evicted, deduped by command
  ],
  "remembered_args": {
    "fx_paint_brush": ["50", "0.5", "5", "2"],
    "fx_light_glow":  ["30", "3"]
    // … capped at 256 entries, LRU, total file size capped at 256 KB
  }
}
```

`last` powers Last-Filter repeat (§5.1 CONTINUE fallback) and the
picker's initial selection. `recent` powers the dialog's Recent
pseudo-folder. `remembered_args` gives the dialog a sticky feel.

### 6.3 Read / write semantics

- **Read**: at the start of `PARAMETERS`. ≤1 ms for small JSON; no
  in-memory cache games.
- **Write**: after the picker returns `Some(ChosenFilter)`, and a smaller
  write after Last-Filter (to bump `last.ts` only). Atomic rename: write
  `settings.json.tmp`, `fsync`, `rename()`.
- **Parse failure**: log error, rename file to
  `settings.json.broken-<ts>` so the user can recover it, continue with
  defaults. Never abort the picker for a settings problem.
- **Version drift**: missing `version` → treat as 0 → run migrations
  (currently a no-op). Future `version > current` → log, ignore the
  file, continue with defaults; don't overwrite until the user picks
  something new.

### 6.4 `remembered_args` reconciliation

When the picker opens a filter and finds remembered args, it zips them
positionally with the current `Vec<Param>`:

- If arg count matches and each value parses as the current `ParamKind`,
  use it.
- If anything diverges, fall back to `ParamKind.default` for the
  mismatched rows and log a one-line warning. Don't reject the whole row
  — partial reuse is better than none.

### 6.5 End-user-visible behaviour

- First-ever invocation: dialog opens with Recent empty, tree at root,
  OK disabled.
- Subsequent invocations: dialog opens with last-picked filter
  pre-selected, its remembered values pre-filled, focus on the search
  field.
- Filter → Last Filter: re-runs `last` with `last.args`, no dialog.
- Filter → Last Filter when `last` is empty: NSAlert telling them to
  pick one first.

---

## 7. Build pipeline

### 7.1 New Cargo dependencies

```text
objc2            = "0.5"           # core Obj-C runtime bindings
objc2-foundation = "0.2"           # NSString, NSArray, NSDictionary, NSURL, NSNumber
objc2-app-kit    = "0.2"           # NSPanel, NSOutlineView, NSSearchField, NSStackView,
                                   # NSAlert, NSSlider, NSPopUpButton, NSColorWell, …
serde            = { version = "1", features = ["derive"] }
serde_json       = "1"
flate2           = { version = "1", default-features = false, features = ["rust_backend"] }
```

No `cocoa` / `core-graphics` / `cacao` — `objc2-app-kit` is the modern,
actively-maintained binding set. No `block2` — only synchronous
`runModal` paths, never closure-based callbacks. `flate2` with the pure
`miniz_oxide` backend = zero new C deps.

### 7.2 Assets

| Path                                  | Storage          | Size      | Purpose                                          |
|---------------------------------------|------------------|-----------|--------------------------------------------------|
| `assets/gmic-catalogue.gmic.gz`       | **Git LFS**      | ~3 MB     | The file we `include_bytes!`.                    |
| `assets/gmic-catalogue.toc.txt`       | regular git      | ~50 KB    | Human-readable diff target for PR review.        |
| `assets/gmic-catalogue.version.txt`   | regular git      | ~80 bytes | gmic version + ISO refresh timestamp.            |

`.gitattributes` adds one line:

```text
assets/gmic-catalogue.gmic.gz filter=lfs diff=lfs merge=lfs -text
```

### 7.3 Fail-fast for un-pulled LFS files

In the `bundle` Makefile recipe, immediately before `cargo build`:

```text
@head -c 2 assets/gmic-catalogue.gmic.gz | od -An -tx1 | tr -d ' \n' | \
   grep -q '^1f8b' || { \
     echo ""; \
     echo "ERROR: assets/gmic-catalogue.gmic.gz is not a real gzip file."; \
     echo "       Looks like Git LFS hasn't pulled it. Run:"; \
     echo ""; \
     echo "           git lfs install   # one-time per machine"; \
     echo "           git lfs pull"; \
     echo ""; \
     exit 1; }
```

### 7.4 `make refresh-catalogue` (manual local-dev only)

```text
make refresh-catalogue
   ├── git lfs install        # idempotent
   ├── gmic update >/dev/null
   ├── gzip -9 -c "~/.config/gmic/update<ver>.gmic" > assets/gmic-catalogue.gmic.gz
   ├── cargo run --bin dump-toc > assets/gmic-catalogue.toc.txt
   ├── printf '%s\n%s\n' "$(gmic --version | head -n1)" "$(date -u +%FT%TZ)" \
   │     > assets/gmic-catalogue.version.txt
   ├── cargo test --test catalogue_snapshot
   └── git status -- assets/
```

`src/bin/dump-toc.rs` loads `catalogue::builtin()` and prints one
stable-formatted line per filter (e.g.
`Artistic / Paint Brush  →  fx_paint_brush`), so the TOC stays in sync
with what the parser actually sees.

### 7.5 Existing targets — what changes

- `bundle` / `universal` / `install` / `universal-install` /
  `verify-bundle`: unchanged except the fail-fast LFS check.
- New: `make picker-example` wraps
  `cargo run --example picker --features live` for standalone UI dev
  (~2 s iteration loop, no Affinity).
- `make clean` keeps not touching `assets/`.

### 7.6 CI

GitHub Actions `actions/checkout@v4` step gains `with: { lfs: true }`.
GitHub-hosted macOS runners ship `git-lfs` pre-installed; no other CI
change required. The SDK-less Rez fallback (committed `GmicFilter.rsrc`)
is unchanged.

LFS quota: ~3 MB per stored version × ~12 monthly refreshes per year =
36 MB/year of LFS storage growth. Bandwidth = 3 MB per fresh clone or CI
run; even at 100 of each per month we're at 600 MB, comfortably inside
GitHub's 1 GB/month free tier.

### 7.7 Contributor onboarding (one new line in README)

```text
# After cloning:
git lfs install   # one-time per machine
git lfs pull      # one-time per fresh clone
```

---

## 8. Testing strategy

### 8.1 Automated (runs on every CI build)

| Test family                            | Location                              | What it asserts                                                                                          |
|----------------------------------------|---------------------------------------|----------------------------------------------------------------------------------------------------------|
| `catalogue::parser` unit tests         | `src/catalogue/parser.rs`             | One test per `ParamKind` variant, folder nesting (`/`-separated paths), filter-without-params, parameter-without-filter (error), `note` / `separator` / `link` pass-through, malformed `choice(…)` rejection. ~25 small focused tests. |
| `catalogue` snapshot smoke             | `tests/catalogue_snapshot.rs`         | Bundled `.gmic.gz` decompresses, parses, contains ≥400 filters across ≥15 top-level folders, and known anchor filters (`fx_paint_brush`, `fx_light_glow`) are present at known paths. |
| `settings` round-trip                  | `src/settings.rs`                     | Encode-decode equality; recents capped at 10 with LRU eviction; `remembered_args` capped at 256; broken-file recovery; version-drift handling. |
| `remembered_args` reconciliation       | `src/catalogue/reconcile.rs`          | Zip-by-position with type checks; mismatched arg count falls back per-row; type mismatch falls back per-row; reconciliation never throws. |
| `ps_data` leak / recover / drop        | `src/ps_data.rs`                      | Box-leak, raw-pointer recover, drop-once cycle. Runs under `cargo +nightly miri test ps_data` in a separate CI job. |
| `ui::alert` text formatting            | `src/ui/alert.rs` (test cfg)          | Every error case from §5.3 produces the exact alert text we promised users. AppKit replaced with a `cfg(test)` capture buffer. |
| `ChosenFilter → argv` builder          | `src/gmic.rs`                         | Existing `MAX_FILTER_ARGS` / `MAX_ARG_BYTES` / NUL & control-char rejection still rejects malicious values, now sourced from the dialog. |

All run on the existing `cargo test` / `cargo test --features live` CI
matrix.

### 8.2 Manual pre-release checklist

Goes into `IMPLEMENTATION_NOTES.md`:

1. `cargo run --example picker` — opens the dialog standalone. Verify
   tree, search, parameter form, OK/Cancel, double-click leaf,
   Esc/Return.
2. `make universal-install FEATURES=live`. In Affinity:
   `Filters → Plugins → G'MIC → G'MIC…`. Pick `Artistic / Paint Brush`
   with defaults, OK, expect visible change.
3. Same again, `Filter → Last Filter` — expect re-run without dialog.
4. Quit and relaunch Affinity, open dialog — expect Paint Brush
   pre-selected with the values from step 2.
5. Force each error path: rename `gmic` (NotFound alert);
   corrupt `settings.json` (graceful recovery); click Cancel (silent).

### 8.3 Mock seams

Two thin abstractions exist solely to keep tests off AppKit:

- `ui::alert::Sink` — trait with `display(&self, title, message)`.
  Production impl wraps `NSAlert`; test impl pushes into a
  `Mutex<Vec<(String, String)>>`. Error handlers take `&dyn Sink`.
- `ui::picker::Backend` — same shape but for the picker. Production
  impl wraps `NSPanel.runModal`; tests inject scripted "user picked
  filter X with args Y" or "user clicked Cancel" responses. Lets the §5
  selector flow be table-tested end-to-end without AppKit.

### 8.4 Deliberately NOT automated

- AppKit rendering — too brittle, too expensive. `examples/picker` is
  the source of truth for "the dialog looks right".
- Real `gmic` subprocess execution under unit tests — existing tests
  stub the binary path.
- Affinity itself — no scriptable CI path.

---

## 9. Risks (have a plan for each)

| # | Risk                                                                                                   | Severity | Plan                                                                                                                                                                                                                                                                          |
|---|---------------------------------------------------------------------------------------------------------|----------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| 1 | `NSApp.runModal` from inside Affinity's run loop misbehaves                                             | **High** | Implementation milestone 1 is "open an empty `NSPanel` from `SELECTOR_PARAMETERS`, click Cancel, return cleanly". Fail fast before any UI work. Fallback: `NSWindow.beginSheet`-style sheet attached to Affinity's key window. If both fail we redesign; everything else in §4 is throw-away if the modal path is blocked. |
| 2 | `objc2::declare_class!` learning curve eats more time than expected                                     | Medium   | Start the outline view with one hardcoded folder + one leaf; grow incrementally. Reference `objc2-app-kit` examples + `cacao` source as known-working analogues.                                                                                                              |
| 3 | `#@gui` format quirks: parameter kinds we haven't seen (`file()`, `folder()`, `button()`, `value()`, …) | Medium   | `ParamKind::Unknown(raw_string)` safety net. UI renders Unknown as a read-only label and excludes its row from `args`. Snapshot smoke test logs a one-line warning per Unknown found; we discover new kinds at refresh time, not user-report time.                            |
| 4 | Some filters require gmic-qt-side preview / IPC machinery and fail headless                             | Medium   | Parser excludes filters whose primary command matches well-known gmic-qt-only patterns (`gmic_qt_*`, leading underscores per gmic-qt convention). We will miss some on first pass and discover them via the gmic-error alert; users can avoid them, we add detection rules.   |
| 5 | Some filters change image dimensions (crop, scale, rotate, multi-frame output)                          | Medium   | Today's pipeline assumes `output_dims == input_dims`. Post-gmic, if dims differ we resize back to input dims with nearest-neighbour and log a warning. Long-term fix (Photoshop's `imageSize` selector) is v2 scope. Acceptable degraded behaviour for v1.                     |
| 6 | First-time contributors clone without `git lfs pull`                                                    | Low      | `bundle` Makefile recipe's "is this really a gzip file" gate (§7.3) makes the failure a build-time error with a copy-pasteable fix-it message.                                                                                                                                |
| 7 | Catalogue refresh produces semantic changes our parser silently mis-interprets                          | Low      | `assets/gmic-catalogue.toc.txt` is the PR-review diff target. Anomalies (huge filter count swings, big folder rearrangements) get human-eye review before merge.                                                                                                               |
| 8 | Bundle size grows from ~1 MB to ~10–15 MB (objc2-app-kit + catalogue blob)                              | Low      | Affinity plugins routinely sit at 50–200 MB. Monitor, don't optimise prematurely.                                                                                                                                                                                             |

---

## 10. Open questions (deliberately deferred)

| # | Question                                                                                                | Decision now                                                                                              |
|---|----------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------|
| 1 | Multi-filter chaining ("blur, then sharpen, then contrast" as one invocation)                            | Out of scope. v2 design — likely a "Pipeline" tab in the picker. gmic supports it natively.               |
| 2 | "Favourites" star next to filter names                                                                   | Defer. Recent + stickiness covers most of the value; revisit after first usage feedback.                  |
| 3 | Long-parameter-list UX (some filters have 20+ params)                                                    | Start with scrollable form; observe; iterate.                                                             |
| 4 | Affinity by Canva v3 plugins-folder install path differs from Affinity Photo 2                           | Independent of this design. `make install` learns about it separately.                                    |
| 5 | Live preview inside the picker                                                                            | Out of scope for v1. Massive complexity (round-trip gmic per parameter change).                           |
| 6 | Settings.json access races when two Affinity windows / apps run simultaneously                            | Accept last-writer-wins. Add file locking only if a user reports a conflict.                              |
| 7 | i18n of the dialog (gmic-qt translates filter names; gmic catalogue itself is English-only)               | English-only for v1.                                                                                      |
| 8 | Fully removing the `~/.config/gmic-affinity/filter.txt` legacy code path                                  | Keep the *read* path in `gmic.rs` (deprecated, undocumented) until the picker has shipped one release. Remove in the release after that. |

---

## 11. Implementation milestone ordering

Designed so the highest-risk piece (§9 #1) is proven first, and every
later milestone builds on already-de-risked ground.

1. **Empty modal panel** — proves `NSApp.runModal` works inside Affinity.
2. **Static one-folder, one-filter outline view** with hard-coded data —
   proves the `objc2` data-source pattern works.
3. **Catalogue parser + `tests/catalogue_snapshot.rs`** — pure Rust, no
   UI, no Affinity. Can land in parallel with #1/#2.
4. **Outline view wired to real catalogue** — connects #2 and #3.
5. **Parameter form** — adds the right-hand pane.
6. **Persistence (`settings.rs`) + Last-Filter behaviour** — last
   because it's straightforward and benefits from having the rest
   working.
7. **`ui::alert` and the full error-handling matrix.**
8. **End-to-end manual checklist + docs.**

---

*End of design.*
