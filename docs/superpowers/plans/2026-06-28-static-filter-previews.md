# Static Filter Previews Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show a pre-computed preview image for each renderable G'MIC filter in the picker, generated at build time against one bundled sample image, recomputed only when a filter's inputs change.

**Architecture:** A non-live shared module (`src/previews/`) provides pure helpers — filesystem-safe key derivation, the default-argv builder that mirrors what the picker sends to gmic, float formatting, and the manifest/up-to-date logic. A build-time binary (`src/bin/gen-previews.rs`) walks the catalogue, runs each filter's default command through a reusable headless gmic run path extracted from `src/gmic.rs`, and writes PNGs + `previews/manifest.json`. The Makefile copies PNGs into the bundle's `Contents/Resources/previews/`. The live picker UI (`src/ui/picker.rs` + `src/ui/picker_form.rs`) gains a third split-view pane that loads the matching PNG from the running bundle's Resources at selection time, falling back to a placeholder.

**Tech Stack:** Rust 2021, `image` crate (PNG encode), `sha2` (hashing), `serde`/`serde_json` (manifest), `objc2`/`objc2-app-kit` (AppKit UI), `gmic` CLI, GNU Make.

## Global Constraints

- Library crate name is `GmicFilter` (PascalCase); `#![allow(non_snake_case)]` is set crate-wide.
- The shared `previews` module and the `gen-previews` binary must compile in the **default** (non-`live`) build — they must not depend on any `#[cfg(feature = "live")]` code or AppKit.
- UI changes live under `#[cfg(feature = "live")]`, matching the rest of `picker.rs` / `picker_form.rs`.
- The preview argv MUST match what the picker actually sends to gmic via `FormController::collect_values` (`src/ui/picker_form.rs:441`): `Int`→clamped integer string, `Float`→`format_float`, `Bool`→`"1"`/`"0"`, `Choice`→selected **index** string, `Color`→`"r,g,b"` bytes, `Text`→raw, `Note`/`Separator`/`Link`/`Unknown`→**no argv entry**, `Internal`→`default` verbatim.
- Subprocess execution reuses the hardened pattern in `src/gmic.rs`: `env_clear()` + minimal `PATH`/`HOME`/`TMPDIR`/`LANG`, `Stdio::piped()`, 60s timeout (`GMIC_TIMEOUT_SECS`), per-call 0700 tempdir via `tempfile`.
- gmic binary is located via `gmic::locate_gmic()` (`/opt/homebrew/bin/gmic` then `/usr/local/bin/gmic`).
- Bundle identifier: `com.dstrupl.gmic-affinity`; executable at `Contents/MacOS/GmicFilter`; previews at `Contents/Resources/previews/`.
- Run `cargo fmt` before each commit; `make clippy` runs `cargo clippy --all-targets --all-features -- -D warnings`.
- Source image is Git-LFS tracked like `assets/gmic-catalogue.gmic.gz`.

---

## File Structure

- **Create** `src/previews/mod.rs` — shared, non-live module. Re-exports submodules and holds `sanitise_key`, `format_float`, `default_argv`.
- **Create** `src/previews/manifest.rs` — manifest types (`Manifest`, `Entry`, `EntryStatus`), `input_hash`, `decide`, JSON load/save.
- **Create** `src/bin/gen-previews.rs` — build-time generator binary.
- **Modify** `src/lib.rs` — add `pub mod previews;`.
- **Modify** `src/gmic.rs` — extract a reusable headless run path (`render_with_tokens`) that runs gmic on an input image file and writes an output file, with no `FilterRecord`.
- **Modify** `src/ui/picker_form.rs` — make `collect_values` delegate float formatting to `previews::format_float`; add preview-pane update on selection change.
- **Modify** `src/ui/picker.rs` — convert the 2-pane split into a 3-pane split (tree | form | preview); add the preview view + split delegate for min widths; widen default window.
- **Create** `src/ui/preview_pane.rs` — live-only: builds the preview `NSImageView` + description `NSTextField`, resolves the bundle previews dir, loads/clears the image.
- **Modify** `Cargo.toml` — add `image`, `sha2` deps; declare the `gen-previews` bin.
- **Modify** `Makefile` — add `previews` target; copy PNGs into the bundle in `bundle`/`universal`.
- **Create** `assets/preview-source.tiff` (LFS) + `assets/preview-source.LICENSE.txt`.
- **Modify** `.gitattributes` — track the source TIFF via LFS.
- **Create/Generated** `previews/manifest.json` + `previews/*.png` — committed generator output.

---

## Task 1: Add dependencies and the `previews` module skeleton

**Files:**
- Modify: `Cargo.toml`
- Create: `src/previews/mod.rs`
- Modify: `src/lib.rs:22-30` (module declarations)

**Interfaces:**
- Produces: `pub mod previews;` reachable from the crate root in the default build.

- [ ] **Step 1: Add dependencies to `Cargo.toml`**

In the `[dependencies]` section (after `flate2`), add:

```toml
image    = { version = "0.25", default-features = false, features = ["png", "tiff"] }
sha2     = "0.10"
```

- [ ] **Step 2: Declare the bin target in `Cargo.toml`**

After the existing `[[example]]` block add:

```toml
[[bin]]
name = "gen-previews"
path = "src/bin/gen-previews.rs"
```

- [ ] **Step 3: Create the module file**

Create `src/previews/mod.rs`:

```rust
//! Build-time preview-generation support, shared between the
//! `gen-previews` binary and the live picker UI.
//!
//! Everything here compiles in the default (non-`live`) build: it is
//! pure Rust with no AppKit dependency. The binary uses the whole
//! module; the UI uses only [`sanitise_key`] to re-derive a filter's
//! preview filename without parsing the manifest at runtime.

pub mod manifest;

use crate::catalogue::{Param, ParamKind};

// sanitise_key, format_float, default_argv are added in later tasks.
```

- [ ] **Step 4: Declare the module in `src/lib.rs`**

After `pub mod ps_types;` (line 27) add:

```rust
pub mod previews;
```

- [ ] **Step 5: Create a stub manifest module so the crate compiles**

Create `src/previews/manifest.rs`:

```rust
//! Preview manifest: per-filter up-to-date tracking. Filled in by a
//! later task.
```

- [ ] **Step 6: Verify it builds**

Run: `cargo build`
Expected: PASS (compiles; `image`/`sha2`/`gen-previews` resolve — the empty bin path will fail only once we reference it, so this step builds the lib only). If the missing `src/bin/gen-previews.rs` breaks the build, create it now with a placeholder `fn main() {}`.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add Cargo.toml Cargo.lock src/previews/mod.rs src/previews/manifest.rs src/lib.rs
git commit -m "feat(previews): add deps and previews module skeleton"
```

---

## Task 2: `sanitise_key` — filesystem-safe preview filenames

**Files:**
- Modify: `src/previews/mod.rs`

**Interfaces:**
- Produces: `pub fn sanitise_key(command: &str) -> String` — maps a gmic command to a stable `[A-Za-z0-9_-]` token, appending a short hash suffix so two different commands never collide on the same key.

- [ ] **Step 1: Write the failing tests**

Append to `src/previews/mod.rs`:

```rust
#[cfg(test)]
mod sanitise_tests {
    use super::sanitise_key;

    #[test]
    fn keeps_safe_chars() {
        // Plain fx names survive except for the disambiguating suffix.
        let k = sanitise_key("fx_oldphoto");
        assert!(k.starts_with("fx_oldphoto-"));
        assert!(k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    }

    #[test]
    fn replaces_unsafe_chars() {
        let k = sanitise_key("foo/bar baz.qux");
        // No path separators, spaces, or dots in the safe portion.
        let safe = k.rsplit_once('-').unwrap().0;
        assert!(safe.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }

    #[test]
    fn distinct_commands_get_distinct_keys() {
        // Same sanitised prefix but different originals must differ.
        assert_ne!(sanitise_key("a/b"), sanitise_key("a_b"));
    }

    #[test]
    fn stable_for_same_input() {
        assert_eq!(sanitise_key("fx_painting"), sanitise_key("fx_painting"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib previews::sanitise_tests`
Expected: FAIL — `cannot find function sanitise_key`.

- [ ] **Step 3: Implement `sanitise_key`**

Add to `src/previews/mod.rs` (above the test module):

```rust
use sha2::{Digest, Sha256};

/// Map a gmic command to a stable, filesystem-safe filename stem.
///
/// The visible portion keeps `[A-Za-z0-9_]` from the command (other
/// bytes become `_`) so files are recognisable; a short hex suffix of
/// the *original* command guarantees two distinct commands never map
/// to the same key even when their sanitised prefixes collide.
pub fn sanitise_key(command: &str) -> String {
    let safe: String = command
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    let digest = Sha256::digest(command.as_bytes());
    let suffix = &hex8(&digest);
    format!("{safe}-{suffix}")
}

fn hex8(bytes: &[u8]) -> String {
    bytes.iter().take(4).map(|b| format!("{b:02x}")).collect()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib previews::sanitise_tests`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/previews/mod.rs
git commit -m "feat(previews): add sanitise_key for preview filenames"
```

---

## Task 3: `format_float` + `default_argv` — mirror the picker's argv

**Files:**
- Modify: `src/previews/mod.rs`
- Modify: `src/ui/picker_form.rs:1307-1316` (delegate to the shared `format_float`)

**Interfaces:**
- Produces: `pub fn format_float(v: f64) -> String`.
- Produces: `pub fn default_argv(params: &[Param]) -> Vec<String>` — the argv the picker would send to gmic if the user clicked OK without touching any control. Mirrors `FormController::collect_values` exactly (see Global Constraints).

- [ ] **Step 1: Write the failing tests**

Append to `src/previews/mod.rs`:

```rust
#[cfg(test)]
mod argv_tests {
    use super::{default_argv, format_float};
    use crate::catalogue::{Param, ParamKind};

    fn p(kind: ParamKind) -> Param {
        Param { label: "x".into(), kind }
    }

    #[test]
    fn format_float_trims_zeros() {
        assert_eq!(format_float(0.5), "0.5");
        assert_eq!(format_float(2.0), "2");
        assert_eq!(format_float(1.2500), "1.25");
    }

    #[test]
    fn int_is_clamped_default() {
        let out = default_argv(&[p(ParamKind::Int { default: 5, min: 0, max: 10 })]);
        assert_eq!(out, vec!["5"]);
    }

    #[test]
    fn bool_is_one_or_zero() {
        let out = default_argv(&[
            p(ParamKind::Bool { default: true }),
            p(ParamKind::Bool { default: false }),
        ]);
        assert_eq!(out, vec!["1", "0"]);
    }

    #[test]
    fn choice_is_index_not_text() {
        let out = default_argv(&[p(ParamKind::Choice {
            choices: vec!["A".into(), "B".into(), "C".into()],
            default: 2,
        })]);
        assert_eq!(out, vec!["2"]);
    }

    #[test]
    fn color_is_rgb_bytes() {
        let out = default_argv(&[p(ParamKind::Color { default_rgb: [10, 20, 30] })]);
        assert_eq!(out, vec!["10,20,30"]);
    }

    #[test]
    fn presentation_params_contribute_nothing() {
        let out = default_argv(&[
            p(ParamKind::Note("hi".into())),
            p(ParamKind::Separator),
            p(ParamKind::Link { label: "L".into(), url: "u".into() }),
            p(ParamKind::Unknown("point(1,2)".into())),
        ]);
        assert!(out.is_empty());
    }

    #[test]
    fn internal_contributes_default_verbatim() {
        let out = default_argv(&[p(ParamKind::Internal {
            label: "hidden".into(),
            default: "0".into(),
        })]);
        assert_eq!(out, vec!["0"]);
    }

    #[test]
    fn text_is_verbatim() {
        let out = default_argv(&[p(ParamKind::Text { default: "hello world".into() })]);
        assert_eq!(out, vec!["hello world"]);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib previews::argv_tests`
Expected: FAIL — `cannot find function default_argv` / `format_float`.

- [ ] **Step 3: Implement `format_float` and `default_argv`**

Add to `src/previews/mod.rs`:

```rust
/// Format a float the way the picker's slider readback does: integers
/// print without a decimal point, fractions keep up to 4 places with
/// trailing zeros trimmed. Kept identical to the picker so previews
/// match the argv a real OK click would send.
pub fn format_float(v: f64) -> String {
    if (v.round() - v).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Build the argv the picker would send to gmic for `params` with
/// every control left at its default. This MUST stay in lock-step with
/// `FormController::collect_values` in `src/ui/picker_form.rs`: gmic
/// receives a `Choice` as its selected *index*, a `Bool` as `"1"`/`"0"`,
/// and presentation-only params (`Note`/`Separator`/`Link`/`Unknown`)
/// contribute no argv entry at all.
pub fn default_argv(params: &[Param]) -> Vec<String> {
    params
        .iter()
        .filter_map(|param| match &param.kind {
            ParamKind::Int { default, min, max } => Some(default.clamp(*min, *max).to_string()),
            ParamKind::Float { default, min, max } => {
                Some(format_float(default.clamp(*min, *max)))
            }
            ParamKind::Bool { default } => Some(if *default { "1" } else { "0" }.to_string()),
            ParamKind::Choice { default, .. } => Some(default.to_string()),
            ParamKind::Color { default_rgb } => {
                Some(format!("{},{},{}", default_rgb[0], default_rgb[1], default_rgb[2]))
            }
            ParamKind::Text { default } => Some(default.clone()),
            ParamKind::Internal { default, .. } => Some(default.clone()),
            ParamKind::Note(_)
            | ParamKind::Separator
            | ParamKind::Link { .. }
            | ParamKind::Unknown(_) => None,
        })
        .collect()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib previews::argv_tests`
Expected: PASS (8 tests).

- [ ] **Step 5: DRY up `picker_form.rs` to share `format_float`**

In `src/ui/picker_form.rs`, delete the private `fn format_float(...)` at lines 1307-1316 and replace its single use inside `collect_values` (the `FormCell::Float` arm, ~line 452) so it calls the shared one. At the top of the file, add to the `use crate::...` imports:

```rust
use crate::previews::format_float;
```

In the `FormCell::Float` arm of `collect_values`, the call site stays `format_float(v)` — now resolving to the imported function.

- [ ] **Step 6: Verify the live build still compiles and tests pass**

Run: `cargo build --features live && cargo test --features live --lib previews`
Expected: PASS (no `format_float` redefinition; previews tests green).

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add src/previews/mod.rs src/ui/picker_form.rs
git commit -m "feat(previews): add default_argv mirroring picker collect_values"
```

---

## Task 4: Manifest types, `input_hash`, and `decide`

**Files:**
- Modify: `src/previews/manifest.rs`

**Interfaces:**
- Produces:
  - `pub struct Manifest { pub source_hash: String, pub gmic_version: String, pub entries: BTreeMap<String, Entry> }` (serde, `Default`).
  - `pub struct Entry { pub input_hash: String, pub status: EntryStatus, pub file: Option<String>, pub reason: Option<String> }`.
  - `pub enum EntryStatus { Ok, Skip }` (serde lowercase).
  - `pub fn input_hash(source_hash: &str, gmic_version: &str, command: &str, args: &[String]) -> String`.
  - `pub enum Action { Recompute, Keep }`.
  - `pub fn decide(entry: Option<&Entry>, computed_hash: &str, file_exists: bool) -> Action`.
  - `pub fn load(path: &Path) -> Manifest` (missing/corrupt → `Manifest::default()`); `pub fn save(path: &Path, m: &Manifest) -> std::io::Result<()>`.

- [ ] **Step 1: Write the failing tests**

Replace `src/previews/manifest.rs` contents with the doc comment plus:

```rust
//! Preview manifest: per-filter up-to-date tracking shared by the
//! generator. Pure logic — no gmic, no filesystem beyond load/save.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub source_hash: String,
    #[serde(default)]
    pub gmic_version: String,
    #[serde(default)]
    pub entries: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub input_hash: String,
    pub status: EntryStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryStatus {
    Ok,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Recompute,
    Keep,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_entry(hash: &str) -> Entry {
        Entry { input_hash: hash.into(), status: EntryStatus::Ok, file: Some("f.png".into()), reason: None }
    }

    #[test]
    fn hash_changes_with_each_input() {
        let base = input_hash("s", "v", "cmd", &["1".into()]);
        assert_ne!(base, input_hash("s2", "v", "cmd", &["1".into()]));
        assert_ne!(base, input_hash("s", "v2", "cmd", &["1".into()]));
        assert_ne!(base, input_hash("s", "v", "cmd2", &["1".into()]));
        assert_ne!(base, input_hash("s", "v", "cmd", &["2".into()]));
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(
            input_hash("s", "v", "cmd", &["1".into(), "2".into()]),
            input_hash("s", "v", "cmd", &["1".into(), "2".into()]),
        );
    }

    #[test]
    fn missing_entry_recomputes() {
        assert_eq!(decide(None, "h", false), Action::Recompute);
    }

    #[test]
    fn hash_drift_recomputes() {
        let e = ok_entry("old");
        assert_eq!(decide(Some(&e), "new", true), Action::Recompute);
    }

    #[test]
    fn ok_but_file_gone_recomputes() {
        let e = ok_entry("h");
        assert_eq!(decide(Some(&e), "h", false), Action::Recompute);
    }

    #[test]
    fn fresh_ok_is_kept() {
        let e = ok_entry("h");
        assert_eq!(decide(Some(&e), "h", true), Action::Keep);
    }

    #[test]
    fn fresh_skip_is_kept() {
        let e = Entry { input_hash: "h".into(), status: EntryStatus::Skip, file: None, reason: Some("boom".into()) };
        // Skip entries have no file; matching hash keeps them.
        assert_eq!(decide(Some(&e), "h", false), Action::Keep);
    }

    #[test]
    fn load_missing_returns_default() {
        let m = load(Path::new("/no/such/manifest.json"));
        assert!(m.entries.is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib previews::manifest`
Expected: FAIL — `input_hash` / `decide` / `load` not found.

- [ ] **Step 3: Implement the functions**

Append to `src/previews/manifest.rs`:

```rust
/// Hash exactly the inputs that change a rendered preview. A change in
/// any of them flips the hash and triggers a recompute; catalogue
/// folder/display-name moves are deliberately NOT included because they
/// don't alter the rendered pixels.
pub fn input_hash(source_hash: &str, gmic_version: &str, command: &str, args: &[String]) -> String {
    let mut h = Sha256::new();
    // Length-prefix each field so concatenation can't alias across
    // boundaries (e.g. "ab"+"c" vs "a"+"bc").
    for field in [source_hash, gmic_version, command] {
        h.update((field.len() as u64).to_le_bytes());
        h.update(field.as_bytes());
    }
    h.update((args.len() as u64).to_le_bytes());
    for a in args {
        h.update((a.len() as u64).to_le_bytes());
        h.update(a.as_bytes());
    }
    format!("sha256:{:x}", h.finalize())
}

/// Decide whether a filter's preview must be regenerated.
pub fn decide(entry: Option<&Entry>, computed_hash: &str, file_exists: bool) -> Action {
    match entry {
        None => Action::Recompute,
        Some(e) if e.input_hash != computed_hash => Action::Recompute,
        // A recorded success whose PNG vanished must be re-rendered.
        Some(e) if e.status == EntryStatus::Ok && !file_exists => Action::Recompute,
        Some(_) => Action::Keep,
    }
}

/// Load a manifest, treating a missing or unparseable file as empty so
/// the next build simply regenerates everything.
pub fn load(path: &Path) -> Manifest {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Manifest::default(),
    }
}

/// Write the manifest as pretty JSON (stable key order via `BTreeMap`).
pub fn save(path: &Path, m: &Manifest) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(m).expect("manifest serialises");
    std::fs::write(path, json)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib previews::manifest`
Expected: PASS (8 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/previews/manifest.rs
git commit -m "feat(previews): add manifest types, input_hash, and decide"
```

---

## Task 5: Reusable headless gmic run path in `gmic.rs`

**Files:**
- Modify: `src/gmic.rs`

**Interfaces:**
- Consumes: existing `locate_gmic`, `run_subprocess`, `run_with_tokens`, `quote_gmic_arg`, `MAX_FILTER_ARGS`, `MAX_ARG_BYTES`, `GmicError`.
- Produces:
  - `pub fn filter_tokens(command: &str, args: &[String]) -> Vec<String>` — the exact `[command, comma-joined-quoted-args]` token vector the picker sends, extracted from `run_filter_with` so the generator and the in-plugin path share one source of truth.
  - `pub fn render_with_tokens(gmic: &Path, input: &Path, output: &Path, tokens: &[String], output_planes: u32, tmpdir: &Path) -> Result<(), GmicError>` — builds the argv `[input, tokens…, -to_{gray,rgb,rgba}, -output, output]` and runs gmic, with no `FilterRecord` dependency. Reuses the existing `build_argv_from_tokens` (made `pub(crate)` if needed) and `run_subprocess`.

**Why `filter_tokens` matters:** `run_filter_with` (`src/gmic.rs:343`) prefixes the command with `-` when it lacks one, then comma-joins ALL args into a **single** token via `quote_gmic_arg` — it does NOT push one argv entry per arg. A naive `[command, arg, arg, …]` would make gmic treat each parameter as a separate top-level command and break filters that use `${1-N}` substitution. The generator must reuse this exact construction.

This task adds an argv-shape unit test, a `filter_tokens` unit test, and an `#[ignore]` end-to-end test that runs only where gmic is installed.

- [ ] **Step 1: Write the failing tests**

In the `#[cfg(test)] mod tests` block at the bottom of `src/gmic.rs`, add:

```rust
#[test]
fn filter_tokens_prefixes_and_joins() {
    // Bare command gets a leading '-'; all args become ONE comma-joined token.
    let t = filter_tokens("fx_oldphoto", &["1".into(), "2".into()]);
    assert_eq!(t, vec!["-fx_oldphoto".to_string(), "1,2".to_string()]);
    // Already-prefixed command is left alone; no args => no second token.
    let t2 = filter_tokens("-blur", &[]);
    assert_eq!(t2, vec!["-blur".to_string()]);
    // A value containing a comma is quoted so it stays one parameter.
    let t3 = filter_tokens("fx_x", &["5,5".into(), "3".into()]);
    assert_eq!(t3, vec!["-fx_x".to_string(), "\"5,5\",3".to_string()]);
}

#[test]
fn render_tokens_argv_shape() {
    let in_p = PathBuf::from("/tmp/a/in.tif");
    let out_p = PathBuf::from("/tmp/a/out.png");
    let tokens = filter_tokens("fx_oldphoto", &["1".into()]);
    let argv = build_argv_from_tokens(&in_p, &out_p, &tokens, 3).unwrap();
    let strs: Vec<&str> = argv.iter().map(|s| s.to_str().unwrap()).collect();
    assert_eq!(
        strs,
        vec!["/tmp/a/in.tif", "-fx_oldphoto", "1", "-to_rgb", "-output", "/tmp/a/out.png"],
    );
}

#[test]
#[ignore = "requires gmic installed"]
fn render_with_tokens_produces_output() {
    use std::io::Write;
    let gmic = crate::gmic::locate_gmic().expect("gmic installed for this test");
    let dir = tempfile::tempdir().unwrap();
    // Synthesise a tiny input image with gmic itself so we don't need a fixture.
    let input = dir.path().join("in.tif");
    let seed_argv: Vec<std::ffi::OsString> = vec![
        "64,64,1,3".into(),
        "-to_rgb".into(),
        "-output".into(),
        input.clone().into_os_string(),
    ];
    // `64,64,1,3` tells gmic to allocate a 64x64x1x3 image.
    crate::gmic::run_subprocess(&gmic, &seed_argv, dir.path()).unwrap();
    let _ = &mut std::io::stderr().flush();

    let output = dir.path().join("out.png");
    let tokens = crate::gmic::filter_tokens("-blur", &["2".into()]);
    crate::gmic::render_with_tokens(&gmic, &input, &output, &tokens, 3, dir.path()).unwrap();
    assert!(output.exists());
    assert!(std::fs::metadata(&output).unwrap().len() > 0);
}
```

- [ ] **Step 2: Run the non-ignored tests to verify they fail**

Run: `cargo test --lib gmic::tests::filter_tokens_prefixes_and_joins gmic::tests::render_tokens_argv_shape`
Expected: FAIL — `filter_tokens` missing; `build_argv_from_tokens` may be private.

- [ ] **Step 3: Extract `filter_tokens`, expose `build_argv_from_tokens`, add `render_with_tokens`**

In `src/gmic.rs`:

1. Extract the command-prefix + comma-join logic from `run_filter_with` into a reusable function:

```rust
/// Build the `[command, comma-joined-quoted-args]` token vector that a
/// single gmic filter invocation expects. The command gets a leading
/// `-` if it doesn't already have one; all args become ONE token,
/// comma-joined with each value run through [`quote_gmic_arg`]. This is
/// the single source of truth shared by `run_filter_with` (in-plugin)
/// and the preview generator.
pub fn filter_tokens(command: &str, args: &[String]) -> Vec<String> {
    let command = if command.starts_with('-') {
        command.to_string()
    } else {
        format!("-{command}")
    };
    let mut tokens = Vec::with_capacity(2);
    tokens.push(command);
    if !args.is_empty() {
        let joined = args.iter().map(|a| quote_gmic_arg(a)).collect::<Vec<_>>().join(",");
        tokens.push(joined);
    }
    tokens
}
```

2. Replace the inline command/join block inside `run_filter_with` (the `let command = if chosen.command.starts_with('-') … ` through the `tokens.push(joined);` block, ~lines 371-386) with:

```rust
    let tokens = filter_tokens(&chosen.command, &chosen.args);
    run_with_tokens(fr, &tokens)
```

3. Change `fn build_argv_from_tokens` to `pub(crate) fn build_argv_from_tokens`. Add the headless render path near `run_with_tokens`:

```rust
/// Headless render path used by the build-time preview generator.
///
/// Unlike [`run_filter_with`], this takes file paths directly and never
/// touches a `FilterRecord`: load `input`, apply `tokens`, force the
/// colour model from `output_planes`, and write `output`. The argv caps
/// and subprocess hardening are identical to the in-plugin path.
pub fn render_with_tokens(
    gmic: &Path,
    input: &Path,
    output: &Path,
    tokens: &[String],
    output_planes: u32,
    tmpdir: &Path,
) -> Result<(), GmicError> {
    let argv = build_argv_from_tokens(input, output, tokens, output_planes)?;
    run_subprocess(gmic, &argv, tmpdir)
}
```

- [ ] **Step 4: Run the non-ignored tests to verify they pass**

Run: `cargo test --lib gmic::tests::filter_tokens_prefixes_and_joins gmic::tests::render_tokens_argv_shape`
Expected: PASS. Also confirm the existing gmic tests still pass: `cargo test --lib gmic`.

- [ ] **Step 5 (optional, where gmic is installed): run the ignored e2e test**

Run: `cargo test --lib gmic::tests::render_with_tokens_produces_output -- --ignored`
Expected: PASS if gmic is installed; otherwise skip.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/gmic.rs
git commit -m "feat(gmic): add headless render_with_tokens for previews"
```

---

## Task 6: The `gen-previews` generator binary

**Files:**
- Modify/Create: `src/bin/gen-previews.rs`

**Interfaces:**
- Consumes: `catalogue::builtin`, `catalogue::{Folder, Node, Filter}`, `previews::{sanitise_key, default_argv}`, `previews::manifest::{self, Manifest, Entry, EntryStatus, Action}`, `gmic::{locate_gmic, render_with_tokens, GmicError}`.
- Produces: a CLI that writes `<out>/<key>.png` and `<manifest>`.

CLI: `gen-previews [--source PATH] [--out DIR] [--manifest PATH] [--jobs N] [--only CMD] [--force]`. Defaults: `--source assets/preview-source.tiff`, `--out previews`, `--manifest previews/manifest.json`, `--jobs` = available parallelism.

- [ ] **Step 1: Write the smoke test (ignored; needs gmic)**

Create `src/bin/gen-previews.rs` starting with the logic module + test so it is unit-testable. Put the pure planning step (which filters need recompute) in a function:

```rust
//! Build-time preview generator. See docs/superpowers/plans for design.
//!
//! Walks the bundled catalogue, renders one PNG per renderable filter
//! against a single sample image, and records results in a manifest so
//! reruns only recompute filters whose inputs changed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use GmicFilter::catalogue::{self, Filter, Folder, Node};
use GmicFilter::gmic::{self, GmicError};
use GmicFilter::previews::manifest::{self, Action, Entry, EntryStatus, Manifest};
use GmicFilter::previews::{default_argv, sanitise_key};

/// A unit of work: one renderable filter plus the argv we will send.
struct Job {
    command: String,
    args: Vec<String>,
    key: String,
}

/// Flatten the catalogue tree into render jobs.
fn collect_jobs(folder: &Folder, out: &mut Vec<Job>) {
    for child in &folder.children {
        match child {
            Node::Folder(f) => collect_jobs(f, out),
            Node::Filter(Filter { command, params, .. }) => {
                out.push(Job {
                    command: command.clone(),
                    args: default_argv(params),
                    key: sanitise_key(command),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_jobs_covers_every_filter() {
        let cat = catalogue::builtin();
        let mut jobs = Vec::new();
        collect_jobs(&cat.root, &mut jobs);
        // Sanity: the bundled catalogue has well over a thousand filters.
        assert!(jobs.len() > 1000, "expected >1000 jobs, got {}", jobs.len());
        // Keys must be unique so no two filters clobber each other's PNG.
        let mut keys: Vec<&str> = jobs.iter().map(|j| j.key.as_str()).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len(), "preview keys collided");
    }
}
```

- [ ] **Step 2: Run the test to verify it builds and passes**

Run: `cargo test --bin gen-previews`
Expected: PASS — confirms catalogue flattening + key uniqueness across the real catalogue.

- [ ] **Step 3: Implement `main` with arg parsing, the up-to-date check, and parallel rendering**

Append to `src/bin/gen-previews.rs`:

```rust
struct Config {
    source: PathBuf,
    out: PathBuf,
    manifest: PathBuf,
    jobs: usize,
    only: Option<String>,
    force: bool,
}

fn parse_args() -> Config {
    let mut cfg = Config {
        source: PathBuf::from("assets/preview-source.tiff"),
        out: PathBuf::from("previews"),
        manifest: PathBuf::from("previews/manifest.json"),
        jobs: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4),
        only: None,
        force: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--source" => cfg.source = it.next().expect("--source needs a value").into(),
            "--out" => cfg.out = it.next().expect("--out needs a value").into(),
            "--manifest" => cfg.manifest = it.next().expect("--manifest needs a value").into(),
            "--jobs" => cfg.jobs = it.next().and_then(|v| v.parse().ok()).unwrap_or(cfg.jobs),
            "--only" => cfg.only = Some(it.next().expect("--only needs a value")),
            "--force" => cfg.force = true,
            other => panic!("unknown flag: {other}"),
        }
    }
    cfg
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)?;
    Ok(format!("sha256:{:x}", Sha256::digest(&bytes)))
}

fn gmic_version(gmic: &Path) -> String {
    let out = std::process::Command::new(gmic).arg("--version").output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => "unknown".to_string(),
    }
}

fn main() {
    let cfg = parse_args();
    std::fs::create_dir_all(&cfg.out).expect("create out dir");

    let gmic = gmic::locate_gmic().expect("gmic must be installed to generate previews");
    let source_hash = sha256_file(&cfg.source).expect("source image must exist");
    let version = gmic_version(&gmic);

    let cat = catalogue::builtin();
    let mut jobs = Vec::new();
    collect_jobs(&cat.root, &mut jobs);
    if let Some(only) = &cfg.only {
        jobs.retain(|j| &j.command == only);
    }

    let prev = manifest::load(&cfg.manifest);
    // If the global inputs changed, every entry's hash will differ, so
    // we don't special-case it — but we DO carry the old entries so
    // unchanged filters short-circuit.
    let old_entries = prev.entries;

    let new_entries: Mutex<BTreeMap<String, Entry>> = Mutex::new(BTreeMap::new());
    let recomputed = AtomicUsize::new(0);
    let kept = AtomicUsize::new(0);
    let skipped = AtomicUsize::new(0);

    // Simple fixed-size worker pool over a shared job index.
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..cfg.jobs.max(1) {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(job) = jobs.get(i) else { break };
                let hash = manifest::input_hash(&source_hash, &version, &job.command, &job.args);
                let png_path = cfg.out.join(format!("{}.png", job.key));
                let existing = old_entries.get(&job.command);
                let action = if cfg.force {
                    Action::Recompute
                } else {
                    manifest::decide(existing, &hash, png_path.exists())
                };
                let entry = match action {
                    Action::Keep => {
                        kept.fetch_add(1, Ordering::Relaxed);
                        existing.cloned().expect("Keep implies an existing entry")
                    }
                    Action::Recompute => render_one(&gmic, &cfg.source, &png_path, job, &hash, &skipped, &recomputed),
                };
                new_entries.lock().unwrap().insert(job.command.clone(), entry);
            });
        }
    });

    let manifest_out = Manifest {
        source_hash,
        gmic_version: version,
        entries: new_entries.into_inner().unwrap(),
    };
    manifest::save(&cfg.manifest, &manifest_out).expect("write manifest");

    println!(
        "previews: {} recomputed, {} unchanged, {} skipped ({} total)",
        recomputed.load(Ordering::Relaxed),
        kept.load(Ordering::Relaxed),
        skipped.load(Ordering::Relaxed),
        jobs.len(),
    );
}

/// Render a single filter. On any gmic failure the preview is skipped
/// and recorded — the build never aborts on one bad filter.
fn render_one(
    gmic: &Path,
    source: &Path,
    png_path: &Path,
    job: &Job,
    hash: &str,
    skipped: &AtomicUsize,
    recomputed: &AtomicUsize,
) -> Entry {
    let dir = match tempfile::Builder::new().prefix("gmic-preview").tempdir() {
        Ok(d) => d,
        Err(e) => return skip(skipped, hash, format!("tempdir: {e}")),
    };
    // gmic infers RGB(3) output; the sample image is a colour photo.
    // `filter_tokens` reproduces EXACTLY what the picker sends: command
    // prefixed with `-`, args comma-joined into a single quoted token.
    let tokens = gmic::filter_tokens(&job.command, &job.args);
    match gmic::render_with_tokens(gmic, source, png_path, &tokens, 3, dir.path()) {
        Ok(()) if png_path.exists() => {
            recomputed.fetch_add(1, Ordering::Relaxed);
            Entry {
                input_hash: hash.to_string(),
                status: EntryStatus::Ok,
                file: Some(png_path.file_name().unwrap().to_string_lossy().into_owned()),
                reason: None,
            }
        }
        Ok(()) => skip(skipped, hash, "gmic produced no output file".to_string()),
        Err(e) => {
            // A failed render may have left a partial file; remove it.
            let _ = std::fs::remove_file(png_path);
            skip(skipped, hash, describe(&e))
        }
    }
}

fn skip(skipped: &AtomicUsize, hash: &str, reason: String) -> Entry {
    skipped.fetch_add(1, Ordering::Relaxed);
    Entry { input_hash: hash.to_string(), status: EntryStatus::Skip, file: None, reason: Some(reason) }
}

fn describe(e: &GmicError) -> String {
    match e {
        GmicError::TimedOut { seconds } => format!("timeout after {seconds}s"),
        GmicError::Failed { status } => format!("gmic exit {status:?}"),
        other => format!("{other}"),
    }
}
```

Note: `GmicFilter::gmic::{run_subprocess, render_with_tokens, filter_tokens}` are all `pub`; `GmicError` is `pub`. `filter_tokens` handles the leading-`-` prefix and arg comma-joining identically to the in-plugin path, so previews match what the picker sends.

- [ ] **Step 4: Re-run the unit test and a tiny live run**

Run: `cargo test --bin gen-previews`
Expected: PASS.

Where gmic is installed and after Task 7 provides the source image, a one-filter run validates end to end:
Run: `cargo run --release --bin gen-previews -- --only fx_oldphoto`
Expected: prints `1 recomputed, …`; `previews/fx_oldphoto-*.png` exists; `previews/manifest.json` written.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/bin/gen-previews.rs
git commit -m "feat(previews): add gen-previews generator binary"
```

---

## Task 7: Add the sample source image and generate the preview set

**Files:**
- Create: `assets/preview-source.tiff` (Git LFS)
- Create: `assets/preview-source.LICENSE.txt`
- Modify: `.gitattributes`
- Create/Generated: `previews/manifest.json`, `previews/*.png`

This task produces data, not code, so it has no TDD cycle; verification is by inspecting outputs.

- [ ] **Step 1: Obtain a CC0 / public-domain photo**

Pick a CC0 photo with varied content (faces/skin, texture, sharp edges, smooth sky/gradient) — e.g. from Wikimedia Commons (PD) or a CC0 source. Save the original locally (not in the repo yet).

- [ ] **Step 2: Downscale to a 512px max edge TIFF**

Use the installed gmic to produce the canonical source so it matches the render pipeline's expectations:

```bash
gmic <downloaded-image> -resize2dx 512,512,2 -to_rgb -output assets/preview-source.tiff
```

(`-resize2dx 512` scales the longest edge to 512 preserving aspect; adjust to `-resize2dy` if the image is portrait. Confirm the result is ≤512 on its max edge with `gmic assets/preview-source.tiff -echo_stdout {w},{h}` or `sips -g pixelWidth -g pixelHeight assets/preview-source.tiff`.)

- [ ] **Step 3: Record the license**

Create `assets/preview-source.LICENSE.txt`:

```text
Preview sample image
=====================
Source URL: <exact URL the image came from>
Author:     <author or "Unknown">
License:    <CC0 / Public Domain — with link to the deed>
Retrieved:  2026-06-28

Downscaled to 512px max edge and converted to TIFF for use as the
fixed input to build-time G'MIC filter previews (see
docs/superpowers/plans/2026-06-28-static-filter-previews.md).
```

- [ ] **Step 4: Track the TIFF via Git LFS**

Append to `.gitattributes`:

```text
assets/preview-source.tiff filter=lfs diff=lfs merge=lfs -text
```

Then:

```bash
git lfs track "assets/preview-source.tiff"   # idempotent; confirms the pattern
git add .gitattributes assets/preview-source.tiff assets/preview-source.LICENSE.txt
```

- [ ] **Step 5: Generate the full preview set**

Run: `cargo run --release --bin gen-previews`
Expected: prints `N recomputed, 0 unchanged, K skipped (M total)` with M > 1000. `previews/` fills with PNGs + `manifest.json`.

- [ ] **Step 6: Spot-check outputs and commit**

```bash
# A few known filters should have non-empty PNGs:
ls -la previews | head
git add previews
git commit -m "feat(previews): add sample image and generated preview set"
```

(If `previews/` proves very large, this is the point to reconsider LFS for `previews/*.png`; deferred per spec — no code change needed.)

---

## Task 8: Wire the generator and PNGs into the Makefile build

**Files:**
- Modify: `Makefile` (`.PHONY` line ~146, `bundle` ~215, `universal` ~227, add `previews` target + vars near top)

**Interfaces:**
- Produces: `make previews` runs the generator (up-to-date-checked); `bundle`/`universal` copy `previews/*.png` into `$(BUNDLE)/Contents/Resources/previews/`.

- [ ] **Step 1: Add path vars near the other `BUNDLE_*` definitions (~line 30)**

```make
PREVIEWS_SRC    := previews
BUNDLE_PREVIEWS := $(BUNDLE)/Contents/Resources/previews
```

- [ ] **Step 2: Add `previews` to `.PHONY` (line ~146)**

Append `previews` to the existing `.PHONY:` list.

- [ ] **Step 3: Add the `previews` target**

After the `refresh-catalogue` target (or near the other tooling targets), add:

```make
# Regenerate filter previews. The generator does its own per-filter
# up-to-date check against previews/manifest.json, so re-running this
# when nothing changed is cheap. Requires the gmic CLI.
previews: check-lfs
	cargo run --release --bin gen-previews
```

- [ ] **Step 4: Copy PNGs into the bundle in `bundle` and `universal`**

In BOTH the `bundle:` and `universal:` recipes, after the `cp -R "$(LPROJ_SRC)" "$(BUNDLE_LPROJ)"` line and BEFORE the `codesign` line, add:

```make
	mkdir -p "$(BUNDLE_PREVIEWS)"
	cp "$(PREVIEWS_SRC)"/*.png "$(BUNDLE_PREVIEWS)/" 2>/dev/null || true
```

(The `|| true` keeps a clean checkout without generated PNGs from breaking the bundle build; the picker simply shows placeholders.)

- [ ] **Step 5: Build and verify the bundle contains previews**

Run: `make bundle && ls GmicFilter.plugin/Contents/Resources/previews | head`
Expected: bundle builds; preview PNGs are present in Resources.

- [ ] **Step 6: Commit**

```bash
git add Makefile
git commit -m "build(previews): generate and bundle filter previews"
```

---

## Task 9: Resolve the bundle previews directory at runtime

**Files:**
- Create: `src/ui/preview_pane.rs`
- Modify: `src/ui/mod.rs` (declare the module, live-gated)

**Interfaces:**
- Produces: `pub(crate) fn previews_dir() -> Option<std::path::PathBuf>` — returns `…/GmicFilter.plugin/Contents/Resources/previews` by locating the running binary via `dladdr`, or `None` if it can't be resolved.
- Produces: `pub(crate) fn preview_path_for(command: &str) -> Option<std::path::PathBuf>` — `previews_dir()/<sanitise_key(command)>.png` if that file exists, else `None`.

- [ ] **Step 1: Write the failing test**

Create `src/ui/preview_pane.rs` with the path logic + a test that doesn't need AppKit:

```rust
//! Live-only preview pane for the picker: resolves, loads, and renders
//! the pre-computed PNG for the selected filter, or a placeholder.

use std::path::PathBuf;

use crate::previews::sanitise_key;

/// Locate the `previews` directory inside the running plugin bundle.
///
/// The loadable binary lives at `…/GmicFilter.plugin/Contents/MacOS/
/// GmicFilter`; previews sit at `…/Contents/Resources/previews`. We find
/// our own on-disk path with `dladdr` on a local symbol, then walk up
/// from `MacOS/<exe>` to `Contents` and back down to `Resources`.
pub(crate) fn previews_dir() -> Option<PathBuf> {
    // Dev/example override: point at the repo's `previews/` dir when not
    // running inside an installed bundle (used by `make picker-example`).
    if let Some(dir) = std::env::var_os("GMIC_PREVIEWS_DIR") {
        return Some(PathBuf::from(dir));
    }
    let exe = self_path()?;
    // exe = …/Contents/MacOS/GmicFilter ; parent twice = …/Contents
    let contents = exe.parent()?.parent()?;
    Some(contents.join("Resources").join("previews"))
}

/// Path to the PNG for `command` if it exists on disk.
pub(crate) fn preview_path_for(command: &str) -> Option<PathBuf> {
    let p = previews_dir()?.join(format!("{}.png", sanitise_key(command)));
    p.exists().then_some(p)
}

fn self_path() -> Option<PathBuf> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // Take the address of a function in THIS image so dladdr resolves
    // to our loadable bundle, not the host app.
    let addr = self_path as *const () as *const libc::c_void;
    if unsafe { libc::dladdr(addr, &mut info) } == 0 || info.dli_fname.is_null() {
        return None;
    }
    let cstr = unsafe { CStr::from_ptr(info.dli_fname) };
    let os = std::ffi::OsStr::from_bytes(cstr.to_bytes());
    Some(PathBuf::from(os))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_dir_ends_with_expected_suffix() {
        // We can't assert the absolute path in a test binary, but the
        // resolver must always end with Resources/previews when it
        // returns Some (and no env override is in play).
        if std::env::var_os("GMIC_PREVIEWS_DIR").is_none() {
            if let Some(dir) = previews_dir() {
                assert!(dir.ends_with("Resources/previews"));
            }
        }
    }

    #[test]
    fn preview_filename_stem_is_path_safe() {
        // The filename stem the loader builds must contain no separators
        // that would escape the previews dir — sanitise_key guarantees
        // this and it's the contract the loader relies on.
        let key = sanitise_key("foo/../bar baz");
        assert!(!key.contains('/') && !key.contains('\\') && !key.contains(".."));
    }
}
```

- [ ] **Step 2: Declare the module (live-gated) in `src/ui/mod.rs`**

After the other `#[cfg(feature = "live")]` module lines add:

```rust
#[cfg(feature = "live")]
pub(crate) mod preview_pane;
```

- [ ] **Step 3: Run the tests**

Run: `cargo test --features live --lib ui::preview_pane`
Expected: PASS (2 tests; `previews_dir` returns the test binary's path or `None`, both acceptable).

- [ ] **Step 4: Commit**

```bash
cargo fmt
git add src/ui/preview_pane.rs src/ui/mod.rs
git commit -m "feat(ui): resolve bundle previews dir via dladdr"
```

---

## Task 10: Add the preview pane to the picker (third split-view column)

**Files:**
- Modify: `src/ui/preview_pane.rs` (add the AppKit view builder + image loader)
- Modify: `src/ui/picker.rs` (3-pane split, min-width delegate, wider window)
- Modify: `src/ui/picker_form.rs` (notify the preview pane on selection change)

**Interfaces:**
- Consumes: `preview_pane::preview_path_for`, `picker_form::FormController` selection callback.
- Produces: `pub(crate) struct PreviewView { root: Retained<NSView>, image: Retained<NSImageView>, caption: Retained<NSTextField> }` with `pub(crate) fn build_preview_view(mtm) -> PreviewView` and `pub(crate) fn show(&self, command: &str, description: Option<&str>)`.

This is AppKit glue; it is verified by building the live target and by the existing manual picker example (`make picker-example`). No new unit test (UI construction isn't unit-testable here); the logic pieces it relies on are already tested in Tasks 2 and 9.

- [ ] **Step 1: Add the view builder and loader to `preview_pane.rs`**

Append to `src/ui/preview_pane.rs`:

```rust
use objc2::rc::Retained;
use objc2::ClassType;
use objc2_app_kit::{
    NSImage, NSImageScaling, NSImageView, NSTextField, NSView,
};
use objc2_foundation::{CGRect, CGPoint, CGSize, MainThreadMarker, NSString};

/// The preview column: an image view on top, a wrapping caption below.
pub(crate) struct PreviewView {
    pub(crate) root: Retained<NSView>,
    image: Retained<NSImageView>,
    caption: Retained<NSTextField>,
}

/// Build the preview column. Layout is handled by autoresizing masks so
/// the split view can resize the pane freely.
pub(crate) fn build_preview_view(mtm: MainThreadMarker) -> PreviewView {
    let root = unsafe {
        NSView::initWithFrame(mtm.alloc(), CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize { width: 280.0, height: 400.0 },
        })
    };
    let image = unsafe { NSImageView::initWithFrame(mtm.alloc(), CGRect {
        origin: CGPoint { x: 8.0, y: 110.0 },
        size: CGSize { width: 264.0, height: 282.0 },
    }) };
    unsafe {
        image.setImageScaling(NSImageScaling::NSImageScaleProportionallyUpOrDown);
        image.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::NSViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::NSViewHeightSizable,
        );
    }
    let caption = unsafe {
        let label = NSTextField::initWithFrame(mtm.alloc(), CGRect {
            origin: CGPoint { x: 8.0, y: 8.0 },
            size: CGSize { width: 264.0, height: 96.0 },
        });
        label.setBezeled(false);
        label.setEditable(false);
        label.setSelectable(false);
        label.setDrawsBackground(false);
        label.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::NSViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::NSViewMaxYMargin,
        );
        label
    };
    unsafe {
        let image_view: &NSView = &image;
        let caption_view: &NSView = &caption;
        root.addSubview(image_view);
        root.addSubview(caption_view);
    }
    let view = PreviewView { root, image, caption };
    view.show_placeholder();
    view
}

impl PreviewView {
    /// Show the preview for `command` (or a placeholder if none exists)
    /// and the filter `description` as the caption.
    pub(crate) fn show(&self, command: &str, description: Option<&str>) {
        match super::preview_pane::preview_path_for(command) {
            Some(path) => self.set_image_from(&path),
            None => self.show_placeholder(),
        }
        let text = description.unwrap_or("");
        unsafe { self.caption.setStringValue(&NSString::from_str(text)); }
    }

    fn set_image_from(&self, path: &std::path::Path) {
        let ns = NSString::from_str(&path.to_string_lossy());
        let img: Option<Retained<NSImage>> =
            unsafe { NSImage::initWithContentsOfFile(NSImage::alloc(), &ns) };
        match img {
            Some(image) => unsafe { self.image.setImage(Some(&image)) },
            None => self.show_placeholder(),
        }
    }

    fn show_placeholder(&self) {
        unsafe { self.image.setImage(None) };
        // A nil image + a clear caption reads as "no preview"; the
        // caption is overwritten by `show` with the description when one
        // exists.
        unsafe { self.caption.setStringValue(&NSString::from_str("No preview available")); }
    }
}
```

(If `NSImage::initWithContentsOfFile` / `setImage` require additional `objc2-app-kit` features, add `NSImage` and `NSImageView` to the `objc2-app-kit` feature list in `Cargo.toml`.)

- [ ] **Step 2: Add the `NSImage`/`NSImageView` features to `Cargo.toml`**

In the `objc2-app-kit` `features = [...]` array add `"NSImage"` and `"NSImageView"` (keep the list alphabetical-ish, matching the existing style).

- [ ] **Step 3: Make the split view 3-pane in `picker.rs`**

In `src/ui/picker.rs`:

1. Add a constant near the other pane constants:

```rust
/// Initial width of the rightmost preview pane, in points.
const PREVIEW_PANE_WIDTH: CGFloat = 280.0;
/// Minimum preview-pane width so the image stays legible.
const PREVIEW_PANE_MIN_WIDTH: CGFloat = 220.0;
```

2. Build the preview view and pass it into the split. After `build_form_pane(...)` returns, add:

```rust
let preview = crate::ui::preview_pane::build_preview_view(mtm);
```

3. Change `build_split_view` to accept and add a third subview, and set two divider positions. Update its signature to `fn build_split_view(mtm, content_bounds, tree, form, preview: &NSView)`; after `split.addSubview(form_view);` add `split.addSubview(preview);`. Replace the single `setPosition_ofDividerAtIndex` block with positioning for two dividers:

```rust
let total_width = frame.size.width;
let mut tree_width = (total_width * TREE_PANE_WIDTH_FRACTION).max(TREE_PANE_MIN_WIDTH);
let preview_width = PREVIEW_PANE_WIDTH.min(total_width - TREE_PANE_MIN_WIDTH - FORM_PANE_MIN_WIDTH);
if total_width - tree_width - preview_width < FORM_PANE_MIN_WIDTH {
    tree_width = total_width - preview_width - FORM_PANE_MIN_WIDTH;
}
// Divider 0 between tree and form; divider 1 between form and preview.
split.setPosition_ofDividerAtIndex(tree_width, 0);
split.setPosition_ofDividerAtIndex(tree_width + (total_width - tree_width - preview_width), 1);
```

4. Update the call site: `let split = build_split_view(mtm, content_bounds, &tree_scroll, &form_scroll, &preview.root);`

5. Widen the default window: in `build_picker_window`, change the initial content rect `width: 720.0` to `width: 1040.0` and `window.setMinSize` width from `520.0` to `760.0`.

- [ ] **Step 4: Keep `preview` alive and wire selection changes**

The `FormController` already observes outline selection changes (it rebuilds the form). Give it a way to also update the preview:

1. In `src/ui/picker_form.rs`, add an optional preview handle to `FormController`'s ivars. Where the ivars struct is declared, add a field `preview: OnceCell<Retained<crate::ui::preview_pane::PreviewView>>` is not possible (PreviewView isn't an objc object). Instead store a boxed callback. Simplest faithful approach: store `preview: RefCell<Option<std::rc::Rc<crate::ui::preview_pane::PreviewView>>>`. Change `build_preview_view` to return `Rc<PreviewView>` (wrap at the call site) OR add a setter on `FormController`:

```rust
// in FormController impl
pub(crate) fn set_preview(&self, preview: std::rc::Rc<crate::ui::preview_pane::PreviewView>) {
    self.ivars().preview.replace(Some(preview));
}
```

with the ivar `preview: RefCell<Option<std::rc::Rc<crate::ui::preview_pane::PreviewView>>>` (initialise to `RefCell::new(None)` in the ivars constructor).

2. In `outline_view_selection_did_change` (after the form is rebuilt for the selected `Filter`), call:

```rust
if let Some(preview) = self.ivars().preview.borrow().as_ref() {
    preview.show(&filter.command, filter.description.as_deref());
}
```

(`filter` is the `&Filter` already resolved in that handler when a leaf row is selected; for folder/empty selection call `preview.show("", None)` to reset.)

3. In `picker.rs`, wrap the preview in an `Rc` and hand it to the controller before the modal pump:

```rust
let preview = std::rc::Rc::new(crate::ui::preview_pane::build_preview_view(mtm));
form_controller.set_preview(preview.clone());
```

and pass `&preview.root` to `build_split_view`. Keep `preview` bound until after `run_modal_window` returns so it isn't dropped early (drop it alongside `drop(form_controller)` at the end).

- [ ] **Step 5: Build the live target**

Run: `cargo build --features live`
Expected: PASS. Fix any objc2 feature-gate errors by adding the missing class to the `objc2-app-kit` features list.

- [ ] **Step 6: Manual smoke test via the example**

Run: `GMIC_PREVIEWS_DIR="$PWD/previews" make picker-example` (builds and launches `examples/picker.rs` with `--features live`; the env var points the loader at the repo's generated PNGs since the example isn't an installed `.plugin` bundle).
Expected: the picker opens with three columns; selecting a filter shows its preview image on the right with the description beneath, and filters without a PNG show "No preview available".

- [ ] **Step 7: Run the full test + lint gate**

Run: `make test && make clippy`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
cargo fmt
git add src/ui/preview_pane.rs src/ui/picker.rs src/ui/picker_form.rs Cargo.toml
git commit -m "feat(ui): add filter preview pane to the picker"
```

---

## Self-Review Notes

- **Spec coverage:** scope (all renderable filters) → Task 6 `collect_jobs`; medium 512px source → Task 7; storage in Resources, lazy-loaded → Tasks 8–10; per-filter content-hash manifest → Tasks 4, 6; generator as Rust bin reusing catalogue/defaults/gmic → Tasks 3, 5, 6; skip+record+placeholder failure handling → Tasks 6, 10; three-column UI with description → Task 10. All spec sections map to a task.
- **Critical correctness point:** previews must match the picker's real argv (`collect_values`), not `reconcile::default_for`. Task 3's `default_argv` encodes this and the tests lock it (Choice→index, Bool→`1/0`, Unknown→nothing).
- **Type consistency:** `Entry`, `EntryStatus`, `Action`, `Manifest` defined in Task 4 are used unchanged in Task 6; `sanitise_key` (Task 2) is the single filename source of truth used by both generator (Task 6) and UI loader (Task 9). `format_float` defined once (Task 3) and reused by `picker_form`.
- **Deferred (no code impact):** moving `previews/*.png` to LFS if the directory grows large.
