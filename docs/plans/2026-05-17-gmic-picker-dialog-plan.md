# G'MIC Picker Dialog Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the single hard-coded `G'MIC...` menu entry with an in-process Cocoa modal that browses G'MIC's full ~500–700 filter catalogue as a searchable tree with a parameter form, persists user choices, and feeds the result into the existing gmic pipeline.

**Architecture:** Add `catalogue` (data + parser), `ui::picker` (objc2 NSPanel), `ui::alert` (NSAlert wrapper), `settings` (JSON state), and `ps_data` (FFI Box-leak helpers) modules. Rewire `PluginMain` to open the picker in `SELECTOR_PARAMETERS`, stash the chosen filter in plugin-private `*data`, recover it in `CONTINUE`, drop it in `FINISH`. The existing `FilterRecord`, `gmic.rs`, and `tiff_io.rs` pipeline is unchanged except for one new `run_filter_with(&ChosenFilter)` entry point on `gmic.rs`.

**Tech Stack:** Rust stable, `objc2` + `objc2-app-kit` + `objc2-foundation` for Cocoa, `serde` + `serde_json` for settings, `flate2` (pure-Rust `miniz_oxide` backend) for the bundled catalogue, `tempfile` (already present) for gmic subprocess scratch. Build via existing `staticlib` + `clang -bundle` Makefile; LFS-tracked catalogue snapshot.

> Source-of-truth design: [docs/design/2026-05-17-gmic-picker-dialog.md](../design/2026-05-17-gmic-picker-dialog.md).
> Cross-reference: [PRD.md](../../PRD.md), [IMPLEMENTATION_NOTES.md](../../IMPLEMENTATION_NOTES.md).

---

## Parallelization strategy

Tasks are grouped into five waves. Within a wave the marker `[PARALLEL]` flags tasks that have zero file overlap and zero shared state and can be dispatched as concurrent subagents. `[SEQUENTIAL]` tasks must complete before the next one starts.

| Wave | Tasks                        | Mode                                                                              |
|------|------------------------------|-----------------------------------------------------------------------------------|
| 1    | T1                           | `[SEQUENTIAL]` — de-risks AppKit modal from inside Affinity. Must run alone, first. |
| 2    | T2, T3, T4, T5               | `[PARALLEL]` — pure-Rust, isolated modules. **Dispatch all 4 as concurrent subagents.** |
| 3    | T6 → T7 → T8 → T9            | `[SEQUENTIAL]` — UI grows incrementally; each task touches the picker module.     |
| 4    | T10 → T11 → T12 → T13 → T14  | `[SEQUENTIAL]` — `PluginMain` rewiring; each task touches `src/lib.rs`.           |
| 5    | T15, T16, T17, T18, T19      | `[PARALLEL within wave]` after T14 lands — docs / build / CI / examples touch different files. |

After Wave 2 finishes, the main coordinator session runs a quick integration check (`cargo test --features live`) before opening Wave 3.

---

## File structure

### New files

| Path                                              | Owner task | Responsibility                                                                                 |
|---------------------------------------------------|------------|------------------------------------------------------------------------------------------------|
| `src/catalogue/mod.rs`                            | T2         | `Catalogue`, `Folder`, `Node`, `Filter`, `Param`, `ParamKind`, `ChosenFilter` types + `builtin()`. |
| `src/catalogue/parser.rs`                         | T2         | Pure-Rust `#@gui` line-driven parser. ~25 unit tests.                                          |
| `src/catalogue/reconcile.rs`                      | T2         | `remembered_args` ↔ current params reconciliation.                                             |
| `src/settings.rs`                                 | T3         | Read/write `~/Library/Application Support/gmic-affinity/settings.json`.                        |
| `src/ps_data.rs`                                  | T4         | Box-leak / raw-ptr-recover / drop helpers for `FilterRecord`'s `*data` field.                  |
| `src/ui/mod.rs`                                   | T1         | `pub mod alert; pub mod picker; pub mod runloop;`                                              |
| `src/ui/runloop.rs`                               | T1         | Thin `NSApp.runModal` helper. The de-risking surface.                                          |
| `src/ui/alert.rs`                                 | T5         | `Sink` trait + production `NSAlert` impl + test capture impl + `alert_error` helper.           |
| `src/ui/picker.rs`                                | T1/T7-T10  | Public `show_picker(&Catalogue, last_choice) -> Option<ChosenFilter>` + Cocoa internals.       |
| `src/bin/dump-toc.rs`                             | T6         | Tiny CLI that dumps the parsed catalogue as a stable text TOC.                                 |
| `examples/picker.rs`                              | T15        | Standalone executable opening the picker outside Affinity for UI dev iteration.                |
| `tests/catalogue_snapshot.rs`                     | T6         | Smoke test: bundled `.gz` decompresses, parses, anchor filters present.                        |
| `assets/gmic-catalogue.gmic.gz`                   | T6         | LFS-tracked snapshot of `~/.config/gmic/update<ver>.gmic`, gzip-9.                              |
| `assets/gmic-catalogue.toc.txt`                   | T6         | Plain-text TOC for PR-diff readability.                                                        |
| `assets/gmic-catalogue.version.txt`               | T6         | Two-line: gmic version + ISO refresh timestamp.                                                |
| `.gitattributes`                                  | T6         | One line directing the `.gz` through Git LFS.                                                  |

### Modified files

| Path                            | Owner task        | What changes                                                                            |
|---------------------------------|-------------------|-----------------------------------------------------------------------------------------|
| `Cargo.toml`                    | T1                | Adds `objc2`, `objc2-foundation`, `objc2-app-kit`, `serde`, `serde_json`, `flate2`.    |
| `src/lib.rs`                    | T1, T11–T14       | New module declarations; `PARAMETERS`/`CONTINUE`/`FINISH` selector handlers expanded.   |
| `src/gmic.rs`                   | T12               | Adds `pub fn run_filter_with(fr, chosen: &ChosenFilter) -> Result<…, GmicError>`.       |
| `Makefile`                      | T16               | `refresh-catalogue`, `picker-example`, LFS fail-fast in `bundle`.                       |
| `.github/workflows/ci.yml`      | T17               | `actions/checkout@v4` gains `with: lfs: true`.                                          |
| `PRD.md`                        | T18               | Status table flips "Parameter UI" from ⛔ v2 to ✅ shipped.                              |
| `IMPLEMENTATION_NOTES.md`       | T18, T19          | New section on the picker; new manual e2e checklist.                                    |
| `README.md`                     | T18               | One-line `git lfs install` / `git lfs pull` onboarding step.                            |

---

# Wave 1 — De-risking AppKit modal (sequential)

## Task 1: Empty modal panel from `SELECTOR_PARAMETERS`

`[SEQUENTIAL]` — must complete and be verified inside Affinity before any other UI work begins.

**Files:**
- Create: `src/ui/mod.rs`
- Create: `src/ui/runloop.rs`
- Create: `src/ui/picker.rs`
- Modify: `Cargo.toml`
- Modify: `src/lib.rs` (add `pub mod ui;` and call into `ui::picker::show_empty` from `SELECTOR_PARAMETERS` under `#[cfg(feature = "live")]`)

---

- [ ] **Step 1: Add Cocoa dependencies to `Cargo.toml`**

Append under `[dependencies]`:

```toml
objc2            = "0.5"
objc2-foundation = "0.2"
objc2-app-kit    = "0.2"
serde            = { version = "1", features = ["derive"] }
serde_json       = "1"
flate2           = { version = "1", default-features = false, features = ["rust_backend"] }
```

Run: `cargo build --features live`
Expected: clean build, fetches the new crates. No code yet so just dependency wiring.

- [ ] **Step 2: Create `src/ui/mod.rs`**

```rust
//! macOS AppKit-facing UI for the plugin.
//!
//! Everything in this module needs to run on the main thread, where
//! Affinity calls `PluginMain` for us. None of it is reachable when the
//! crate is built without the `live` feature.

#![cfg(feature = "live")]

pub mod runloop;
pub mod picker;
```

- [ ] **Step 3: Create `src/ui/runloop.rs` with the bare-minimum `NSPanel` opener**

```rust
//! Wrapper around `NSApp.runModal(for:)` so the rest of the UI never
//! has to touch `objc2` directly for the run-loop dance.
//!
//! The first milestone of the picker design is "open an empty `NSPanel`
//! from `SELECTOR_PARAMETERS`, click Cancel, return cleanly to
//! Affinity". This file is the surface that proves that works. If
//! `runModal` misbehaves inside Affinity's run loop, this is where
//! we will find out and decide on the sheet-based fallback.

use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSModalResponse, NSWindow};
use objc2_foundation::MainThreadMarker;

/// Show `window` modally and block until it is dismissed. Returns the
/// AppKit modal response code (`.OK` / `.cancel` / a custom int).
///
/// MUST be called on the main thread. `PluginMain` is always invoked
/// on the main thread per the Photoshop SDK contract.
pub fn run_modal_window(window: &Retained<NSWindow>) -> NSModalResponse {
    let mtm = MainThreadMarker::new()
        .expect("ui::runloop::run_modal_window must be called on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    window.makeKeyAndOrderFront(None);
    let response = unsafe { app.runModalForWindow(window) };
    window.orderOut(None);
    response
}
```

> **objc2 note for the implementer:** the exact `Retained<…>`, `MainThreadMarker`, and `sharedApplication(mtm)` spellings have shifted between `objc2` 0.4 and 0.5. If the build complains, consult `cargo doc --open --package objc2-app-kit` and the upstream examples at <https://github.com/madsmtm/objc2/tree/master/examples>. Adjust types/imports as the current API requires; the *shape* of the function (resolve main-thread marker → shared app → makeKey → runModal → orderOut) is the contract this task is verifying.

- [ ] **Step 4: Create `src/ui/picker.rs` with a single public `show_empty` that opens a blank panel**

```rust
//! The user-facing picker dialog. For Wave 1 this is a stub that opens
//! an empty `NSPanel` with OK / Cancel buttons. Each subsequent UI
//! task (T7 → T8 → T9 → T10) layers in real content here.

use objc2::rc::Retained;
use objc2_app_kit::{
    NSBackingStoreType, NSPanel, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{CGRect, CGPoint, CGSize, MainThreadMarker};

use crate::ui::runloop::run_modal_window;

/// Open an empty panel with a title bar and a close button. Blocks
/// until the user dismisses it. Returns `Some(())` on close, `None`
/// on any internal error — for the stub these are indistinguishable;
/// real return types come in T10.
pub fn show_empty() -> Option<()> {
    let mtm = MainThreadMarker::new()?;

    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Resizable;
    let content_rect = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize { width: 720.0, height: 520.0 },
    };

    let panel: Retained<NSPanel> = unsafe {
        NSPanel::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            content_rect,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    let window: Retained<NSWindow> = panel.into_super();
    window.setTitle(&objc2_foundation::NSString::from_str("G'MIC (stub)"));

    let _response = run_modal_window(&window);
    Some(())
}
```

> **objc2 note:** `init_with_content_rect_…` / `initWithContentRect…` casing changed between releases. Method-name spelling above is the snake_case projection objc2 0.5 currently uses; if your toolchain wants camelCase or a slightly different selector, follow what `cargo build` tells you. Don't fight the macro.

- [ ] **Step 5: Wire `SELECTOR_PARAMETERS` to call `ui::picker::show_empty`**

In `src/lib.rs`, find the `SELECTOR_PARAMETERS` arm of the `#[cfg(feature = "live")] fn dispatch` (or the equivalent in the default no-op build — for the live build it's where parameters would be handled). Add:

```rust
SELECTOR_PARAMETERS => {
    log(&format!("PARAMETERS: opening picker (stub)"));
    match ui::picker::show_empty() {
        Some(()) => NO_ERR,
        None => USER_CANCEL,
    }
}
```

Add at the top of `src/lib.rs`:

```rust
#[cfg(feature = "live")]
pub mod ui;
```

- [ ] **Step 6: Build the universal bundle and install**

Run:

```bash
make universal-install FEATURES=live
```

Expected: clean build of both arm64 and x86_64 slices; `make verify-bundle` passes (filetype 8 / `MH_BUNDLE` on both slices); plugin copied into `~/Library/Application Support/Affinity Photo 2/Plugins/`.

- [ ] **Step 7: Manual verification inside Affinity Photo 2**

1. Fully quit Affinity Photo 2 (`Cmd-Q`).
2. Relaunch it; open any 8-bit RGB document.
3. Choose **Filter → Plugins → G'MIC → G'MIC...**.
4. Verify an empty modal panel titled "G'MIC (stub)" appears.
5. Click its close button.
6. Verify Affinity is responsive afterwards (no hang, no crash).
7. Check `~/Library/Logs/gmic-affinity.log`: a `PARAMETERS: opening picker (stub)` line must be present.

If any step fails, fall back to the design's fallback plan (NSWindow.beginSheet sheet) before continuing.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/ui
git commit -m "feat(ui): de-risk empty NSPanel modal from PluginMain"
```

---

# Wave 2 — Parallel pure-Rust foundation `[PARALLEL]`

The following four tasks have **zero file overlap** and **no shared state**. Dispatch them as four concurrent subagents.

Each agent gets a focused prompt: "implement Task T<n> from `docs/plans/2026-05-17-gmic-picker-dialog-plan.md`. Read the source design at `docs/design/2026-05-17-gmic-picker-dialog.md` for context. Do not modify any file outside the Owner-task list at the top of this plan; specifically you may not touch `src/lib.rs`, `src/ui/*.rs`, or `src/gmic.rs`. Commit at the end of your task and report back the commit SHA + summary."

After all four agents return, the coordinator runs `cargo test` and `cargo test --features live` to confirm no cross-task breakage before opening Wave 3.

---

## Task 2: `catalogue` module + `#@gui` parser

`[PARALLEL with T3, T4, T5]`

**Files:**
- Create: `src/catalogue/mod.rs`
- Create: `src/catalogue/parser.rs`
- Create: `src/catalogue/reconcile.rs`
- Modify: `src/lib.rs` (one line: `pub mod catalogue;`)

---

- [ ] **Step 1: Create `src/catalogue/mod.rs` with the data types**

```rust
//! Parsed G'MIC filter catalogue + supporting types.
//!
//! The IR mirrors what gmic-qt's filter browser shows: a tree of
//! folders containing filters, each filter carrying a `command`, an
//! optional `description`, and a flat parameter list.

pub mod parser;
pub mod reconcile;

use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq)]
pub struct Catalogue {
    pub root: Folder,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Folder {
    pub name: String,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Folder(Folder),
    Filter(Filter),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    pub display_name: String,
    pub command: String,
    pub description: Option<String>,
    pub params: Vec<Param>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub label: String,
    pub kind: ParamKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamKind {
    Int { default: i64, min: i64, max: i64 },
    Float { default: f64, min: f64, max: f64 },
    Bool { default: bool },
    Choice { choices: Vec<String>, default: usize },
    Color { default_rgb: [u8; 3] },
    Text { default: String },
    Note(String),
    Separator,
    Link { label: String, url: String },
    Unknown(String),
}

/// What the picker hands back when the user clicks OK.
#[derive(Debug, Clone, PartialEq)]
pub struct ChosenFilter {
    pub command: String,
    pub args: Vec<String>,
}

/// Lazily-decoded bundled catalogue. T6 populates `BUNDLED_GZ` with
/// real content; until then we expose a tiny placeholder so this
/// module compiles standalone.
static BUILTIN: OnceLock<Catalogue> = OnceLock::new();

#[cfg(not(test))]
pub fn builtin() -> &'static Catalogue {
    BUILTIN.get_or_init(|| {
        // Populated for real in T6 via include_bytes!() + flate2.
        // Until then, returning an empty catalogue lets the rest of
        // the modules compile and unit-test in isolation.
        Catalogue { root: Folder { name: String::new(), children: Vec::new() } }
    })
}

#[cfg(test)]
pub fn builtin() -> &'static Catalogue {
    BUILTIN.get_or_init(|| Catalogue {
        root: Folder { name: String::new(), children: Vec::new() },
    })
}
```

- [ ] **Step 2: Add `pub mod catalogue;` to `src/lib.rs`**

Right next to where `pub mod logging;` lives (i.e. unconditional, not `#[cfg(feature = "live")]`):

```rust
pub mod catalogue;
```

Run: `cargo check`
Expected: clean compile.

- [ ] **Step 3: Write the parser skeleton with the public entry point in `src/catalogue/parser.rs`**

```rust
//! Line-driven parser for G'MIC's `#@gui` annotation format.
//!
//! Format spec we rely on:
//! - `#@gui FolderPath` opens or selects a folder. `/` nests.
//! - `#@gui Display Name : command[, preview_command]` opens a
//!   filter inside the current folder.
//! - `#@gui : <label> = <typed-default>` appends one Param row to
//!   the currently-open filter.
//! - Any other `#@gui` row is treated as noise for v1.
//! - Lines that don't start with `#@gui` are ignored.

use crate::catalogue::{Catalogue, Filter, Folder, Node, Param, ParamKind};

#[derive(Debug)]
pub enum ParseError {
    OrphanParam { line: usize, raw: String },
    Malformed { line: usize, reason: String, raw: String },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OrphanParam { line, raw } => {
                write!(f, "line {line}: parameter row without an open filter: {raw}")
            }
            Self::Malformed { line, reason, raw } => {
                write!(f, "line {line}: {reason}: {raw}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

pub fn parse(input: &str) -> Result<Catalogue, ParseError> {
    let mut state = ParseState::new();
    for (idx, line) in input.lines().enumerate() {
        state.consume(idx + 1, line)?;
    }
    state.finish()
}

struct ParseState {
    folder_stack: Vec<Folder>,
    current_filter: Option<Filter>,
}

impl ParseState {
    fn new() -> Self {
        Self {
            folder_stack: vec![Folder { name: String::new(), children: Vec::new() }],
            current_filter: None,
        }
    }

    fn consume(&mut self, line_no: usize, raw: &str) -> Result<(), ParseError> {
        let Some(body) = raw.trim_start().strip_prefix("#@gui") else {
            return Ok(());
        };
        let body = body.trim_start();

        if let Some(rest) = body.strip_prefix(':') {
            self.consume_param_row(line_no, rest.trim_start())?;
        } else if body.contains(':') {
            self.consume_filter_header(line_no, body)?;
        } else if !body.is_empty() {
            self.consume_folder(body);
        }
        Ok(())
    }

    fn consume_folder(&mut self, body: &str) {
        self.flush_filter();
        let path = body.trim();
        let segments: Vec<&str> = path.split('/').map(str::trim).collect();
        // Pop back to root, then push each segment as a new folder
        // (we don't merge with existing folders for v1; gmic catalogues
        // never re-open a folder, but if one ever does we'll merge here).
        while self.folder_stack.len() > 1 {
            let done = self.folder_stack.pop().unwrap();
            self.folder_stack.last_mut().unwrap().children.push(Node::Folder(done));
        }
        for seg in segments {
            self.folder_stack.push(Folder { name: seg.to_string(), children: Vec::new() });
        }
    }

    fn consume_filter_header(&mut self, _line_no: usize, body: &str) -> Result<(), ParseError> {
        self.flush_filter();
        let (name_part, command_part) = body.split_once(':').unwrap();
        let display_name = name_part.trim().to_string();
        // command_part = "fx_paint_brush, fx_paint_brush_preview(0)" — we only keep the first token.
        let command = command_part
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        self.current_filter = Some(Filter {
            display_name,
            command,
            description: None,
            params: Vec::new(),
        });
        Ok(())
    }

    fn consume_param_row(&mut self, line_no: usize, body: &str) -> Result<(), ParseError> {
        let Some(filter) = self.current_filter.as_mut() else {
            return Err(ParseError::OrphanParam { line: line_no, raw: body.to_string() });
        };
        let Some((label_part, decl_part)) = body.split_once('=') else {
            // Some rows are bare separators with no '='; treat as Note.
            filter.params.push(Param {
                label: body.trim().to_string(),
                kind: ParamKind::Note(body.trim().to_string()),
            });
            return Ok(());
        };
        let label = label_part.trim().to_string();
        let kind = parse_kind(decl_part.trim());
        filter.params.push(Param { label, kind });
        Ok(())
    }

    fn flush_filter(&mut self) {
        if let Some(filter) = self.current_filter.take() {
            self.folder_stack.last_mut().unwrap().children.push(Node::Filter(filter));
        }
    }

    fn finish(mut self) -> Result<Catalogue, ParseError> {
        self.flush_filter();
        while self.folder_stack.len() > 1 {
            let done = self.folder_stack.pop().unwrap();
            self.folder_stack.last_mut().unwrap().children.push(Node::Folder(done));
        }
        let root = self.folder_stack.pop().unwrap();
        Ok(Catalogue { root })
    }
}

fn parse_kind(decl: &str) -> ParamKind {
    let Some(open) = decl.find('(') else {
        return ParamKind::Unknown(decl.to_string());
    };
    let (head, rest) = decl.split_at(open);
    let inner = rest
        .strip_prefix('(')
        .and_then(|s| s.rsplit_once(')'))
        .map(|(args, _)| args)
        .unwrap_or("");
    match head.trim() {
        "int"       => parse_int(inner),
        "float"     => parse_float(inner),
        "bool"      => parse_bool(inner),
        "choice"    => parse_choice(inner),
        "color"     => parse_color(inner),
        "text"      => parse_text(inner),
        "note"      => ParamKind::Note(strip_quotes(inner).to_string()),
        "separator" => ParamKind::Separator,
        "link"      => parse_link(inner),
        _           => ParamKind::Unknown(decl.to_string()),
    }
}

fn parse_int(s: &str) -> ParamKind {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    match parts.as_slice() {
        [d, lo, hi] => match (d.parse(), lo.parse(), hi.parse()) {
            (Ok(default), Ok(min), Ok(max)) => ParamKind::Int { default, min, max },
            _ => ParamKind::Unknown(format!("int({s})")),
        },
        _ => ParamKind::Unknown(format!("int({s})")),
    }
}

fn parse_float(s: &str) -> ParamKind {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    match parts.as_slice() {
        [d, lo, hi] => match (d.parse(), lo.parse(), hi.parse()) {
            (Ok(default), Ok(min), Ok(max)) => ParamKind::Float { default, min, max },
            _ => ParamKind::Unknown(format!("float({s})")),
        },
        _ => ParamKind::Unknown(format!("float({s})")),
    }
}

fn parse_bool(s: &str) -> ParamKind {
    match s.trim() {
        "true"  | "1" => ParamKind::Bool { default: true },
        "false" | "0" => ParamKind::Bool { default: false },
        other          => ParamKind::Unknown(format!("bool({other})")),
    }
}

fn parse_choice(s: &str) -> ParamKind {
    // choice(default_idx, "a", "b", "c", ...)
    let mut iter = split_top_level(s);
    let default = iter.next().and_then(|d| d.trim().parse().ok()).unwrap_or(0);
    let choices: Vec<String> = iter.map(|c| strip_quotes(c).to_string()).collect();
    if choices.is_empty() {
        ParamKind::Unknown(format!("choice({s})"))
    } else {
        ParamKind::Choice { choices, default }
    }
}

fn parse_color(s: &str) -> ParamKind {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return ParamKind::Unknown(format!("color({s})"));
    }
    match (parts[0].parse(), parts[1].parse(), parts[2].parse()) {
        (Ok(r), Ok(g), Ok(b)) => ParamKind::Color { default_rgb: [r, g, b] },
        _ => ParamKind::Unknown(format!("color({s})")),
    }
}

fn parse_text(s: &str) -> ParamKind {
    ParamKind::Text { default: strip_quotes(s).to_string() }
}

fn parse_link(s: &str) -> ParamKind {
    let mut iter = split_top_level(s);
    let label = iter.next().map(strip_quotes).unwrap_or("").to_string();
    let url = iter.next().map(strip_quotes).unwrap_or("").to_string();
    if label.is_empty() && url.is_empty() {
        ParamKind::Unknown(format!("link({s})"))
    } else {
        ParamKind::Link { label, url }
    }
}

fn strip_quotes(s: &str) -> &str {
    s.trim().trim_matches('"')
}

/// Split a comma-separated list that may contain quoted strings
/// containing commas. Quote-aware, no escapes (gmic doesn't use them).
fn split_top_level(s: &str) -> impl Iterator<Item = &str> {
    let mut depth = 0i32;
    let mut in_quote = false;
    let mut start = 0;
    let mut out: Vec<&str> = Vec::new();
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'"' => in_quote = !in_quote,
            b'(' if !in_quote => depth += 1,
            b')' if !in_quote => depth -= 1,
            b',' if !in_quote && depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out.into_iter()
}
```

Run: `cargo build`
Expected: clean compile.

- [ ] **Step 4: Add unit tests covering every `ParamKind` and the folder/filter state machine**

Append to `src/catalogue/parser.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::{Node, ParamKind};

    fn first_filter<'a>(cat: &'a Catalogue) -> &'a Filter {
        fn walk<'a>(folder: &'a Folder) -> Option<&'a Filter> {
            for child in &folder.children {
                match child {
                    Node::Filter(f) => return Some(f),
                    Node::Folder(f) => {
                        if let Some(found) = walk(f) {
                            return Some(found);
                        }
                    }
                }
            }
            None
        }
        walk(&cat.root).expect("expected at least one filter")
    }

    #[test]
    fn parses_int_param() {
        let cat = parse("#@gui Artistic\n#@gui Paint : fx_paint_brush\n#@gui : Radius = int(5,1,30)\n").unwrap();
        let p = &first_filter(&cat).params[0];
        assert_eq!(p.label, "Radius");
        assert_eq!(p.kind, ParamKind::Int { default: 5, min: 1, max: 30 });
    }

    #[test]
    fn parses_float_param() {
        let cat = parse("#@gui Artistic\n#@gui Paint : fx_paint_brush\n#@gui : Density (%) = float(50,0,100)\n").unwrap();
        let p = &first_filter(&cat).params[0];
        assert_eq!(p.label, "Density (%)");
        assert_eq!(p.kind, ParamKind::Float { default: 50.0, min: 0.0, max: 100.0 });
    }

    #[test]
    fn parses_bool_param() {
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : On = bool(true)\n").unwrap();
        assert_eq!(first_filter(&cat).params[0].kind, ParamKind::Bool { default: true });
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : On = bool(0)\n").unwrap();
        assert_eq!(first_filter(&cat).params[0].kind, ParamKind::Bool { default: false });
    }

    #[test]
    fn parses_choice_with_commas_inside_strings() {
        let cat = parse(
            "#@gui A\n#@gui F : f\n#@gui : Mode = choice(2,\"Red, Green\",\"Other\")\n",
        ).unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Choice {
                choices: vec!["Red, Green".into(), "Other".into()],
                default: 2,
            },
        );
    }

    #[test]
    fn parses_color_param() {
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : Tint = color(255,0,128)\n").unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Color { default_rgb: [255, 0, 128] },
        );
    }

    #[test]
    fn parses_text_with_quotes() {
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : Caption = text(\"Hello\")\n").unwrap();
        assert_eq!(
            first_filter(&cat).params[0].kind,
            ParamKind::Text { default: "Hello".into() },
        );
    }

    #[test]
    fn parses_note_and_separator_and_link() {
        let cat = parse(
            "#@gui A\n#@gui F : f\n\
             #@gui : note = note(\"author\")\n\
             #@gui : sep = separator()\n\
             #@gui : help = link(\"docs\",\"https://example.com\")\n",
        ).unwrap();
        let params = &first_filter(&cat).params;
        assert_eq!(params[0].kind, ParamKind::Note("author".into()));
        assert_eq!(params[1].kind, ParamKind::Separator);
        assert_eq!(
            params[2].kind,
            ParamKind::Link { label: "docs".into(), url: "https://example.com".into() },
        );
    }

    #[test]
    fn unknown_kind_does_not_fail_the_parse() {
        let cat = parse("#@gui A\n#@gui F : f\n#@gui : X = wat(1,2,3)\n").unwrap();
        assert!(matches!(first_filter(&cat).params[0].kind, ParamKind::Unknown(_)));
    }

    #[test]
    fn orphan_param_is_an_error() {
        assert!(matches!(
            parse("#@gui : Radius = int(1,0,10)\n"),
            Err(ParseError::OrphanParam { .. }),
        ));
    }

    #[test]
    fn nested_folders_via_slashes() {
        let cat = parse("#@gui Artistic/Painting\n#@gui Oil : fx_oilpaint\n").unwrap();
        let outer = match &cat.root.children[0] {
            Node::Folder(f) => f,
            _ => panic!("expected folder"),
        };
        assert_eq!(outer.name, "Artistic");
        let inner = match &outer.children[0] {
            Node::Folder(f) => f,
            _ => panic!("expected nested folder"),
        };
        assert_eq!(inner.name, "Painting");
        assert!(matches!(inner.children[0], Node::Filter(_)));
    }

    #[test]
    fn two_filters_under_one_folder() {
        let cat = parse(
            "#@gui Artistic\n\
             #@gui Paint : fx_paint_brush\n\
             #@gui Oil : fx_oilpaint\n",
        ).unwrap();
        let folder = match &cat.root.children[0] {
            Node::Folder(f) => f,
            _ => panic!(),
        };
        assert_eq!(folder.children.len(), 2);
    }

    #[test]
    fn whitespace_only_lines_are_ignored() {
        let cat = parse("\n\n#@gui A\n\n   \n#@gui F : f\n").unwrap();
        assert_eq!(first_filter(&cat).command, "f");
    }

    #[test]
    fn comment_after_filter_command_strips_preview() {
        let cat = parse(
            "#@gui A\n#@gui F : fx_real, fx_real_preview(0)\n",
        ).unwrap();
        assert_eq!(first_filter(&cat).command, "fx_real");
    }
}
```

Run: `cargo test --lib catalogue::parser`
Expected: PASS — at least 13 tests green.

- [ ] **Step 5: Write `src/catalogue/reconcile.rs`**

```rust
//! Reconcile a saved `remembered_args` Vec<String> against the
//! current filter's parameter list. Out-of-band drift between gmic
//! releases means we can't assume the saved vector lines up; this
//! module picks per-row whether to keep the saved value or fall back
//! to the parameter's default.

use crate::catalogue::{Param, ParamKind};

/// Returns a `Vec<String>` of length `params.len()` suitable for
/// pre-filling the picker's parameter form. For each position:
/// - If `remembered` has a value at this position and it parses as
///   the current `ParamKind`, the remembered value is used.
/// - Otherwise the default is computed from the `ParamKind`.
pub fn reconcile(remembered: &[String], params: &[Param]) -> Vec<String> {
    params
        .iter()
        .enumerate()
        .map(|(i, p)| {
            remembered
                .get(i)
                .filter(|v| value_matches_kind(v, &p.kind))
                .cloned()
                .unwrap_or_else(|| default_for(&p.kind))
        })
        .collect()
}

fn value_matches_kind(value: &str, kind: &ParamKind) -> bool {
    match kind {
        ParamKind::Int { min, max, .. } => value
            .parse::<i64>()
            .map(|v| v >= *min && v <= *max)
            .unwrap_or(false),
        ParamKind::Float { min, max, .. } => value
            .parse::<f64>()
            .map(|v| v >= *min && v <= *max)
            .unwrap_or(false),
        ParamKind::Bool { .. } => matches!(value, "true" | "false" | "0" | "1"),
        ParamKind::Choice { choices, .. } => choices.iter().any(|c| c == value),
        ParamKind::Color { .. } => value.split(',').count() == 3
            && value.split(',').all(|c| c.trim().parse::<u8>().is_ok()),
        ParamKind::Text { .. } => true,
        ParamKind::Note(_) | ParamKind::Separator | ParamKind::Link { .. } => true,
        ParamKind::Unknown(_) => true,
    }
}

fn default_for(kind: &ParamKind) -> String {
    match kind {
        ParamKind::Int { default, .. } => default.to_string(),
        ParamKind::Float { default, .. } => default.to_string(),
        ParamKind::Bool { default } => default.to_string(),
        ParamKind::Choice { choices, default } => choices
            .get(*default)
            .cloned()
            .unwrap_or_default(),
        ParamKind::Color { default_rgb } => format!(
            "{},{},{}",
            default_rgb[0], default_rgb[1], default_rgb[2],
        ),
        ParamKind::Text { default } => default.clone(),
        ParamKind::Note(_) | ParamKind::Separator | ParamKind::Link { .. } => String::new(),
        ParamKind::Unknown(s) => s.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(label: &str, kind: ParamKind) -> Param {
        Param { label: label.into(), kind }
    }

    #[test]
    fn matching_int_is_kept() {
        let params = vec![p("R", ParamKind::Int { default: 5, min: 0, max: 10 })];
        let out = reconcile(&["7".into()], &params);
        assert_eq!(out, vec!["7".to_string()]);
    }

    #[test]
    fn out_of_range_int_falls_back() {
        let params = vec![p("R", ParamKind::Int { default: 5, min: 0, max: 10 })];
        let out = reconcile(&["99".into()], &params);
        assert_eq!(out, vec!["5".to_string()]);
    }

    #[test]
    fn extra_saved_values_are_ignored() {
        let params = vec![p("R", ParamKind::Int { default: 1, min: 0, max: 10 })];
        let out = reconcile(&["3".into(), "4".into(), "5".into()], &params);
        assert_eq!(out, vec!["3".to_string()]);
    }

    #[test]
    fn missing_saved_values_use_defaults() {
        let params = vec![
            p("A", ParamKind::Int { default: 1, min: 0, max: 10 }),
            p("B", ParamKind::Float { default: 0.5, min: 0.0, max: 1.0 }),
        ];
        let out = reconcile(&["7".into()], &params);
        assert_eq!(out, vec!["7".to_string(), "0.5".to_string()]);
    }

    #[test]
    fn type_mismatch_uses_default() {
        let params = vec![p("R", ParamKind::Int { default: 5, min: 0, max: 10 })];
        let out = reconcile(&["not-a-number".into()], &params);
        assert_eq!(out, vec!["5".to_string()]);
    }
}
```

Run: `cargo test --lib catalogue::reconcile`
Expected: PASS, 5 tests green.

- [ ] **Step 6: Exclude gmic-qt-only filters (spec §9 risk #4)**

Many catalogue entries are filters that only work via gmic-qt's IPC machinery (live preview, GIMP-specific UI, etc.) and fail when invoked headless via our subprocess. The convention: their command starts with `gmic_qt_`, `_gmic_qt_`, or a single leading underscore (the gmic-qt internal-namespace marker).

Add to `src/catalogue/parser.rs` immediately after the `consume_filter_header` function:

```rust
/// Filters whose primary command matches any of these patterns are
/// excluded from the catalogue — they require gmic-qt's IPC and fail
/// headlessly. Documented in plan §9 risk #4.
fn is_gmic_qt_only(command: &str) -> bool {
    command.starts_with("gmic_qt_")
        || command.starts_with("_gmic_qt_")
        || command.starts_with('_')
}
```

And change `consume_filter_header` to drop excluded filters before opening them:

```rust
fn consume_filter_header(&mut self, _line_no: usize, body: &str) -> Result<(), ParseError> {
    self.flush_filter();
    let (name_part, command_part) = body.split_once(':').unwrap();
    let display_name = name_part.trim().to_string();
    let command = command_part
        .split(',')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if is_gmic_qt_only(&command) {
        // Swallow the header and any param rows that follow until the
        // next folder/filter. We do that by leaving current_filter
        // *None* and treating subsequent ": rows" as orphan-but-OK
        // (see consume_param_row's guard below).
        self.skip_until_next_filter = true;
        return Ok(());
    }
    self.skip_until_next_filter = false;
    self.current_filter = Some(Filter {
        display_name,
        command,
        description: None,
        params: Vec::new(),
    });
    Ok(())
}
```

Add the `skip_until_next_filter: bool` field to `ParseState`, default `false`, and update `consume_param_row` to silently no-op when the flag is set rather than returning `OrphanParam`.

Add a unit test:

```rust
#[test]
fn gmic_qt_only_filters_are_excluded() {
    let src = "#@gui Cat\n\
               #@gui Visible : fx_visible\n\
               #@gui Hidden  : _gmic_qt_internal\n\
               #@gui Also Hidden : gmic_qt_dialog\n\
               #@gui : Density = int(5,0,10)\n\
               #@gui Visible2 : fx_visible2\n";
    let cat = parse(src).unwrap();
    let folder = match &cat.root.children[0] {
        Node::Folder(f) => f,
        _ => panic!(),
    };
    let commands: Vec<&str> = folder.children.iter().filter_map(|c| match c {
        Node::Filter(f) => Some(f.command.as_str()),
        _ => None,
    }).collect();
    assert_eq!(commands, vec!["fx_visible", "fx_visible2"]);
}
```

Run: `cargo test --lib catalogue::parser::tests::gmic_qt_only_filters_are_excluded`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/catalogue src/lib.rs
git commit -m "feat(catalogue): #@gui parser, IR, remembered_args reconciliation, gmic-qt exclusion"
```

---

## Task 3: `settings` module — JSON persistence

`[PARALLEL with T2, T4, T5]`

**Files:**
- Create: `src/settings.rs`
- Modify: `src/lib.rs` (one line: `pub mod settings;`)

---

- [ ] **Step 1: Add `pub mod settings;` to `src/lib.rs`**

Right next to `pub mod logging;`:

```rust
pub mod settings;
```

- [ ] **Step 2: Create `src/settings.rs` with the schema and atomic-write logic**

```rust
//! Persistence for last-picked filter, recents, and per-filter
//! remembered argument values. Stored as JSON at
//! `~/Library/Application Support/gmic-affinity/settings.json`.
//!
//! Atomic-write via tmp + rename. On parse failure the broken file is
//! renamed to `settings.json.broken-<ts>` and defaults are returned;
//! never aborts a picker session.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::logging::log;

const SCHEMA_VERSION: u32 = 1;
const RECENTS_CAP: usize = 10;
const REMEMBERED_CAP: usize = 256;
const FILE_BYTES_CAP: u64 = 256 * 1024;

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub last: Option<LastChoice>,
    #[serde(default)]
    pub recent: Vec<RecentEntry>,
    #[serde(default)]
    pub remembered_args: BTreeMap<String, Vec<String>>,
}

fn default_version() -> u32 { 0 }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LastChoice {
    pub command: String,
    pub args: Vec<String>,
    pub display_path: String,
    pub ts: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecentEntry {
    pub command: String,
    pub display_path: String,
    pub ts: String,
}

impl Settings {
    pub fn new() -> Self {
        Self {
            version: SCHEMA_VERSION,
            last: None,
            recent: Vec::new(),
            remembered_args: BTreeMap::new(),
        }
    }

    /// Read the settings file, returning defaults on absence or any
    /// parse / IO error. Renames a corrupted file aside so the user
    /// can recover it.
    pub fn load() -> Self {
        let path = match settings_path() {
            Some(p) => p,
            None => {
                log("settings: no HOME, using defaults");
                return Self::new();
            }
        };
        load_from(&path).unwrap_or_else(|| Self::new())
    }

    /// Atomic-write: serialize → settings.json.tmp → fsync → rename().
    pub fn save(&self) {
        let Some(path) = settings_path() else {
            log("settings: no HOME, save skipped");
            return;
        };
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log(&format!("settings: create_dir_all({parent:?}) failed: {e}"));
                return;
            }
        }
        let serialized = match serde_json::to_vec_pretty(self) {
            Ok(b) => b,
            Err(e) => {
                log(&format!("settings: serialize failed: {e}"));
                return;
            }
        };
        if (serialized.len() as u64) > FILE_BYTES_CAP {
            log(&format!(
                "settings: serialized {} bytes exceeds cap {FILE_BYTES_CAP}, skipping save",
                serialized.len(),
            ));
            return;
        }
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = write_atomic(&tmp, &path, &serialized) {
            log(&format!("settings: atomic save failed: {e}"));
        }
    }

    /// Record a user pick into `last`, push onto `recent` (dedup +
    /// cap), and update `remembered_args[command]` with the same args.
    pub fn record_pick(
        &mut self,
        command: &str,
        args: Vec<String>,
        display_path: &str,
        ts: &str,
    ) {
        self.version = SCHEMA_VERSION;
        self.last = Some(LastChoice {
            command: command.to_string(),
            args: args.clone(),
            display_path: display_path.to_string(),
            ts: ts.to_string(),
        });
        self.recent.retain(|r| r.command != command);
        self.recent.insert(
            0,
            RecentEntry {
                command: command.to_string(),
                display_path: display_path.to_string(),
                ts: ts.to_string(),
            },
        );
        if self.recent.len() > RECENTS_CAP {
            self.recent.truncate(RECENTS_CAP);
        }
        self.remembered_args.insert(command.to_string(), args);
        if self.remembered_args.len() > REMEMBERED_CAP {
            // BTreeMap has no LRU eviction; for v1 we shed the lexically
            // first entry that isn't the just-recorded one. Good enough
            // — REMEMBERED_CAP is 256 and won't be reached in practice.
            let evict = self
                .remembered_args
                .keys()
                .find(|k| k.as_str() != command)
                .cloned();
            if let Some(k) = evict {
                self.remembered_args.remove(&k);
            }
        }
    }

    /// Bump just `last.ts` for the Last-Filter case (where args don't change).
    pub fn touch_last(&mut self, ts: &str) {
        if let Some(last) = &mut self.last {
            last.ts = ts.to_string();
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library/Application Support/gmic-affinity/settings.json"),
    )
}

fn load_from(path: &Path) -> Option<Settings> {
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return None, // absence is normal
    };
    let mut buf = String::new();
    if let Err(e) = file.read_to_string(&mut buf) {
        log(&format!("settings: read failed: {e}; using defaults"));
        rename_broken(path);
        return None;
    }
    match serde_json::from_str::<Settings>(&buf) {
        Ok(mut s) => {
            if s.version > SCHEMA_VERSION {
                log(&format!(
                    "settings: file version {} > current {SCHEMA_VERSION}; using defaults without overwriting",
                    s.version,
                ));
                return None;
            }
            if s.version == 0 {
                s.version = SCHEMA_VERSION;
            }
            Some(s)
        }
        Err(e) => {
            log(&format!("settings: parse failed: {e}; renaming aside"));
            rename_broken(path);
            None
        }
    }
}

fn rename_broken(path: &Path) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let broken = path.with_extension(format!("json.broken-{ts}"));
    if let Err(e) = std::fs::rename(path, &broken) {
        log(&format!("settings: rename to {broken:?} failed: {e}"));
    }
}

fn write_atomic(tmp: &Path, final_: &Path, bytes: &[u8]) -> std::io::Result<()> {
    {
        let mut f = std::fs::File::create(tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(tmp, final_)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trip_empty() {
        let s = Settings::new();
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn record_pick_caps_recent_at_10() {
        let mut s = Settings::new();
        for i in 0..15 {
            s.record_pick(&format!("cmd{i}"), vec![], &format!("Path{i}"), "ts");
        }
        assert_eq!(s.recent.len(), 10);
        assert_eq!(s.recent[0].command, "cmd14"); // most recent first
    }

    #[test]
    fn record_pick_dedupes_recent() {
        let mut s = Settings::new();
        s.record_pick("a", vec![], "A", "1");
        s.record_pick("b", vec![], "B", "2");
        s.record_pick("a", vec![], "A", "3");
        assert_eq!(s.recent.len(), 2);
        assert_eq!(s.recent[0].command, "a");
        assert_eq!(s.recent[0].ts, "3");
    }

    #[test]
    fn record_pick_updates_remembered_args() {
        let mut s = Settings::new();
        s.record_pick("blur", vec!["3".into()], "X", "t");
        assert_eq!(s.remembered_args.get("blur").unwrap(), &vec!["3"]);
        s.record_pick("blur", vec!["7".into()], "X", "t2");
        assert_eq!(s.remembered_args.get("blur").unwrap(), &vec!["7"]);
    }

    #[test]
    fn atomic_write_and_read_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let tmp = path.with_extension("json.tmp");
        let mut s = Settings::new();
        s.record_pick("blur", vec!["3".into()], "Effects/Blur", "ts");
        let bytes = serde_json::to_vec_pretty(&s).unwrap();
        write_atomic(&tmp, &path, &bytes).unwrap();
        assert!(!tmp.exists());
        let back: Settings = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn broken_file_returns_defaults_and_renames() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{not json}").unwrap();
        let result = load_from(&path);
        assert!(result.is_none());
        let mut found_broken = false;
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let name = entry.unwrap().file_name();
            if name.to_string_lossy().contains("broken") {
                found_broken = true;
            }
        }
        assert!(found_broken, "expected a *.broken-* file in tempdir");
    }

    #[test]
    fn future_version_returns_defaults_without_clobbering() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"version":999}"#).unwrap();
        assert!(load_from(&path).is_none());
        // file should still be present unchanged
        assert!(path.exists());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"version":999}"#,
        );
    }

    #[test]
    fn missing_version_field_treats_as_v0_and_migrates() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"recent":[]}"#).unwrap();
        let s = load_from(&path).unwrap();
        assert_eq!(s.version, SCHEMA_VERSION);
    }
}
```

Run: `cargo test --lib settings`
Expected: PASS — 7 tests green.

- [ ] **Step 3: Commit**

```bash
git add src/settings.rs src/lib.rs
git commit -m "feat(settings): JSON persistence for last + recent + remembered_args"
```

---

## Task 4: `ps_data` — typed `*data` Box-leak helpers

`[PARALLEL with T2, T3, T5]`

**Files:**
- Create: `src/ps_data.rs`
- Modify: `src/lib.rs` (one line: `pub mod ps_data;`)

---

- [ ] **Step 1: Add `pub mod ps_data;` to `src/lib.rs`**

```rust
pub mod ps_data;
```

- [ ] **Step 2: Create `src/ps_data.rs`**

```rust
//! Typed helpers around the Photoshop plugin SDK's `*data` field — a
//! single `intptr_t *data` pointer the host preserves across selector
//! calls for plugin-private state.
//!
//! Lifetime contract:
//! - `leak(value, data)` puts a freshly Boxed value at the pointer.
//! - `borrow(data)` returns a `&T` view; the host still owns the slot.
//! - `take_and_drop(data)` reclaims ownership and runs `T`'s drop;
//!   it MUST be the only drop site (else double-free).
//!
//! Used by `PluginMain` to stash `ChosenFilter` from PARAMETERS, read
//! it back in CONTINUE, and free it in FINISH.

use std::ffi::c_void;

/// Move `value` to the heap and store its raw pointer in `*data`.
/// Any previous content in `*data` is dropped first via `take_and_drop`.
///
/// # Safety
/// `data` must be a valid `*mut isize` provided by the host. After
/// this call, `*data` is the raw pointer to a `Box<T>`.
pub unsafe fn leak<T>(value: T, data: *mut isize) {
    if data.is_null() {
        return;
    }
    take_and_drop::<T>(data);
    let boxed = Box::new(value);
    *data = Box::into_raw(boxed) as isize;
}

/// Return a shared reference to the `T` currently stashed at `*data`,
/// or `None` if the slot is null or `data` itself is null.
///
/// # Safety
/// The pointer at `*data` must have been produced by `leak::<T>` for
/// the same `T`.
pub unsafe fn borrow<'a, T>(data: *const isize) -> Option<&'a T> {
    if data.is_null() {
        return None;
    }
    let raw = *data;
    if raw == 0 {
        return None;
    }
    Some(&*(raw as *const T))
}

/// Reclaim `Box<T>` ownership from `*data` and drop it. Zeroes the slot.
///
/// # Safety
/// The pointer at `*data`, if non-null, must have been produced by
/// `leak::<T>` for the same `T`. After this call, `*data` is 0.
pub unsafe fn take_and_drop<T>(data: *mut isize) {
    if data.is_null() {
        return;
    }
    let raw = *data;
    if raw == 0 {
        return;
    }
    let _ = Box::from_raw(raw as *mut T);
    *data = 0;
}

/// Round-trip a `*data` through a C-call boundary helper for tests.
#[doc(hidden)]
pub fn _data_slot() -> *mut isize {
    Box::into_raw(Box::new(0_isize))
}

#[doc(hidden)]
pub unsafe fn _free_data_slot(p: *mut isize) {
    if !p.is_null() {
        let _ = Box::from_raw(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct Probe(u32);
    impl Drop for Probe {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn leak_and_borrow_round_trip() {
        unsafe {
            let slot = _data_slot();
            leak(Probe(42), slot);
            let view = borrow::<Probe>(slot).unwrap();
            assert_eq!(view.0, 42);
            take_and_drop::<Probe>(slot);
            assert!(borrow::<Probe>(slot).is_none());
            _free_data_slot(slot);
        }
    }

    #[test]
    fn borrow_null_data_returns_none() {
        unsafe {
            assert!(borrow::<Probe>(std::ptr::null::<isize>()).is_none());
        }
    }

    #[test]
    fn leak_replaces_existing_value_dropping_old_one() {
        DROP_COUNT.store(0, Ordering::SeqCst);
        unsafe {
            let slot = _data_slot();
            leak(Probe(1), slot);
            leak(Probe(2), slot);
            take_and_drop::<Probe>(slot);
            _free_data_slot(slot);
        }
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 2, "old + new must both be dropped");
    }

    #[test]
    fn take_and_drop_on_zero_is_noop() {
        unsafe {
            let slot = _data_slot();
            *slot = 0;
            take_and_drop::<Probe>(slot);
            _free_data_slot(slot);
        }
    }
}
```

- [ ] **Step 3: Run the tests under regular cargo**

```bash
cargo test --lib ps_data
```

Expected: PASS — 4 tests green.

- [ ] **Step 4: (Optional, recommended) Run under Miri to verify pointer lifetime**

```bash
rustup toolchain install nightly --component miri --component rust-src
cargo +nightly miri test --lib ps_data
```

Expected: PASS with no UB reported. If Miri isn't installed locally, skip — the regular test still catches the double-drop case.

- [ ] **Step 5: Commit**

```bash
git add src/ps_data.rs src/lib.rs
git commit -m "feat(ps_data): typed Box-leak helpers for PluginMain *data slot"
```

---

## Task 5: `ui::alert` — NSAlert wrapper with `Sink` trait

`[PARALLEL with T2, T3, T4]`

> **Coordination note:** this task creates `src/ui/alert.rs` and adds `pub mod alert;` to `src/ui/mod.rs`. T1 created `src/ui/mod.rs` already, with `pub mod runloop; pub mod picker;`. The agent for this task must append, not overwrite.

**Files:**
- Create: `src/ui/alert.rs`
- Modify: `src/ui/mod.rs` (add `pub mod alert;`)

---

- [ ] **Step 1: Add `pub mod alert;` to `src/ui/mod.rs`**

After the existing two `pub mod` lines:

```rust
pub mod alert;
```

- [ ] **Step 2: Create `src/ui/alert.rs` with the Sink trait + production + test impls**

```rust
//! User-facing error reporting via `NSAlert`.
//!
//! `Sink::display(title, message)` is the single point of contact
//! with AppKit for error UI. Tests substitute a capture `Sink` that
//! pushes the alert text into a `Mutex<Vec<…>>` so the error-handling
//! matrix in PluginMain can be table-tested without ever opening a
//! window.

use std::sync::Mutex;

pub trait Sink: Send + Sync {
    fn display(&self, title: &str, message: &str);
}

/// Append the standard log-hint footer to a message when appropriate.
fn with_log_hint(message: &str, log_hint: bool) -> String {
    if log_hint {
        format!(
            "{message}\n\nSee ~/Library/Logs/gmic-affinity.log for details.",
        )
    } else {
        message.to_string()
    }
}

/// Convenience: route an error through the configured sink with an
/// optional log-file pointer footer.
pub fn alert_error(sink: &dyn Sink, title: &str, message: &str, log_hint: bool) {
    sink.display(title, &with_log_hint(message, log_hint));
}

// ---- Production NSAlert implementation ----

pub struct NsAlertSink;

impl Sink for NsAlertSink {
    fn display(&self, title: &str, message: &str) {
        nsalert_runmodal(title, message);
    }
}

#[cfg(feature = "live")]
fn nsalert_runmodal(title: &str, message: &str) {
    use objc2_app_kit::{NSAlert, NSAlertStyle};
    use objc2_foundation::{MainThreadMarker, NSString};

    let Some(mtm) = MainThreadMarker::new() else {
        // Off the main thread: log instead of crashing. Should never
        // happen from PluginMain but worth defending.
        crate::logging::log(&format!(
            "alert: not on main thread; would have shown title={title:?}, message={message:?}",
        ));
        return;
    };
    unsafe {
        let alert: objc2::rc::Retained<NSAlert> = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(message));
        alert.setAlertStyle(NSAlertStyle::Warning);
        let _ = alert.runModal();
    }
}

#[cfg(not(feature = "live"))]
fn nsalert_runmodal(title: &str, message: &str) {
    // The default no-op build never opens AppKit; just log.
    crate::logging::log(&format!("alert (default build): title={title:?} message={message:?}"));
}

// ---- Test capture sink ----

#[derive(Default)]
pub struct CaptureSink {
    pub events: Mutex<Vec<(String, String)>>,
}

impl Sink for CaptureSink {
    fn display(&self, title: &str, message: &str) {
        self.events.lock().unwrap().push((title.to_string(), message.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_hint_appends_footer() {
        let s = with_log_hint("Oops.", true);
        assert!(s.contains("Oops."));
        assert!(s.contains("~/Library/Logs/gmic-affinity.log"));
    }

    #[test]
    fn no_log_hint_does_not_append_footer() {
        let s = with_log_hint("Quick note.", false);
        assert!(!s.contains("gmic-affinity.log"));
    }

    #[test]
    fn capture_sink_records_calls() {
        let sink = CaptureSink::default();
        alert_error(&sink, "T", "M", false);
        alert_error(&sink, "T2", "M2", true);
        let events = sink.events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, "T");
        assert_eq!(events[0].1, "M");
        assert!(events[1].1.contains("gmic-affinity.log"));
    }
}
```

> **objc2 note:** as in T1, the exact `Retained<…>` / `NSAlert::new(mtm)` syntax depends on the installed `objc2-app-kit` version. Adjust to the current macro/method spelling as `cargo build --features live` directs.

- [ ] **Step 3: Run the tests in both default and live builds**

```bash
cargo test --lib ui::alert
cargo test --features live --lib ui::alert
```

Expected: PASS, 3 tests green in each.

- [ ] **Step 4: Commit**

```bash
git add src/ui/mod.rs src/ui/alert.rs
git commit -m "feat(ui): NSAlert sink with Sink trait + capture impl for tests"
```

---

# Wave 3 — Sequential UI growth

## Task 6: LFS-tracked catalogue snapshot + smoke test + `dump-toc` binary

`[SEQUENTIAL]` — depends on T2 (parser must exist).

**Files:**
- Create: `assets/gmic-catalogue.gmic.gz` (Git LFS)
- Create: `assets/gmic-catalogue.toc.txt`
- Create: `assets/gmic-catalogue.version.txt`
- Create: `.gitattributes`
- Create: `src/bin/dump-toc.rs`
- Create: `tests/catalogue_snapshot.rs`
- Modify: `src/catalogue/mod.rs` (real `builtin()` body with `include_bytes!` + flate2)

---

- [ ] **Step 1: One-time per machine: `git lfs install`**

```bash
git lfs install
```

Expected: `Updated Git hooks. Git LFS initialized.`

- [ ] **Step 2: Refresh the gmic filter catalogue and place it under `assets/`**

```bash
mkdir -p assets
gmic update >/dev/null 2>&1
GMIC_VER=$(gmic --version 2>&1 | head -n1 | awk '{print $NF}')
UPDATE_FILE=$(ls -t ~/.config/gmic/update*.gmic | head -n1)
gzip -9 -c "$UPDATE_FILE" > assets/gmic-catalogue.gmic.gz
printf '%s\n%s\n' "$(gmic --version 2>&1 | head -n1)" "$(date -u +%FT%TZ)" > assets/gmic-catalogue.version.txt
ls -la assets/
```

Expected: `gmic-catalogue.gmic.gz` is in the 2–4 MB range. `version.txt` has two lines.

- [ ] **Step 3: Create `.gitattributes`**

```text
assets/gmic-catalogue.gmic.gz filter=lfs diff=lfs merge=lfs -text
```

- [ ] **Step 4: Replace `src/catalogue/mod.rs`'s `builtin()` body with the real one**

Replace the placeholder body of `builtin()` (both `cfg(test)` and `cfg(not(test))` branches) with this single implementation:

```rust
pub fn builtin() -> &'static Catalogue {
    BUILTIN.get_or_init(|| {
        use std::io::Read;
        const GZ: &[u8] = include_bytes!("../../assets/gmic-catalogue.gmic.gz");
        let mut text = String::new();
        flate2::read::GzDecoder::new(GZ)
            .read_to_string(&mut text)
            .expect("bundled gmic-catalogue.gmic.gz must decompress");
        parser::parse(&text).expect("bundled gmic-catalogue.gmic.gz must parse")
    })
}
```

Run: `cargo build`
Expected: clean compile. (`include_bytes!` resolves a real ~3 MB file now.)

- [ ] **Step 5: Create `src/bin/dump-toc.rs`**

```rust
//! Dump the parsed bundled catalogue as a stable one-line-per-filter TOC.
//! Used by `make refresh-catalogue` to keep `assets/gmic-catalogue.toc.txt`
//! in sync with what our parser actually understands of the snapshot.

use gmic_affinity::catalogue::{self, Folder, Node};

fn main() {
    let cat = catalogue::builtin();
    let mut lines: Vec<String> = Vec::new();
    walk(&cat.root, &mut Vec::new(), &mut lines);
    lines.sort();
    for line in lines {
        println!("{line}");
    }
}

fn walk(folder: &Folder, path: &mut Vec<String>, out: &mut Vec<String>) {
    for child in &folder.children {
        match child {
            Node::Folder(f) => {
                path.push(f.name.clone());
                walk(f, path, out);
                path.pop();
            }
            Node::Filter(f) => {
                let full_path = if path.is_empty() {
                    f.display_name.clone()
                } else {
                    format!("{} / {}", path.join(" / "), f.display_name)
                };
                out.push(format!("{full_path}  ->  {}", f.command));
            }
        }
    }
}
```

- [ ] **Step 6: Generate `assets/gmic-catalogue.toc.txt`**

```bash
cargo run --bin dump-toc > assets/gmic-catalogue.toc.txt
wc -l assets/gmic-catalogue.toc.txt
head -5 assets/gmic-catalogue.toc.txt
```

Expected: ≥400 lines, alphabetised, each looking like `Artistic / Paint Brush  ->  fx_paint_brush`.

- [ ] **Step 7: Create `tests/catalogue_snapshot.rs`**

```rust
//! Smoke test: bundled gmic-catalogue.gmic.gz decompresses, parses,
//! and contains a sensible number of filters + known anchors.

use gmic_affinity::catalogue::{self, Folder, Node};

#[test]
fn snapshot_decompresses_parses_and_has_minimum_content() {
    let cat = catalogue::builtin();
    let (folders, filters) = count(&cat.root);
    assert!(folders >= 15, "expected >=15 top-or-nested folders, got {folders}");
    assert!(filters >= 400, "expected >=400 filters, got {filters}");
}

#[test]
fn anchor_filters_present() {
    let cat = catalogue::builtin();
    let mut commands: Vec<&str> = Vec::new();
    collect_commands(&cat.root, &mut commands);
    for anchor in ["fx_paint_brush", "fx_light_glow"] {
        assert!(
            commands.contains(&anchor),
            "expected anchor command {anchor} in catalogue (sample: {:?})",
            &commands[..commands.len().min(5)],
        );
    }
}

fn count(folder: &Folder) -> (usize, usize) {
    let mut folders = 0;
    let mut filters = 0;
    for child in &folder.children {
        match child {
            Node::Folder(f) => {
                folders += 1;
                let (sf, sfi) = count(f);
                folders += sf;
                filters += sfi;
            }
            Node::Filter(_) => filters += 1,
        }
    }
    (folders, filters)
}

fn collect_commands<'a>(folder: &'a Folder, out: &mut Vec<&'a str>) {
    for child in &folder.children {
        match child {
            Node::Folder(f) => collect_commands(f, out),
            Node::Filter(f) => out.push(&f.command),
        }
    }
}
```

Run: `cargo test --test catalogue_snapshot`
Expected: PASS, 2 tests green. If a test fails because gmic shipped a new filter format we haven't taught the parser, look at the TOC diff and either add a new `ParamKind` case to the parser or add the unrecognised kind name to the parser's allow list.

- [ ] **Step 8: Commit (LFS will route the `.gz` automatically)**

```bash
git add .gitattributes assets/ src/bin src/catalogue/mod.rs tests/catalogue_snapshot.rs
git lfs status                          # sanity: gmic-catalogue.gmic.gz shown as LFS object
git commit -m "feat(catalogue): bundle gmic snapshot via LFS + smoke test"
```

---

## Task 7: Static one-folder-one-filter outline view

`[SEQUENTIAL]` — depends on T1. Proves the `objc2::declare_class!` data-source pattern works inside Affinity's run loop.

**Files:**
- Modify: `src/ui/picker.rs` (replace the empty-panel stub with a panel that hosts an `NSOutlineView` showing a hardcoded tree).

---

- [ ] **Step 1: Replace `show_empty` with a single-folder static outline view**

Rewrite `src/ui/picker.rs` so it builds the panel content view with an `NSScrollView` holding an `NSOutlineView`, backed by a hard-coded data source ("Artistic" folder containing one filter named "Paint Brush"). Keep the public function name `show_empty()` for now so T1's `lib.rs` wiring still compiles; T10 renames it to `show_picker`.

> **objc2 note:** declaring an `NSOutlineViewDataSource` from Rust requires `objc2::declare_class!` (or `define_class!` in newer versions). The macro syntax has moved between objc2 0.4 and 0.5. The structure your class needs:
>
> - One ivar: the static tree data (use `OnceLock<…>` or a static).
> - Four `#[method(...)]` selectors:
>   - `outlineView:numberOfChildrenOfItem:` → `NSInteger`
>   - `outlineView:child:ofItem:` → `id`
>   - `outlineView:isItemExpandable:` → `BOOL`
>   - `outlineView:objectValueForTableColumn:byItem:` → `id`
>
> See <https://github.com/madsmtm/objc2/tree/master/crates/objc2-app-kit/examples> for current
> outline-view examples. The code below sketches the structure;
> spellings of macro keys (`mutability`, `name`, `super`, etc.) may need
> to match the version on your toolchain.

```rust
//! Minimal static outline view for milestone T7.
//! - Hard-coded data: one folder "Artistic" containing one leaf "Paint Brush".
//! - Verifies that NSOutlineView via objc2 data-source declaration
//!   works inside Affinity's run loop.
//! - T8 replaces the hard-coded data with `catalogue::builtin()`.

#![cfg(feature = "live")]

use objc2::rc::Retained;
use objc2_app_kit::{
    NSBackingStoreType, NSOutlineView, NSPanel, NSScrollView,
    NSTableColumn, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{CGRect, CGPoint, CGSize, MainThreadMarker, NSString};

use crate::ui::runloop::run_modal_window;

/// (Stub name retained from T1; T10 renames to `show_picker`.)
pub fn show_empty() -> Option<()> {
    let mtm = MainThreadMarker::new()?;

    let frame = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize { width: 720.0, height: 520.0 },
    };
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Resizable;

    let panel: Retained<NSPanel> = unsafe {
        NSPanel::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            frame,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    let window: Retained<NSWindow> = panel.into_super();
    window.setTitle(&NSString::from_str("G'MIC (static tree)"));

    let outline_view = build_static_outline_view(mtm);
    let scroll = unsafe { NSScrollView::initWithFrame(mtm.alloc(), frame) };
    unsafe {
        scroll.setHasVerticalScroller(true);
        scroll.setDocumentView(Some(outline_view.as_super()));
    }
    if let Some(content) = window.contentView() {
        unsafe { content.addSubview(&scroll) };
    }

    let _ = run_modal_window(&window);
    Some(())
}

fn build_static_outline_view(mtm: MainThreadMarker) -> Retained<NSOutlineView> {
    let outline: Retained<NSOutlineView> = unsafe {
        NSOutlineView::initWithFrame(
            mtm.alloc(),
            CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size: CGSize { width: 720.0, height: 520.0 },
            },
        )
    };
    let column = unsafe {
        NSTableColumn::initWithIdentifier(mtm.alloc(), &NSString::from_str("name"))
    };
    unsafe {
        outline.addTableColumn(&column);
        outline.setOutlineTableColumn(Some(&column));
    }

    let data_source = data_source::StaticDataSource::new(mtm);
    unsafe {
        outline.setDataSource(Some(&data_source));
    }
    // Keep the data source alive for the life of the outline view. The
    // outline view's `setDataSource:` is a weak retain. We leak the
    // Retained here since the outline view itself is owned by the panel
    // which is dismissed at runModal-exit.
    std::mem::forget(data_source);
    outline
}

mod data_source {
    use objc2::rc::Retained;
    use objc2_foundation::MainThreadMarker;

    // The declare_class! / define_class! macro invocation lives here.
    // Structure of the class (pseudocode for current objc2 0.5 syntax):
    //
    // declare_class!(
    //     pub struct StaticDataSource;
    //     unsafe impl ClassType for StaticDataSource {
    //         type Super = NSObject;
    //         type Mutability = Mutable;
    //         const NAME: &'static str = "GmicAffinityStaticDataSource";
    //     }
    //
    //     unsafe impl StaticDataSource {
    //         #[method(outlineView:numberOfChildrenOfItem:)]
    //         fn number_of_children(&self, _view: &NSOutlineView, item: Option<&AnyObject>) -> NSInteger {
    //             if item.is_none() { 1 } else { 0 }  // root has one folder; folder has one leaf
    //         }
    //         #[method_id(outlineView:child:ofItem:)]
    //         fn child(&self, _view: &NSOutlineView, idx: NSInteger, item: Option<&AnyObject>) -> Retained<NSString> {
    //             if item.is_none() {
    //                 NSString::from_str("Artistic")
    //             } else {
    //                 NSString::from_str("Paint Brush")
    //             }
    //         }
    //         #[method(outlineView:isItemExpandable:)]
    //         fn is_expandable(&self, _view: &NSOutlineView, item: &AnyObject) -> bool {
    //             // "Artistic" is expandable; "Paint Brush" is not.
    //             // Compare by string identity if items are NSStrings; for real T8 we use indices.
    //             let s: &NSString = unsafe { msg_send![item, description] };
    //             s.to_string() == "Artistic"
    //         }
    //         #[method_id(outlineView:objectValueForTableColumn:byItem:)]
    //         fn value_for(&self, _view: &NSOutlineView, _col: &NSTableColumn, item: &AnyObject) -> Retained<NSString> {
    //             let s: &NSString = unsafe { msg_send![item, description] };
    //             s.copy()
    //         }
    //     }
    // );

    impl StaticDataSource {
        pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = mtm.alloc::<Self>().set_ivars(());
            unsafe { msg_send_id![super(this), init] }
        }
    }

    // Placeholder so the file compiles before the macro is filled in.
    // Replace with the declare_class! / define_class! invocation above
    // following the current objc2-app-kit example pattern.
    pub struct StaticDataSource;
    impl StaticDataSource { /* see comment above */ }
}
```

> **Implementer guidance:** the `data_source` module above is intentionally a *structural sketch* because `objc2`'s `declare_class!` / `define_class!` macro syntax changed between releases and pasting the wrong version into the plan is worse than guiding you to the right reference. Open the upstream examples mirror (`cargo doc --open --package objc2-app-kit`, examples link in the README) for a current outline-view example and adapt. The four selectors listed are the only ones AppKit will call for a single-column outline view in pull mode. Once the macro compiles, the rest of this task is plumbing.

- [ ] **Step 2: Build and reinstall**

```bash
make universal-install FEATURES=live
```

- [ ] **Step 3: Manual verification inside Affinity**

1. Restart Affinity, open an 8-bit RGB doc.
2. Filter → Plugins → G'MIC → G'MIC...
3. Panel shows an outline view with one folder ("Artistic") that expands to show one filter ("Paint Brush").
4. Close the panel; Affinity still responsive.

- [ ] **Step 4: Commit**

```bash
git add src/ui/picker.rs
git commit -m "feat(picker): static single-folder NSOutlineView (proves data source pattern)"
```

---

## Task 8: Wire outline view to real catalogue + search + Recent pseudo-folder

`[SEQUENTIAL]` — depends on T7 (outline view scaffold) and T6 (catalogue bundled).

**Files:**
- Modify: `src/ui/picker.rs` (data source now reads from `&'static Catalogue`; add `NSSearchField` above the outline view; add a virtual "Recent" pseudo-folder built from `settings.rs` recents at panel-open time).

---

- [ ] **Step 1: Define a `TreeNode` index abstraction in `src/ui/picker.rs`**

The data source needs stable item identifiers AppKit will hand back to us in `child:ofItem:` and `isItemExpandable:`. The simplest scheme: each item is a `Box<TreeNode>` we leak, AppKit retains the raw pointer, we drop everything on panel close.

```rust
#[derive(Debug, Clone, Copy)]
enum TreeNode {
    Recent,                            // virtual top-level folder
    RecentEntry(usize),                // index into Settings.recent
    Folder(*const crate::catalogue::Folder),
    Filter(*const crate::catalogue::Filter),
}
```

(Storing raw pointers into the `&'static Catalogue` is safe because the catalogue lives in a `OnceLock` and never moves.)

- [ ] **Step 2: Replace the static data source with one that walks `catalogue::builtin()`**

Same `declare_class!` pattern as T7. The four selectors now:

- `number_of_children`: `None` → `1 + cat.root.children.len()` (Recent + real top-level folders); for `Folder(p)` → `(*p).children.len()`; for `Recent` → `settings.last().recent.len()`; for leaves → `0`.
- `child:ofItem:` returns a freshly-leaked `Box<TreeNode>` raw pointer.
- `is_expandable`: `true` for `Recent` / `Folder(_)`; `false` for leaves and `RecentEntry`.
- `objectValueForTableColumn:byItem:` returns the display name as `NSString`.

> Add an explicit drop on panel-close that walks every leaked `TreeNode` and frees it; alternatively allocate all `TreeNode`s into a `typed_arena::Arena` owned by the data source and drop them en masse. The arena pattern keeps the macro body simpler.

- [ ] **Step 3: Add `NSSearchField` above the outline view**

```rust
// pseudocode
let search = unsafe { NSSearchField::initWithFrame(mtm.alloc(), search_frame) };
search.setRecentsAutosaveName(&NSString::from_str("GmicPickerSearch"));
content.addSubview(&search);
```

Hook its delegate so each keystroke triggers `outline_view.reloadData()` and the data source filters its visible-children list by a stored substring (set the substring via a regular Rust `RefCell<String>` ivar on the data source, set from the search field's text in the delegate's `controlTextDidChange:` callback).

For v1 the filter is "expand all folders whose name OR whose direct children's names contain the substring (case-insensitive); show only matching leaves and their ancestor folders". Empty substring restores the saved expansion state.

- [ ] **Step 4: Wire `setFrameAutosaveName` so window size + split position persist**

```rust
window.setFrameAutosaveName(&NSString::from_str("GmicPickerPanel"));
```

- [ ] **Step 5: Manual verification inside Affinity**

```bash
make universal-install FEATURES=live
```

Restart Affinity, open the picker. Verify:

1. The top-level entries include the real gmic categories (Artistic, Lights & Shadows, Repair, …) and a `Recent` pseudo-folder.
2. Expanding a category shows nested folders and filters from the catalogue.
3. Typing into the search field filters the tree as you type.
4. Resizing the panel and reopening it preserves the size.

- [ ] **Step 6: Commit**

```bash
git add src/ui/picker.rs
git commit -m "feat(picker): wire NSOutlineView to bundled catalogue + search + Recent"
```

---

## Task 9: Parameter form on the right pane

`[SEQUENTIAL]` — depends on T8.

**Files:**
- Modify: `src/ui/picker.rs` (`NSSplitView` split into tree pane (left) + form pane (right); rebuild form on `outlineViewSelectionDidChange:`; one control per `ParamKind`).

---

- [ ] **Step 1: Replace the single `NSScrollView` content with an `NSSplitView`**

Restructure `show_empty`'s layout:

```text
content
└── NSSearchField (top, fixed height)
└── NSSplitView (vertical, autoresizes)
    ├── NSScrollView (NSOutlineView)   ← left
    └── NSScrollView (NSStackView)     ← right (form)
```

Set 60/40 split initial widths.

- [ ] **Step 2: Add the selection-change handler that rebuilds the form**

In the outline-view delegate (separate from the data source class, also declared via `declare_class!`), implement `outlineViewSelectionDidChange:` which:

1. Reads the current selection's `TreeNode`.
2. If it's a `Filter(p)`, calls `rebuild_form(&*p)`.
3. Otherwise empties the form.

- [ ] **Step 3: Implement `rebuild_form` mapping `ParamKind` → control**

| `ParamKind` | Control                                                                                                                  |
|-------------|--------------------------------------------------------------------------------------------------------------------------|
| `Int`       | `NSSlider` (continuous, integer steps via `setNumberOfTickMarks`) + `NSTextField`; both bound to a shared cell.          |
| `Float`     | `NSSlider` (continuous) + `NSTextField`; both bound to a shared cell.                                                    |
| `Bool`      | `NSButton` with `setButtonType(NSButtonType::Switch)`.                                                                   |
| `Choice`    | `NSPopUpButton` populated from `choices`; `selectItemAtIndex(default)`.                                                  |
| `Color`     | `NSColorWell` initialised from `default_rgb`.                                                                            |
| `Text`      | `NSTextField`.                                                                                                           |
| `Note`      | `NSTextField` with `setEditable(false)`, `setBezeled(false)`, `setDrawsBackground(false)`, multi-line wrap.              |
| `Separator` | `NSBox` with `setBoxType(NSBoxType::Separator)`.                                                                          |
| `Link`      | `NSButton` with `setBordered(false)`, action opens `NSWorkspace::shared().openURL(url)`.                                  |
| `Unknown`   | `NSTextField` (read-only) showing the raw declaration.                                                                    |

Keep a `Vec<FormCell>` ivar on the form-controller class so we can later read every control's current value back into `Vec<String>` (next task).

- [ ] **Step 4: Manual verification**

```bash
make universal-install FEATURES=live
```

In Affinity, open the picker, click "Lights & Shadows / Light Glow". Confirm the right pane populates with controls labelled "Density" and "Mode" pre-filled with their gmic defaults. Switch to "Artistic / Paint Brush" and confirm the form rebuilds to match.

- [ ] **Step 5: Commit**

```bash
git add src/ui/picker.rs
git commit -m "feat(picker): dynamic parameter form pane (NSStackView)"
```

---

# Wave 4 — Sequential PluginMain integration

## Task 10: Public `ui::picker::show_picker` API + `Backend` trait

`[SEQUENTIAL]` — depends on T9.

**Files:**
- Modify: `src/ui/picker.rs` (rename `show_empty` to `show_picker`, return `Option<ChosenFilter>`, implement OK button collecting form values; add `Backend` trait + scripted impl for tests).

---

- [ ] **Step 1: Rename and re-type the public function**

```rust
pub fn show_picker(
    catalogue: &'static crate::catalogue::Catalogue,
    last_choice: Option<&crate::settings::LastChoice>,
) -> Option<crate::catalogue::ChosenFilter> {
    // existing panel construction, augmented with:
    //   - OK / Cancel buttons at the bottom
    //   - OK action: collect form values + return Some(ChosenFilter)
    //   - Cancel action: return None
    //   - On open, locate last_choice.command in the catalogue,
    //     select that row, scroll it into view, pre-fill the form
    //     with `reconcile(last_choice.args, &filter.params)`.
}
```

OK / Cancel buttons go in a bottom strip:

```text
└── NSStackView (horizontal, bottom strip)
    ├── NSButton "Reset defaults" (leading)
    ├── flexible spacer
    ├── NSButton "Cancel"          → NSApp.stopModal(.cancel)
    └── NSButton "OK"              → collect form + stopModal(.OK)
```

`NSApp.runModalForWindow(&window)` returns the response code; map `.OK` → `Some(collected_chosen_filter)`, `.cancel` → `None`.

- [ ] **Step 2: Update `src/lib.rs` so `SELECTOR_PARAMETERS` calls `show_picker` with the bundled catalogue and loaded settings**

```rust
SELECTOR_PARAMETERS => {
    let settings = crate::settings::Settings::load();
    let cat = crate::catalogue::builtin();
    match crate::ui::picker::show_picker(cat, settings.last.as_ref()) {
        Some(chosen) => {
            log(&format!("PARAMETERS: user picked {}", chosen.command));
            // T11 leaks `chosen` into *data; for now we drop it.
            NO_ERR
        }
        None => USER_CANCEL,
    }
}
```

- [ ] **Step 3: Add `Backend` trait + scripted test impl**

```rust
pub trait Backend {
    fn run(
        &self,
        catalogue: &crate::catalogue::Catalogue,
        last_choice: Option<&crate::settings::LastChoice>,
    ) -> Option<crate::catalogue::ChosenFilter>;
}

pub struct CocoaBackend;
impl Backend for CocoaBackend {
    fn run(
        &self,
        catalogue: &crate::catalogue::Catalogue,
        last_choice: Option<&crate::settings::LastChoice>,
    ) -> Option<crate::catalogue::ChosenFilter> {
        show_picker_inner(catalogue, last_choice)
    }
}

#[cfg(test)]
pub struct ScriptedBackend {
    pub response: Option<crate::catalogue::ChosenFilter>,
}
#[cfg(test)]
impl Backend for ScriptedBackend {
    fn run(
        &self,
        _catalogue: &crate::catalogue::Catalogue,
        _last_choice: Option<&crate::settings::LastChoice>,
    ) -> Option<crate::catalogue::ChosenFilter> {
        self.response.clone()
    }
}
```

`show_picker_inner` is the real Cocoa code from Steps 1–2. The exposed function `show_picker` calls `CocoaBackend.run(...)` in production; tests substitute `ScriptedBackend`.

- [ ] **Step 4: Wire keyboard / button affordances per design §4.4**

In the panel construction:

- `window.setDefaultButtonCell(ok_button.cell())` — Return triggers OK.
- `window.setInitialFirstResponder(&search_field)` — typing immediately filters.
- Override the panel's `cancelOperation:` (or set up an `NSEvent` local monitor for `Escape`) to invoke the Cancel handler.
- On the outline view, set `setDoubleAction(sel!(okClicked:))` and `setTarget(self.button_controller)` so double-clicking a leaf invokes OK. Guard the OK handler with "selection is a `TreeNode::Filter`" — if the user double-clicked a folder, treat it as expand/collapse (default behaviour, no action).
- Implement OK-enable-on-leaf:
  - Hook the outline view's selection-change delegate (same one that drives `rebuild_form` from T9).
  - On every selection change set `ok_button.setEnabled(matches!(current_node, Some(TreeNode::Filter(_))))`.

Add a "Reset defaults" button handler:

```rust
fn on_reset_defaults(&self) {
    if let Some(TreeNode::Filter(p)) = self.current_selection() {
        let filter = unsafe { &*p };
        // Force the form to use defaults, not remembered values.
        self.rebuild_form(filter, /* use_remembered = */ false);
    }
}
```

(`rebuild_form` from T9 gains a `use_remembered: bool` parameter; the normal path passes `true`.)

- [ ] **Step 5: Build, install, manually verify the wiring**

```bash
make universal-install FEATURES=live
```

In Affinity, open the picker. Verify:

1. Press Esc → panel closes, plugin returns USER_CANCEL.
2. Pick a folder → OK is disabled (greyed).
3. Click a leaf → OK is enabled. Press Return → OK fires.
4. Double-click a leaf → OK fires.
5. Click "Reset defaults" → form repopulates with gmic stdlib values regardless of what was remembered.

- [ ] **Step 6: Commit**

```bash
git add src/ui/picker.rs src/lib.rs
git commit -m "feat(picker): show_picker returns ChosenFilter + Backend trait + keyboard wiring"
```

---

## Task 11: PluginMain PARAMETERS — leak `ChosenFilter` into `*data` + update settings

`[SEQUENTIAL]` — depends on T10, T3 (settings), T4 (ps_data).

**Files:**
- Modify: `src/lib.rs` (PARAMETERS arm)

---

- [ ] **Step 1: Replace the temporary `NO_ERR` arm with full handling**

```rust
SELECTOR_PARAMETERS => {
    use crate::catalogue::ChosenFilter;

    let mut settings = crate::settings::Settings::load();
    let cat = crate::catalogue::builtin();
    let chosen: Option<ChosenFilter> =
        crate::ui::picker::show_picker(cat, settings.last.as_ref());

    match chosen {
        Some(chosen) => {
            let ts = chrono_like_now_iso();
            let display_path = lookup_display_path(cat, &chosen.command)
                .unwrap_or_else(|| chosen.command.clone());
            settings.record_pick(&chosen.command, chosen.args.clone(), &display_path, &ts);
            settings.save();
            unsafe { crate::ps_data::leak::<ChosenFilter>(chosen, _data) };
            log("PARAMETERS: ChosenFilter leaked into *data, settings saved");
            NO_ERR
        }
        None => USER_CANCEL,
    }
}
```

`_data` is the `*mut isize` parameter PluginMain already receives. `chrono_like_now_iso()` is a small helper appended to `src/logging.rs` (its `iso8601_now` already exists for log lines — re-export or call it).

`lookup_display_path(cat, command)` is a tiny helper in `src/catalogue/mod.rs`:

```rust
pub fn lookup_display_path(cat: &Catalogue, command: &str) -> Option<String> {
    fn walk(folder: &Folder, command: &str, path: &mut Vec<String>) -> Option<String> {
        for child in &folder.children {
            match child {
                Node::Folder(f) => {
                    path.push(f.name.clone());
                    if let Some(found) = walk(f, command, path) {
                        return Some(found);
                    }
                    path.pop();
                }
                Node::Filter(f) if f.command == command => {
                    let full = if path.is_empty() {
                        f.display_name.clone()
                    } else {
                        format!("{} / {}", path.join(" / "), f.display_name)
                    };
                    return Some(full);
                }
                Node::Filter(_) => {}
            }
        }
        None
    }
    walk(&cat.root, command, &mut Vec::new())
}
```

Add a small unit test for `lookup_display_path` (folder/filter walking).

- [ ] **Step 2: Build, install, manually verify**

```bash
make universal-install FEATURES=live
```

In Affinity: pick a filter, click OK. Inspect `~/Library/Application Support/gmic-affinity/settings.json` — it must contain `last`, one entry in `recent`, one entry in `remembered_args`.

- [ ] **Step 3: Commit**

```bash
git add src/lib.rs src/catalogue/mod.rs
git commit -m "feat(plugin): PARAMETERS persists pick + leaks ChosenFilter into *data"
```

---

## Task 12: PluginMain CONTINUE — recover `ChosenFilter`, call new `run_filter_with`, Last-Filter fallback

`[SEQUENTIAL]` — depends on T11, T3, T4. Also adds the new `gmic.rs` entry point.

**Files:**
- Modify: `src/gmic.rs` (add `run_filter_with`)
- Modify: `src/lib.rs` (CONTINUE arm)

---

- [ ] **Step 1: Add `pub fn run_filter_with` to `src/gmic.rs`**

Locate the existing `pub fn run_filter(fr: &mut FilterRecord) -> Result<…>`. Add alongside it:

```rust
/// Like `run_filter` but takes a `ChosenFilter` directly (from the
/// picker) instead of reading filter.txt. Goes through the same
/// MAX_FILTER_ARGS / MAX_ARG_BYTES / NUL & control-char checks as the
/// existing file-based path; those move from "validate parsed file"
/// to "validate dialog output".
pub fn run_filter_with(
    fr: &mut FilterRecord,
    chosen: &crate::catalogue::ChosenFilter,
) -> Result<(), GmicError> {
    if chosen.args.len() > MAX_FILTER_ARGS - 4 {
        return Err(GmicError::TooManyArgs(chosen.args.len()));
    }
    for arg in &chosen.args {
        if arg.len() > MAX_ARG_BYTES {
            return Err(GmicError::ArgTooLong(arg.len()));
        }
        if arg.bytes().any(|b| b == 0 || b.is_ascii_control()) {
            return Err(GmicError::InvalidCharsInConfig);
        }
    }
    let mut tokens: Vec<String> = Vec::with_capacity(chosen.args.len() + 1);
    tokens.push(chosen.command.clone());
    tokens.extend(chosen.args.iter().cloned());

    // Reuse the existing run_with_tokens path. If today's gmic.rs
    // doesn't expose one, extract the body of `run_filter` after its
    // own file-load step into a private `run_with_tokens(fr, tokens)`
    // helper and call it from both `run_filter` and `run_filter_with`.
    run_with_tokens(fr, &tokens)
}
```

Add unit tests in `src/gmic.rs`:

```rust
#[cfg(test)]
mod tests_chosen {
    use super::*;
    use crate::catalogue::ChosenFilter;

    #[test]
    fn run_filter_with_rejects_too_many_args() {
        let chosen = ChosenFilter {
            command: "fx".into(),
            args: (0..MAX_FILTER_ARGS).map(|i| i.to_string()).collect(),
        };
        let mut fr = unsafe { std::mem::zeroed() };
        let err = run_filter_with(&mut fr, &chosen).err();
        assert!(matches!(err, Some(GmicError::TooManyArgs(_))));
    }

    #[test]
    fn run_filter_with_rejects_oversized_arg() {
        let chosen = ChosenFilter {
            command: "fx".into(),
            args: vec!["x".repeat(MAX_ARG_BYTES + 1)],
        };
        let mut fr = unsafe { std::mem::zeroed() };
        let err = run_filter_with(&mut fr, &chosen).err();
        assert!(matches!(err, Some(GmicError::ArgTooLong(_))));
    }

    #[test]
    fn run_filter_with_rejects_nul_byte() {
        let chosen = ChosenFilter {
            command: "fx".into(),
            args: vec!["bad\0value".into()],
        };
        let mut fr = unsafe { std::mem::zeroed() };
        let err = run_filter_with(&mut fr, &chosen).err();
        assert!(matches!(err, Some(GmicError::InvalidCharsInConfig)));
    }
}
```

Run: `cargo test --lib gmic::tests_chosen`
Expected: PASS, 3 tests green.

- [ ] **Step 2: Replace the CONTINUE arm in `src/lib.rs`**

```rust
SELECTOR_CONTINUE => {
    use crate::catalogue::ChosenFilter;

    // First try plugin-private *data (set by PARAMETERS).
    let chosen_owned: Option<ChosenFilter> = unsafe {
        crate::ps_data::borrow::<ChosenFilter>(_data).cloned()
    };

    // Last-Filter case: no PARAMETERS was called this invocation,
    // *data is null. Fall back to settings.last.
    let chosen = match chosen_owned {
        Some(c) => c,
        None => {
            let settings = crate::settings::Settings::load();
            match settings.last {
                Some(last) => ChosenFilter { command: last.command, args: last.args },
                None => {
                    // No last-choice persisted either — explain via NSAlert.
                    crate::ui::alert::alert_error(
                        &crate::ui::alert::NsAlertSink,
                        "G'MIC",
                        "No previous G'MIC filter to repeat. \
                         Pick one from Filters > Plugins > G'MIC > G'MIC….",
                        false,
                    );
                    log("CONTINUE: no *data, no settings.last — USER_CANCEL");
                    return USER_CANCEL;
                }
            }
        }
    };

    log(&format!("CONTINUE: running {} with {} args", chosen.command, chosen.args.len()));
    match crate::gmic::run_filter_with(fr, &chosen) {
        Ok(()) => NO_ERR,
        Err(e) => {
            // T14 wires NSAlert for each error variant; for now log + cancel.
            log(&format!("CONTINUE: gmic failed: {e}"));
            USER_CANCEL
        }
    }
}
```

- [ ] **Step 3: Handle dimension-mismatch in `tiff_io::read_tiff` (spec §9 risk #5)**

Many gmic filters change image dimensions (crop, rotate, scale, multi-frame output). Today's pipeline assumes `output_dims == input_dims`. For v1, when gmic's output TIFF has different dimensions than the host's `FilterRecord` rect, we resize back to the input dims with nearest-neighbour and log a one-line warning. v2 will use Photoshop's `imageSize` selector to negotiate the actual new size with Affinity.

Locate `pub fn read_tiff(...)` in `src/tiff_io.rs`. After the existing decode but before copying into `out_buf`, insert:

```rust
let (gmic_width, gmic_height) = decoder.dimensions()?;
if (gmic_width, gmic_height) != (expected_width, expected_height) {
    crate::logging::log(&format!(
        "tiff: gmic returned {}x{}, expected {}x{}; resampling nearest-neighbour",
        gmic_width, gmic_height, expected_width, expected_height,
    ));
    pixels = resample_nearest_to_u8(
        &pixels,
        gmic_width, gmic_height,
        expected_width, expected_height,
        planes,
    );
}
```

Then add the `resample_nearest_to_u8` helper at the bottom of the file:

```rust
fn resample_nearest_to_u8(
    src: &[u8],
    src_w: u32, src_h: u32,
    dst_w: u32, dst_h: u32,
    planes: u32,
) -> Vec<u8> {
    let pl = planes as usize;
    let src_w_us = src_w as usize;
    let dst_w_us = dst_w as usize;
    let dst_h_us = dst_h as usize;
    let mut out = vec![0u8; dst_w_us * dst_h_us * pl];
    for y in 0..dst_h_us {
        // floor(y * src_h / dst_h)
        let sy = (y as u64 * src_h as u64 / dst_h as u64) as usize;
        for x in 0..dst_w_us {
            let sx = (x as u64 * src_w as u64 / dst_w as u64) as usize;
            let src_idx = (sy * src_w_us + sx) * pl;
            let dst_idx = (y * dst_w_us + x) * pl;
            out[dst_idx..dst_idx + pl]
                .copy_from_slice(&src[src_idx..src_idx + pl]);
        }
    }
    out
}

#[cfg(test)]
mod resample_tests {
    use super::*;

    #[test]
    fn identity_passthrough() {
        let src = vec![1u8, 2, 3, 4];
        let out = resample_nearest_to_u8(&src, 2, 2, 2, 2, 1);
        assert_eq!(out, src);
    }

    #[test]
    fn shrink_by_half() {
        // 4x1 → 2x1, single channel, expected nearest picks indices 0,2
        let src = vec![10, 20, 30, 40];
        let out = resample_nearest_to_u8(&src, 4, 1, 2, 1, 1);
        assert_eq!(out, vec![10, 30]);
    }

    #[test]
    fn upsample_doubles() {
        let src = vec![5, 9];
        let out = resample_nearest_to_u8(&src, 2, 1, 4, 1, 1);
        assert_eq!(out, vec![5, 5, 9, 9]);
    }

    #[test]
    fn rgb_planes_are_preserved() {
        // 2x1 rgb, shrink to 1x1, expected first pixel
        let src = vec![1, 2, 3, 4, 5, 6];
        let out = resample_nearest_to_u8(&src, 2, 1, 1, 1, 3);
        assert_eq!(out, vec![1, 2, 3]);
    }
}
```

> **Implementer note:** the exact `decoder.dimensions()` call shape may differ in the `tiff` crate; today's code already reads them inside `read_tiff` to size buffers, so the value is in-scope under a slightly different name. Wire from the existing local.

Run: `cargo test --lib tiff_io::resample_tests`
Expected: PASS, 4 tests green.

- [ ] **Step 4: Build, install, manually verify Last-Filter behaviour**

```bash
make universal-install FEATURES=live
```

In Affinity:

1. Pick a filter (PARAMETERS path); apply. Image transforms.
2. Filter → Last Filter (or `Cmd-F`). The same filter re-runs without the dialog.
3. Delete `~/Library/Application Support/gmic-affinity/settings.json` and click Last Filter — NSAlert "No previous G'MIC filter to repeat" appears.
4. (Optional, only if a known size-changing gmic filter is reachable through the picker) — pick e.g. `Deformations / Spread` and confirm the result lands as a same-size image with no crash. Check the log for the `resampling nearest-neighbour` line.

- [ ] **Step 5: Commit**

```bash
git add src/gmic.rs src/tiff_io.rs src/lib.rs
git commit -m "feat(plugin): CONTINUE recovers ChosenFilter; Last-Filter + dim-mismatch fallback"
```

---

## Task 13: PluginMain FINISH — drop the Boxed `ChosenFilter`

`[SEQUENTIAL]` — depends on T11.

**Files:**
- Modify: `src/lib.rs` (FINISH arm)

---

- [ ] **Step 1: Replace FINISH arm**

```rust
SELECTOR_FINISH => {
    use crate::catalogue::ChosenFilter;
    unsafe { crate::ps_data::take_and_drop::<ChosenFilter>(_data) };
    log("FINISH: *data reclaimed");
    NO_ERR
}
```

- [ ] **Step 2: Build, install, run two filters back-to-back in Affinity**

```bash
make universal-install FEATURES=live
```

In Affinity: pick a filter, apply. Pick a different filter, apply. Both should succeed. Inspect `~/Library/Logs/gmic-affinity.log`: two `FINISH: *data reclaimed` entries; no double-free / segfault traces from the kernel.

- [ ] **Step 3: Commit**

```bash
git add src/lib.rs
git commit -m "feat(plugin): FINISH drops the Boxed ChosenFilter"
```

---

## Task 14: NSAlert wiring across the full error matrix

`[SEQUENTIAL]` — depends on T12, T13, T5.

**Files:**
- Modify: `src/lib.rs` (PARAMETERS error paths + CONTINUE gmic-failure paths)

---

- [ ] **Step 1: Replace PARAMETERS error path so AppKit failures alert**

When `show_picker` itself panics or returns `None` due to an internal error (not user cancellation), we want an alert. Wrap the call in `std::panic::catch_unwind`:

```rust
let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
    crate::ui::picker::show_picker(cat, settings.last.as_ref())
}));
let chosen = match result {
    Ok(opt) => opt,                             // None == user cancelled
    Err(_) => {
        log("PARAMETERS: picker panicked");
        crate::ui::alert::alert_error(
            &crate::ui::alert::NsAlertSink,
            "G'MIC",
            "Couldn't open the G'MIC dialog.",
            true,
        );
        return USER_CANCEL;
    }
};
```

Catalogue parse failures already panic on first access via `OnceLock` init (per `catalogue::builtin()`); wrap the first `let cat = ...` in the same `catch_unwind` and alert "G'MIC filter list is unreadable — this build of the plugin may be corrupted." on `Err`.

- [ ] **Step 2: Replace CONTINUE error path with the matrix from the design**

```rust
match crate::gmic::run_filter_with(fr, &chosen) {
    Ok(()) => NO_ERR,
    Err(e) => {
        let sink = &crate::ui::alert::NsAlertSink;
        match &e {
            crate::gmic::GmicError::NotFound => {
                crate::ui::alert::alert_error(sink, "G'MIC",
                    "G'MIC isn't installed. Install it with: brew install gmic-qt and try again.",
                    false);
            }
            crate::gmic::GmicError::Failed { status } => {
                crate::ui::alert::alert_error(sink, "G'MIC",
                    &format!("G'MIC reported an error running `{}` (exit {}).",
                             chosen.command,
                             status.map(|s| s.to_string()).unwrap_or_else(|| "signal".into())),
                    true);
            }
            crate::gmic::GmicError::Tiff(_) => {
                crate::ui::alert::alert_error(sink, "G'MIC",
                    "G'MIC produced an image we couldn't read back.",
                    true);
            }
            _ => {
                crate::ui::alert::alert_error(sink, "G'MIC",
                    &format!("G'MIC pipeline failed: {e}"),
                    true);
            }
        }
        log(&format!("CONTINUE: gmic failed: {e}"));
        USER_CANCEL
    }
}
```

- [ ] **Step 3: Add an integration test that uses `CaptureSink` and `ScriptedBackend`**

`tests/error_matrix.rs`:

```rust
//! Drive the error-handling matrix from the design's §5.3 without
//! involving AppKit. We can't call into PluginMain directly without
//! a real FilterRecord, but the alert messages are produced by the
//! same `alert_error` helper used in lib.rs, so we test those
//! formatting paths here.

use gmic_affinity::ui::alert::{alert_error, CaptureSink};

#[test]
fn gmic_not_found_message() {
    let sink = CaptureSink::default();
    alert_error(&sink, "G'MIC",
        "G'MIC isn't installed. Install it with: brew install gmic-qt and try again.",
        false);
    let evts = sink.events.lock().unwrap();
    assert_eq!(evts.len(), 1);
    assert!(evts[0].1.contains("brew install gmic-qt"));
    assert!(!evts[0].1.contains("gmic-affinity.log"));
}

#[test]
fn tiff_error_includes_log_hint() {
    let sink = CaptureSink::default();
    alert_error(&sink, "G'MIC",
        "G'MIC produced an image we couldn't read back.",
        true);
    let evts = sink.events.lock().unwrap();
    assert!(evts[0].1.contains("gmic-affinity.log"));
}

#[test]
fn last_filter_empty_message() {
    let sink = CaptureSink::default();
    alert_error(&sink, "G'MIC",
        "No previous G'MIC filter to repeat. \
         Pick one from Filters > Plugins > G'MIC > G'MIC….",
        false);
    let evts = sink.events.lock().unwrap();
    assert!(evts[0].1.contains("Pick one"));
}
```

Run: `cargo test --test error_matrix`
Expected: PASS, 3 tests green.

- [ ] **Step 4: Manual verification**

```bash
make universal-install FEATURES=live
```

In Affinity, exercise each error path:

| Path                 | How to trigger                                                                |
|----------------------|-------------------------------------------------------------------------------|
| gmic missing         | `sudo mv $(which gmic) /tmp/gmic.bak`; pick a filter; expect NSAlert. Restore. |
| gmic non-zero exit   | Use the picker to send a syntactically-invalid arg (or temporarily rename a gmic stdlib filter to force failure). |
| TIFF read failure    | Choose a filter that gmic processes successfully but exports unusually (rare in practice; verified during MVP). |
| Last-Filter empty    | Delete `settings.json`, press Cmd-F.                                          |

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs tests/error_matrix.rs
git commit -m "feat(plugin): NSAlert error matrix wired into PARAMETERS + CONTINUE"
```

---

# Wave 5 — Polish (parallel within wave)

After T14 lands, the picker is functionally complete. These five tasks touch disjoint files and can be parallel-dispatched.

---

## Task 15: `examples/picker.rs` for standalone UI iteration

`[PARALLEL]` within Wave 5.

**Files:**
- Create: `examples/picker.rs`

---

- [ ] **Step 1: Add the example**

```rust
//! Open the picker outside Affinity for fast UI iteration.
//! `cargo run --example picker --features live`
//! Prints the chosen filter to stdout.

#![cfg(feature = "live")]

fn main() {
    gmic_affinity::logging::log("example/picker: starting");
    let cat = gmic_affinity::catalogue::builtin();
    let settings = gmic_affinity::settings::Settings::load();
    match gmic_affinity::ui::picker::show_picker(cat, settings.last.as_ref()) {
        Some(chosen) => {
            println!("OK");
            println!("command: {}", chosen.command);
            for arg in &chosen.args {
                println!("arg:     {arg}");
            }
        }
        None => println!("CANCEL"),
    }
}
```

Add `required-features = ["live"]` if cargo refuses to build the example otherwise; in that case put it under `[[example]]` in `Cargo.toml`:

```toml
[[example]]
name             = "picker"
required-features = ["live"]
```

Run: `cargo run --example picker --features live`
Expected: panel opens; pick a filter; stdout shows command + args; Cancel prints `CANCEL`.

- [ ] **Step 2: Commit**

```bash
git add examples/picker.rs Cargo.toml
git commit -m "feat(examples): standalone picker for UI dev iteration"
```

---

## Task 16: Makefile additions — `refresh-catalogue`, `picker-example`, LFS fail-fast

`[PARALLEL]` within Wave 5.

**Files:**
- Modify: `Makefile`

---

- [ ] **Step 1: Add the LFS fail-fast guard to `bundle` recipe**

Find the `bundle:` rule in the Makefile. Immediately before the `cargo build` invocation, prepend:

```makefile
	@head -c 2 assets/gmic-catalogue.gmic.gz 2>/dev/null | od -An -tx1 | tr -d ' \n' | \
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

Repeat the same guard at the top of the `universal:` recipe so a fresh CI clone that somehow bypassed LFS fails loudly there too.

- [ ] **Step 2: Add `refresh-catalogue` target**

```makefile
.PHONY: refresh-catalogue
refresh-catalogue:
	git lfs install
	gmic update >/dev/null 2>&1
	@UPDATE_FILE=$$(ls -t $$HOME/.config/gmic/update*.gmic 2>/dev/null | head -n1); \
	 if [ -z "$$UPDATE_FILE" ]; then \
	   echo "ERROR: no update*.gmic in ~/.config/gmic — is gmic installed?"; exit 1; \
	 fi; \
	 gzip -9 -c "$$UPDATE_FILE" > assets/gmic-catalogue.gmic.gz
	cargo run --bin dump-toc > assets/gmic-catalogue.toc.txt
	printf '%s\n%s\n' \
	  "$$(gmic --version 2>&1 | head -n1)" \
	  "$$(date -u +%FT%TZ)" \
	  > assets/gmic-catalogue.version.txt
	cargo test --test catalogue_snapshot
	@echo ""; echo "refresh-catalogue: changes in assets/:"; \
	 git status -- assets/
```

- [ ] **Step 3: Add `picker-example` target**

```makefile
.PHONY: picker-example
picker-example:
	cargo run --example picker --features live
```

- [ ] **Step 4: Verify**

```bash
make picker-example     # opens dialog
make refresh-catalogue  # regenerates the snapshot from local gmic
```

- [ ] **Step 5: Commit**

```bash
git add Makefile
git commit -m "build(make): refresh-catalogue, picker-example, LFS fail-fast"
```

---

## Task 17: GitHub Actions CI — pull LFS objects on checkout

`[PARALLEL]` within Wave 5.

**Files:**
- Modify: `.github/workflows/ci.yml`

---

- [ ] **Step 1: Update the checkout step**

Find `uses: actions/checkout@v4` in `.github/workflows/ci.yml`. Add `with: { lfs: true }`:

```yaml
      - uses: actions/checkout@v4
        with:
          lfs: true
```

If the workflow has multiple jobs that each check out, update all of them.

- [ ] **Step 2: Push to a branch + open a PR + observe CI**

A real CI run is the only way to verify. The fail-fast guard from T16 will trip if LFS isn't pulled, so the build will scream loudly on misconfiguration.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: enable Git LFS for checkout so gmic-catalogue.gmic.gz is available"
```

---

## Task 18: Documentation refresh

`[PARALLEL]` within Wave 5.

**Files:**
- Modify: `README.md`
- Modify: `PRD.md`
- Modify: `IMPLEMENTATION_NOTES.md`

---

- [ ] **Step 1: README — add LFS onboarding line + dialog to "What it does"**

Append to the "Requirements" section:

```text
- Git LFS (one-time): `git lfs install` per machine, `git lfs pull` per fresh clone.
```

In the "What it does" section, replace the "1–4" steps with the new flow (picker first, then the existing pixel pipeline).

- [ ] **Step 2: PRD — flip "Parameter UI" status row**

In §6 (Shipped vs Deferred Status table), change:

```text
| Parameter UI (`filterSelectorParameters`)         | ⛔ v2                   |
```

to:

```text
| Parameter UI (catalogue picker + dynamic form)    | ✅ shipped (v2 — see docs/design/2026-05-17-gmic-picker-dialog.md) |
```

In §3 (Non-Goals), strike-through the "graphical parameter UI" bullet with a note pointing at the design.

- [ ] **Step 3: IMPLEMENTATION_NOTES — add §9 "The catalogue picker"**

A new section after the existing §8 "Recommended project structure". Cover:

- Why the legacy `filter.txt` mechanism was replaced.
- The five-module split (catalogue / settings / ps_data / ui::picker / ui::alert).
- Pointer to the design doc.
- The one paragraph each on:
  - LFS gotcha + fail-fast (cross-reference §2 PiPL pattern).
  - `examples/picker` for dev iteration.
  - Run-loop modal pattern.

Cap at ~300 words; the design doc is the source of truth.

- [ ] **Step 4: Commit**

```bash
git add README.md PRD.md IMPLEMENTATION_NOTES.md
git commit -m "docs: refresh PRD + README + IMPLEMENTATION_NOTES for v2 picker"
```

---

## Task 19: Manual end-to-end checklist

`[PARALLEL]` within Wave 5.

**Files:**
- Modify: `IMPLEMENTATION_NOTES.md` (append the checklist as a new section)

---

- [ ] **Step 1: Append to `IMPLEMENTATION_NOTES.md`**

```markdown
## 10. Manual end-to-end pre-release checklist (picker)

Run these by hand before each release of the picker. None of them are
worth automating against Affinity itself.

1. [ ] `cargo run --example picker --features live` — opens the dialog
   standalone. Verify the tree, search field, parameter form, OK,
   Cancel, double-click leaf, Esc, Return all behave per design §4.

2. [ ] `make universal-install FEATURES=live` — installs the universal
   bundle. Verify `make verify-bundle` passes (filetype 8 on both
   slices).

3. [ ] Open Affinity Photo 2, fresh launch, open an 8-bit RGB doc.
   Filter → Plugins → G'MIC → G'MIC…  Pick `Artistic / Paint Brush`
   with defaults. OK. Image transforms visibly. No crash.

4. [ ] Filter → Last Filter (`Cmd-F`). The same filter re-runs without
   the dialog. No crash.

5. [ ] Quit Affinity. Relaunch. Open the dialog. Paint Brush is
   pre-selected and its sliders show the values from step 3 (not gmic
   stdlib defaults).

6. [ ] Force errors:
   - [ ] `sudo mv $(which gmic) /tmp/`; pick a filter → NSAlert
     "G'MIC isn't installed…"; restore the binary.
   - [ ] Corrupt `~/Library/Application Support/gmic-affinity/settings.json`
     → next picker open still works; the broken file is renamed to
     `.broken-<ts>` and a fresh one is written.
   - [ ] Delete `settings.json` and press `Cmd-F` → NSAlert
     "No previous G'MIC filter to repeat…".

7. [ ] `~/Library/Logs/gmic-affinity.log` shows one structured line
   per interesting event across the run; no panics; no stray prints.
```

- [ ] **Step 2: Commit**

```bash
git add IMPLEMENTATION_NOTES.md
git commit -m "docs: manual e2e pre-release checklist for the picker"
```

---

# Self-review checklist (for the plan author / coordinator)

After all 19 tasks land, the coordinator runs through this once:

- [ ] `cargo test` green.
- [ ] `cargo test --features live` green.
- [ ] `cargo clippy --all-targets --features live -- -D warnings` green.
- [ ] `make verify-bundle` green.
- [ ] Manual e2e checklist (T19) all ticked.
- [ ] No `TODO` / `TBD` left in code (`rg -n 'TODO|TBD|FIXME' src/`).
- [ ] LFS object resolves: `git lfs ls-files` shows `assets/gmic-catalogue.gmic.gz`.
- [ ] CI green on GitHub.

---

*End of implementation plan.*
