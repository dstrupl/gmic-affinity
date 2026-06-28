//! Wrapper around the Homebrew-installed `gmic` binary.
//!
//! Responsibilities:
//! - Locate the `gmic` executable in the well-known Homebrew install paths.
//! - Read the user's filter command (config file or built-in default) and
//!   reject obviously dangerous payloads.
//! - Build a sanitised argv list (no shell, no env inheritance) and execute
//!   the subprocess.
//! - Round-trip pixels through TIFF temp files in a per-invocation
//!   directory created with 0700 permissions via the `tempfile` crate.

use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::filter::{validate_filter_record, FilterError};
use crate::logging::log;
use crate::ps_types::FilterRecord;
use crate::tiff_io::{read_tiff, write_tiff, TiffError};

/// Default filter applied if the user hasn't dropped a custom command into
/// `~/.config/gmic-affinity/filter.txt`. Chosen to produce an unmistakable
/// visual change so it's obvious when the plugin has actually run.
pub const DEFAULT_FILTER: &str = "-fx_oldphoto";

/// Maximum size accepted for `filter.txt` to discourage someone pasting an
/// arbitrarily large script.
pub const MAX_FILTER_CONFIG_BYTES: u64 = 4 * 1024;

/// Cap argv length and per-argument length to keep the subprocess command
/// line sane.
pub const MAX_FILTER_ARGS: usize = 64;
pub const MAX_ARG_BYTES: usize = 1024;
pub const GMIC_TIMEOUT_SECS: u64 = 60;

const LINIFY_MAX_EDGE: u32 = 1024;

#[derive(Debug)]
pub enum GmicError {
    Validation(FilterError),
    Tiff(TiffError),
    Io(std::io::Error),
    NotFound,
    ConfigTooLarge(u64),
    InvalidCharsInConfig,
    TooManyArgs(usize),
    ArgTooLong(usize),
    Failed {
        status: Option<i32>,
    },
    TimedOut {
        seconds: u64,
    },
    UnsupportedForImageSize {
        command: String,
        width: u32,
        height: u32,
        max_edge: u32,
    },
}

impl std::fmt::Display for GmicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GmicError::Validation(e) => write!(f, "{e}"),
            GmicError::Tiff(e) => write!(f, "{e}"),
            GmicError::Io(e) => write!(f, "I/O: {e}"),
            GmicError::NotFound => {
                write!(f, "gmic binary not found in any known Homebrew location")
            }
            GmicError::ConfigTooLarge(n) => {
                write!(f, "filter.txt is {n} bytes (max {MAX_FILTER_CONFIG_BYTES})")
            }
            GmicError::InvalidCharsInConfig => {
                write!(f, "filter.txt contains NUL or control characters")
            }
            GmicError::TooManyArgs(n) => write!(f, "filter has {n} args (max {MAX_FILTER_ARGS})"),
            GmicError::ArgTooLong(n) => write!(f, "filter arg is {n} bytes (max {MAX_ARG_BYTES})"),
            GmicError::Failed { status } => match status {
                Some(c) => write!(f, "gmic exited with status {c}"),
                None => write!(f, "gmic terminated by signal"),
            },
            GmicError::TimedOut { seconds } => {
                write!(f, "gmic did not finish within {seconds}s")
            }
            GmicError::UnsupportedForImageSize {
                command,
                width,
                height,
                max_edge,
            } => write!(
                f,
                "{command} is limited to images up to {max_edge}px on the longest edge (got {width}x{height})"
            ),
        }
    }
}

impl std::error::Error for GmicError {}

impl From<FilterError> for GmicError {
    fn from(e: FilterError) -> Self {
        GmicError::Validation(e)
    }
}
impl From<TiffError> for GmicError {
    fn from(e: TiffError) -> Self {
        GmicError::Tiff(e)
    }
}
impl From<std::io::Error> for GmicError {
    fn from(e: std::io::Error) -> Self {
        GmicError::Io(e)
    }
}

/// Resolve the actual gmic output file for an expected single-image
/// output path. gmic writes `<stem>.<ext>` when the image list has
/// exactly one image, but `<stem>_000000.<ext>`, `_000001`, … when a
/// filter leaves multiple images (animations, or result+passthrough
/// filters). Affinity accepts one image, so we use frame 0: prefer the
/// exact path, else fall back to the `_000000` sibling.
pub(crate) fn resolve_output_path(expected: &Path) -> Option<PathBuf> {
    if expected.exists() {
        return Some(expected.to_path_buf());
    }

    // Construct frame-0 sibling: insert _000000 before the extension
    let parent = expected.parent()?;
    let stem = expected.file_stem()?.to_str()?;
    let frame0_stem = format!("{}_000000", stem);

    let frame0_path = if let Some(ext) = expected.extension() {
        parent.join(format!("{}.{}", frame0_stem, ext.to_str()?))
    } else {
        parent.join(frame0_stem)
    };

    if frame0_path.exists() {
        Some(frame0_path)
    } else {
        None
    }
}

/// Find the Homebrew-installed `gmic`, preferring Apple Silicon's
/// `/opt/homebrew/bin/gmic` over Intel's `/usr/local/bin/gmic`. The path is
/// canonicalised so we don't follow surprise symlinks elsewhere.
pub fn locate_gmic() -> Result<PathBuf, GmicError> {
    for candidate in ["/opt/homebrew/bin/gmic", "/usr/local/bin/gmic"] {
        let p = Path::new(candidate);
        if let Ok(meta) = fs::symlink_metadata(p) {
            // Allow symlinks (Homebrew installs are symlinks into the Cellar)
            // but require the resolved target to actually exist and be
            // executable by the current user.
            if let Ok(canon) = fs::canonicalize(p) {
                if let Ok(canon_meta) = fs::metadata(&canon) {
                    let perms = canon_meta.permissions();
                    if perms.mode() & 0o111 != 0 {
                        let _ = meta;
                        return Ok(canon);
                    }
                }
            }
        }
    }
    Err(GmicError::NotFound)
}

/// Read the user's filter command, falling back to `DEFAULT_FILTER`.
/// Rejects oversized files and any content containing NUL or other control
/// characters that could surprise downstream parsing.
pub fn read_filter_config() -> Result<String, GmicError> {
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(DEFAULT_FILTER.to_string());
    };
    let path = PathBuf::from(home)
        .join(".config")
        .join("gmic-affinity")
        .join("filter.txt");

    let Ok(mut file) = fs::File::open(&path) else {
        return Ok(DEFAULT_FILTER.to_string());
    };

    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if len > MAX_FILTER_CONFIG_BYTES {
        return Err(GmicError::ConfigTooLarge(len));
    }

    let mut contents = String::with_capacity(len as usize);
    file.read_to_string(&mut contents)?;
    validate_filter_string(&contents)?;
    Ok(contents.trim().to_string())
}

fn validate_filter_string(s: &str) -> Result<(), GmicError> {
    for c in s.chars() {
        // Allow tab, space, newline, CR; reject everything else in the
        // C0/C1 control ranges plus NUL. This keeps the command parseable
        // and prevents accidental terminal escapes ending up in logs.
        if c == '\0' {
            return Err(GmicError::InvalidCharsInConfig);
        }
        if (c.is_control()) && !matches!(c, '\t' | '\n' | '\r') {
            return Err(GmicError::InvalidCharsInConfig);
        }
    }
    Ok(())
}

/// Build the full argv to hand to `Command::new(gmic_path).args(...)`.
/// Format:
/// `[<input.tif>, <filter tokens...>, <color-mode>, -output, <output.tif>]`.
///
/// We deliberately *don't* try to force the output pixel type via the
/// `,uchar` (or any other) suffix on `-output`. The set of accepted
/// type names in `output_tiff` is undocumented across gmic versions
/// (3.7.6 verbose-logs the value back but then rejects 'uchar' at
/// write time, for instance).
///
/// We do force the output colour model to match the host plane count.
/// Some filters, notably `fx_ghost`, return float gray+alpha TIFFs
/// (`BlackIsZero` with two 32-bit samples), which the `tiff` crate
/// rejects before handing us samples. Normalising to Gray/RGB/RGBA
/// preserves the existing flexible bit-depth readback while avoiding
/// unsupported two-channel TIFF layouts.
pub fn build_argv(
    input: &Path,
    output: &Path,
    filter_cmd: &str,
    output_planes: u32,
) -> Result<Vec<OsString>, GmicError> {
    let mut args: Vec<OsString> = Vec::with_capacity(8);
    args.push(input.as_os_str().to_owned());

    let mut count = 0usize;
    for token in filter_cmd.split_whitespace() {
        if token.len() > MAX_ARG_BYTES {
            return Err(GmicError::ArgTooLong(token.len()));
        }
        args.push(OsString::from(token));
        count += 1;
        if count > MAX_FILTER_ARGS {
            return Err(GmicError::TooManyArgs(count));
        }
    }

    append_output_args(&mut args, output, output_planes)?;
    Ok(args)
}

/// Execute gmic with a cleared environment, hiding its stdout/stderr so we
/// don't pollute Console.app on success. On failure we surface the exit
/// status so the caller can log it.
pub fn run_subprocess(gmic: &Path, args: &[OsString], tmpdir: &Path) -> Result<(), GmicError> {
    run_subprocess_with_timeout(gmic, args, tmpdir, Duration::from_secs(GMIC_TIMEOUT_SECS))
}

fn run_subprocess_with_timeout(
    gmic: &Path,
    args: &[OsString],
    tmpdir: &Path,
    timeout: Duration,
) -> Result<(), GmicError> {
    log(&format!(
        "spawn {} (argc={}) tmpdir={}",
        gmic.display(),
        args.len(),
        tmpdir.display()
    ));
    let mut child = Command::new(gmic)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", tmpdir)
        .env("TMPDIR", tmpdir)
        .env("LANG", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if started.elapsed() >= timeout {
            log(&format!(
                "gmic timeout after {:.1}s; killing child",
                timeout.as_secs_f64()
            ));
            let _ = child.kill();
            let output = child.wait_with_output()?;
            log_gmic_output(&output);
            return Err(GmicError::TimedOut {
                seconds: timeout.as_secs(),
            });
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let output = child.wait_with_output()?;
    log_gmic_output(&output);
    if !output.status.success() {
        return Err(GmicError::Failed {
            status: output.status.code(),
        });
    }
    Ok(())
}

fn log_gmic_output(output: &std::process::Output) {
    log(&format!(
        "gmic exit status={:?} stdout={}B stderr={}B",
        output.status.code(),
        output.stdout.len(),
        output.stderr.len()
    ));
    // Spill stderr (and stdout) so the user can see *why* gmic complained
    // even when it nominally succeeds. Cap at 8 KiB to avoid blowing up
    // the log when a filter prints a per-pixel warning.
    if !output.stderr.is_empty() {
        let trimmed: String = String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(8 * 1024)
            .collect();
        log(&format!("gmic stderr: {trimmed}"));
    }
    if !output.stdout.is_empty() {
        let trimmed: String = String::from_utf8_lossy(&output.stdout)
            .chars()
            .take(8 * 1024)
            .collect();
        log(&format!("gmic stdout: {trimmed}"));
    }
}

/// Full filter run: validate FilterRecord, write input TIFF, exec gmic,
/// read output TIFF back into host buffer. The temp directory is created
/// with the user's default umask (typically 0700 for tempfile crate's
/// `tempdir()`); both TIFFs live inside it and are auto-removed on drop.
///
/// Uses the legacy `filter.txt` config file to pick the command. Kept
/// for back-compat with the M3 / M4 bring-up tests; production callers
/// (`PluginMain::SELECTOR_CONTINUE`) go through [`run_filter_with`].
pub fn run_filter(fr: &mut FilterRecord) -> Result<(), GmicError> {
    let cmd = read_filter_config()?;
    log(&format!("filter config: {cmd:?}"));
    // `read_filter_config` returns the full command line as a single
    // whitespace-separated string. Tokenise it the same way we did
    // before so `run_with_tokens` sees a stable shape regardless of
    // who composed it.
    let tokens: Vec<String> = cmd.split_whitespace().map(str::to_owned).collect();
    if tokens.is_empty() {
        // Empty config file: surface the same "filter has bad bytes"
        // error rather than passing an empty argv to gmic (which
        // would happily run on the input image and produce an
        // identity output, masking the configuration mistake).
        return Err(GmicError::InvalidCharsInConfig);
    }
    run_with_tokens(fr, &tokens)
}

/// Like [`run_filter`] but takes a [`ChosenFilter`] directly (from the
/// picker) instead of reading `filter.txt`. Goes through the same
/// `MAX_FILTER_ARGS` / `MAX_ARG_BYTES` / NUL & control-char checks as
/// the existing file-based path; those move from "validate parsed
/// file" to "validate dialog output".
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
        if arg
            .bytes()
            .any(|b| b == 0 || (b.is_ascii_control() && !matches!(b, b'\t' | b'\n' | b'\r')))
        {
            return Err(GmicError::InvalidCharsInConfig);
        }
    }
    // gmic's CLI is `cmd a,b,c` — every parameter for a single
    // filter invocation is one comma-joined token, NOT one process
    // argv per parameter. Earlier we pushed each arg as its own
    // token; gmic then saw the parameters as separate top-level
    // commands and the filter's internal `${1-N}` substitution had
    // nothing to grab, producing
    //   *** Error in ./fx_paint_with_brush/*substitute/ ***
    //   Unknown command or filename '1-35'.
    // (The literal '1-35' is fx_paint_with_brush asking for "args
    // 1 through 35" of its declared parameter list.)
    let tokens = filter_tokens(&chosen.command, &chosen.args);
    run_with_tokens(fr, &tokens)
}

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
        let joined = args
            .iter()
            .map(|a| quote_gmic_arg(a))
            .collect::<Vec<_>>()
            .join(",");
        tokens.push(joined);
    }
    tokens
}

/// Quote a single parameter value according to gmic CLI rules so it
/// survives being pasted into a comma-separated parameter list.
///
/// gmic accepts the bare form for any value that contains none of:
///   - a comma  (would split the parameter list)
///   - leading or trailing whitespace (gmic trims those)
///   - an embedded double quote (would close the quoted form)
///
/// Anything else gets wrapped in `"..."` with internal `"` escaped
/// to `\"`. Embedded newlines / tabs are allowed both bare and
/// quoted; gmic treats them as ordinary characters once the
/// parameter boundary is established.
///
/// This matters for, among others, our `point(...)` parser which
/// stores the picker default as the Text value `"x,y"` — without
/// quoting, the embedded comma would silently split a 35-arg filter
/// into a 36-arg one and shift every subsequent positional value.
fn quote_gmic_arg(value: &str) -> String {
    let needs_quoting = value.contains(',')
        || value.contains('"')
        || value.starts_with(|c: char| c.is_whitespace())
        || value.ends_with(|c: char| c.is_whitespace());
    if !needs_quoting {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        if ch == '"' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// Shared body of [`run_filter`] and [`run_filter_with`]: validate
/// the filter record, write the input TIFF, exec gmic, read the
/// output TIFF back. `tokens` is `[command, arg, arg, …]` with the
/// command already prefixed by `-` if it was a builtin filter.
fn run_with_tokens(fr: &mut FilterRecord, tokens: &[String]) -> Result<(), GmicError> {
    let buf = validate_filter_record(fr)?;
    log(&format!(
        "run_filter: width={} height={} planes={} in_row_bytes={} out_row_bytes={}",
        buf.width, buf.height, buf.planes, buf.in_row_bytes, buf.out_row_bytes
    ));
    reject_known_expensive_filter(tokens, buf.width as u32, buf.height as u32)?;

    let gmic = locate_gmic()?;
    log(&format!("located gmic at {}", gmic.display()));

    let dir = tempfile::Builder::new()
        .prefix("gmic-affinity-")
        .tempdir()?;
    // Belt-and-braces: tempfile already chmods to 0700 on Unix; assert that.
    let _ = fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700));

    let in_path = dir.path().join("in.tif");
    let out_path = dir.path().join("out.tif");

    let in_total = (buf.in_row_bytes as usize) * (buf.height as usize);
    let out_total = (buf.out_row_bytes as usize) * (buf.height as usize);
    let in_slice = unsafe { std::slice::from_raw_parts(buf.in_data, in_total) };
    let out_slice = unsafe { std::slice::from_raw_parts_mut(buf.out_data, out_total) };

    write_tiff(
        &in_path,
        in_slice,
        buf.width as u32,
        buf.height as u32,
        buf.planes as u32,
        buf.in_row_bytes as u32,
    )?;

    let argv = build_argv_from_tokens(&in_path, &out_path, tokens, buf.planes as u32)?;
    run_subprocess(&gmic, &argv, dir.path())?;

    // Resolve the actual output path: gmic writes out.tif for single-image
    // results, but out_000000.tif, out_000001.tif, … for multi-output filters.
    // We use frame 0 as the single-image result for Affinity.
    let actual_output = resolve_output_path(&out_path).ok_or_else(|| {
        GmicError::Tiff(TiffError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("gmic produced no output at {}", out_path.display()),
        )))
    })?;

    if actual_output != out_path {
        log(&format!(
            "multi-output filter: using frame 0 at {}",
            actual_output.display()
        ));
    }

    read_tiff(
        &actual_output,
        out_slice,
        buf.width as u32,
        buf.height as u32,
        buf.planes as u32,
        buf.out_row_bytes as u32,
    )?;

    Ok(())
}

/// Headless render path used by the build-time preview generator.
///
/// Unlike [`run_filter_with`], this takes file paths directly and never
/// touches a `FilterRecord`: load `input`, apply `tokens`, force the
/// colour model from `output_planes`, and write `output`. The argv caps
/// and subprocess hardening are identical to the in-plugin path.
///
/// The dimension-keyed [`reject_known_expensive_filter`] guard is
/// intentionally omitted here: the caller (the preview generator)
/// controls the input size by rendering against one small fixed sample
/// image, and each call is already timeout-bounded by `run_subprocess`.
pub fn render_with_tokens(
    gmic: &Path,
    input: &Path,
    output: &Path,
    tokens: &[String],
    output_planes: u32,
    tmpdir: &Path,
) -> Result<(), GmicError> {
    let argv = build_argv_from_tokens(input, output, tokens, output_planes)?;
    run_subprocess(gmic, &argv, tmpdir)?;

    // Multi-output filters write output_000000.ext instead of output.ext.
    // Promote frame 0 to the expected path so the caller's contract holds.
    if !output.exists() {
        if let Some(frame0) = resolve_output_path(output) {
            if frame0 != output {
                log(&format!(
                    "multi-output filter: promoting {} to {}",
                    frame0.display(),
                    output.display()
                ));
                // Both files are in the same tmpdir, so rename should work
                fs::rename(&frame0, output)?;
            }
        }
        // If neither exists, leave as-is (caller treats absent output as skip)
    }

    Ok(())
}

fn reject_known_expensive_filter(
    tokens: &[String],
    width: u32,
    height: u32,
) -> Result<(), GmicError> {
    let Some(command) = tokens.first().map(|s| s.trim_start_matches('-')) else {
        return Ok(());
    };
    if command == "fx_linify" && width.max(height) > LINIFY_MAX_EDGE {
        return Err(GmicError::UnsupportedForImageSize {
            command: command.to_string(),
            width,
            height,
            max_edge: LINIFY_MAX_EDGE,
        });
    }
    Ok(())
}

/// Token-based variant of [`build_argv`]: every element of `tokens`
/// becomes its own argv entry so `chosen.args` with spaces in a single
/// parameter (text fields!) survive intact across the subprocess
/// boundary. Same length / size caps as the file-based path.
pub(crate) fn build_argv_from_tokens(
    input: &Path,
    output: &Path,
    tokens: &[String],
    output_planes: u32,
) -> Result<Vec<OsString>, GmicError> {
    if tokens.is_empty() {
        return Err(GmicError::InvalidCharsInConfig);
    }
    if tokens.len() > MAX_FILTER_ARGS {
        return Err(GmicError::TooManyArgs(tokens.len()));
    }
    let mut args: Vec<OsString> = Vec::with_capacity(tokens.len() + 3);
    args.push(input.as_os_str().to_owned());
    for tok in tokens {
        if tok.len() > MAX_ARG_BYTES {
            return Err(GmicError::ArgTooLong(tok.len()));
        }
        args.push(OsString::from(tok));
    }
    append_output_args(&mut args, output, output_planes)?;
    Ok(args)
}

fn append_output_args(
    args: &mut Vec<OsString>,
    output: &Path,
    output_planes: u32,
) -> Result<(), GmicError> {
    args.push(OsString::from(output_color_mode_command(output_planes)?));
    args.push(OsString::from("-output"));
    args.push(output.as_os_str().to_owned());
    Ok(())
}

fn output_color_mode_command(output_planes: u32) -> Result<&'static str, GmicError> {
    match output_planes {
        1 => Ok("-to_gray"),
        3 => Ok("-to_rgb"),
        4 => Ok("-to_rgba"),
        _ => Err(GmicError::Tiff(TiffError::UnsupportedPlanes(output_planes))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn default_filter_passes_validation() {
        validate_filter_string(DEFAULT_FILTER).unwrap();
    }

    #[test]
    fn rejects_nul_in_config() {
        assert!(matches!(
            validate_filter_string("blur 3\0extra"),
            Err(GmicError::InvalidCharsInConfig)
        ));
    }

    #[test]
    fn rejects_escape_in_config() {
        assert!(matches!(
            validate_filter_string("blur 3 \x1b[31mred"),
            Err(GmicError::InvalidCharsInConfig)
        ));
    }

    #[test]
    fn allows_whitespace_in_config() {
        validate_filter_string("blur 3\nblur 5\tblur 7\r\n").unwrap();
    }

    #[test]
    fn argv_layout_is_input_filter_output() {
        let in_p = PathBuf::from("/tmp/a/in.tif");
        let out_p = PathBuf::from("/tmp/a/out.tif");
        let argv = build_argv(&in_p, &out_p, "-blur 3 -sharpen 2", 4).unwrap();
        let strs: Vec<&str> = argv.iter().map(|s| s.to_str().unwrap()).collect();
        assert_eq!(
            strs,
            vec![
                "/tmp/a/in.tif",
                "-blur",
                "3",
                "-sharpen",
                "2",
                "-to_rgba",
                "-output",
                "/tmp/a/out.tif",
            ]
        );
    }

    #[test]
    fn argv_collapses_extra_whitespace() {
        let argv = build_argv(Path::new("a"), Path::new("b"), "  -blur   3   ", 3).unwrap();
        let strs: Vec<&str> = argv.iter().map(|s| s.to_str().unwrap()).collect();
        assert_eq!(strs, vec!["a", "-blur", "3", "-to_rgb", "-output", "b"]);
    }

    #[test]
    fn argv_rejects_too_many_args() {
        let many = (0..MAX_FILTER_ARGS + 5)
            .map(|i| format!("-x{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(matches!(
            build_argv(Path::new("a"), Path::new("b"), &many, 4),
            Err(GmicError::TooManyArgs(_))
        ));
    }

    #[test]
    fn argv_rejects_arg_too_long() {
        let big = "x".repeat(MAX_ARG_BYTES + 1);
        assert!(matches!(
            build_argv(Path::new("a"), Path::new("b"), &big, 4),
            Err(GmicError::ArgTooLong(_))
        ));
    }

    #[test]
    fn argv_rejects_unsupported_output_planes() {
        assert!(matches!(
            build_argv(Path::new("a"), Path::new("b"), "-blur 3", 2),
            Err(GmicError::Tiff(TiffError::UnsupportedPlanes(2)))
        ));
    }

    #[test]
    fn linify_is_rejected_on_large_images() {
        let tokens = vec!["-fx_linify".to_string(), "40,2,40,10,24,0,0".to_string()];
        assert!(matches!(
            reject_known_expensive_filter(&tokens, 6000, 4000),
            Err(GmicError::UnsupportedForImageSize {
                command,
                width: 6000,
                height: 4000,
                max_edge: LINIFY_MAX_EDGE,
            }) if command == "fx_linify"
        ));
    }

    #[test]
    fn linify_is_allowed_on_small_images() {
        let tokens = vec!["-fx_linify".to_string(), "40,2,40,10,24,0,0".to_string()];
        reject_known_expensive_filter(&tokens, 1024, 768).unwrap();
    }

    #[test]
    fn subprocess_timeout_kills_child() {
        let dir = tempfile::tempdir().unwrap();
        let args = vec![OsString::from("-c"), OsString::from("while :; do :; done")];
        let err = run_subprocess_with_timeout(
            Path::new("/bin/sh"),
            &args,
            dir.path(),
            Duration::from_millis(50),
        )
        .unwrap_err();
        assert!(matches!(err, GmicError::TimedOut { .. }));
    }

    #[test]
    #[ignore = "requires Homebrew gmic at /opt/homebrew/bin or /usr/local/bin"]
    fn locate_gmic_finds_homebrew_install() {
        let p = locate_gmic().expect("gmic not found");
        assert!(p.exists());
        assert!(p.metadata().unwrap().permissions().mode() & 0o111 != 0);
    }

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
            vec![
                "/tmp/a/in.tif",
                "-fx_oldphoto",
                "1",
                "-to_rgb",
                "-output",
                "/tmp/a/out.png"
            ],
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

    #[test]
    fn argv_from_tokens_quotes_each_arg_separately() {
        let argv = build_argv_from_tokens(
            Path::new("/in.tif"),
            Path::new("/out.tif"),
            &[
                "-blur".to_string(),
                "3 5".to_string(), // contains whitespace; must not be re-split
                "-sharpen".to_string(),
            ],
            1,
        )
        .unwrap();
        let strs: Vec<&str> = argv.iter().map(|s| s.to_str().unwrap()).collect();
        assert_eq!(
            strs,
            vec!["/in.tif", "-blur", "3 5", "-sharpen", "-to_gray", "-output", "/out.tif"]
        );
    }

    #[test]
    fn argv_from_tokens_rejects_too_many() {
        let many: Vec<String> = (0..MAX_FILTER_ARGS + 5).map(|i| i.to_string()).collect();
        assert!(matches!(
            build_argv_from_tokens(Path::new("a"), Path::new("b"), &many, 4),
            Err(GmicError::TooManyArgs(_))
        ));
    }

    #[test]
    fn argv_from_tokens_rejects_oversized() {
        let big = "x".repeat(MAX_ARG_BYTES + 1);
        assert!(matches!(
            build_argv_from_tokens(Path::new("a"), Path::new("b"), &["-blur".into(), big], 4),
            Err(GmicError::ArgTooLong(_))
        ));
    }

    #[test]
    #[ignore = "requires Homebrew gmic at /opt/homebrew/bin or /usr/local/bin"]
    fn fx_ghost_rgba_output_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let in_path = dir.path().join("in.tif");
        let out_path = dir.path().join("out.tif");
        let (w, h, planes) = (64_u32, 64_u32, 4_u32);
        let row_bytes = w * planes;
        let mut input = vec![0_u8; (row_bytes * h) as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let i = y * row_bytes as usize + x * planes as usize;
                input[i] = (x * 3) as u8;
                input[i + 1] = (y * 3) as u8;
                input[i + 2] = ((x + y) * 2) as u8;
                input[i + 3] = 255;
            }
        }

        crate::tiff_io::write_tiff(&in_path, &input, w, h, planes, row_bytes).unwrap();
        let argv = build_argv_from_tokens(
            &in_path,
            &out_path,
            &["-fx_ghost".into(), "200,2,2,1,3,16,0".into()],
            planes,
        )
        .unwrap();
        run_subprocess(&locate_gmic().unwrap(), &argv, dir.path()).unwrap();

        let mut output = vec![0_u8; input.len()];
        crate::tiff_io::read_tiff(&out_path, &mut output, w, h, planes, row_bytes).unwrap();
        assert!(
            output.iter().any(|&b| b != 0),
            "fx_ghost output should decode into RGBA bytes"
        );
    }

    #[test]
    fn resolve_output_path_returns_exact_when_exists() {
        let dir = tempfile::tempdir().unwrap();
        let exact = dir.path().join("out.tif");
        std::fs::write(&exact, b"fake").unwrap();

        let result = resolve_output_path(&exact);
        assert_eq!(result, Some(exact));
    }

    #[test]
    fn resolve_output_path_returns_frame0_when_exact_missing() {
        let dir = tempfile::tempdir().unwrap();
        let exact = dir.path().join("out.tif");
        let frame0 = dir.path().join("out_000000.tif");
        std::fs::write(&frame0, b"fake").unwrap();

        let result = resolve_output_path(&exact);
        assert_eq!(result, Some(frame0));
    }

    #[test]
    fn resolve_output_path_returns_none_when_neither_exists() {
        let dir = tempfile::tempdir().unwrap();
        let exact = dir.path().join("out.tif");

        let result = resolve_output_path(&exact);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_output_path_handles_no_extension() {
        let dir = tempfile::tempdir().unwrap();
        let exact = dir.path().join("out");
        let frame0 = dir.path().join("out_000000");
        std::fs::write(&frame0, b"fake").unwrap();

        let result = resolve_output_path(&exact);
        assert_eq!(result, Some(frame0));
    }

    #[test]
    #[ignore = "requires gmic"]
    fn multi_output_filter_promotes_frame0() {
        use std::io::Write;
        let gmic = locate_gmic().expect("gmic installed for this test");
        let dir = tempfile::tempdir().unwrap();
        // Create a tiny input image with gmic itself
        let input = dir.path().join("in.tif");
        let seed_argv: Vec<std::ffi::OsString> = vec![
            "64,64,1,3".into(),
            "-to_rgb".into(),
            "-output".into(),
            input.clone().into_os_string(),
        ];
        run_subprocess(&gmic, &seed_argv, dir.path()).unwrap();
        let _ = &mut std::io::stderr().flush();

        // Run a multi-output filter (cl_colorWheel produces 2 images)
        let output = dir.path().join("out.tif");
        let tokens = filter_tokens("cl_colorWheel", &[]);
        render_with_tokens(&gmic, &input, &output, &tokens, 3, dir.path()).unwrap();

        // After the fix, render_with_tokens should have promoted frame 0
        // to the expected output path
        assert!(
            output.exists(),
            "output path should exist after multi-output filter"
        );
        assert!(std::fs::metadata(&output).unwrap().len() > 0);
    }
}

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

    #[test]
    fn run_filter_with_rejects_control_chars() {
        let chosen = ChosenFilter {
            command: "fx".into(),
            args: vec!["x\x1by".into()],
        };
        let mut fr = unsafe { std::mem::zeroed() };
        let err = run_filter_with(&mut fr, &chosen).err();
        assert!(matches!(err, Some(GmicError::InvalidCharsInConfig)));
    }

    /// Reproduces the Sunday-evening Affinity round-trip failure:
    /// every parameter value was sent as its own process argv,
    /// causing fx_paint_with_brush to fail with
    ///   Unknown command or filename '1-35'.
    /// because gmic could not find its declared parameter list.
    /// `build_argv_for_chosen_unit` simulates the inner joining
    /// step so we can assert the exact argv shape without exec'ing
    /// gmic from a unit test.
    #[test]
    fn chosen_args_collapse_into_one_comma_joined_token() {
        let tokens = build_tokens_for_test(&ChosenFilter {
            command: "fx_paint_with_brush".into(),
            args: (1..=35).map(|i| i.to_string()).collect(),
        });
        assert_eq!(tokens.len(), 2, "got {tokens:?}");
        assert_eq!(tokens[0], "-fx_paint_with_brush");
        assert_eq!(
            tokens[1],
            "1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,\
             21,22,23,24,25,26,27,28,29,30,31,32,33,34,35"
        );
    }

    #[test]
    fn chosen_command_with_leading_dash_is_preserved() {
        let tokens = build_tokens_for_test(&ChosenFilter {
            command: "-blur".into(),
            args: vec!["3".into()],
        });
        assert_eq!(tokens[0], "-blur");
    }

    #[test]
    fn chosen_text_value_with_comma_is_quoted() {
        // Our `point(...)` parser stores the default as "x,y" — the
        // join step has to quote it so the embedded comma doesn't
        // become a parameter separator.
        let tokens = build_tokens_for_test(&ChosenFilter {
            command: "iain_auto_wb".into(),
            args: vec!["5,5".into(), "95,95".into(), "1".into(), "0".into()],
        });
        assert_eq!(tokens[1], "\"5,5\",\"95,95\",1,0");
    }

    #[test]
    fn chosen_text_value_with_embedded_quote_is_escaped() {
        let tokens = build_tokens_for_test(&ChosenFilter {
            command: "fx".into(),
            args: vec!["he said \"hi\"".into()],
        });
        // Quoted because of the embedded ", with the " escaped.
        assert_eq!(tokens[1], "\"he said \\\"hi\\\"\"");
    }

    #[test]
    fn chosen_no_args_emits_only_command_token() {
        let tokens = build_tokens_for_test(&ChosenFilter {
            command: "fx_drama".into(),
            args: vec![],
        });
        assert_eq!(tokens, vec!["-fx_drama"]);
    }

    /// Helper that runs the same join+quote logic as
    /// `run_filter_with` but without touching FilterRecord or
    /// spawning gmic, so we can unit-test the argv shape headlessly.
    fn build_tokens_for_test(chosen: &ChosenFilter) -> Vec<String> {
        let command = if chosen.command.starts_with('-') {
            chosen.command.clone()
        } else {
            format!("-{}", chosen.command)
        };
        let mut tokens = vec![command];
        if !chosen.args.is_empty() {
            tokens.push(
                chosen
                    .args
                    .iter()
                    .map(|a| super::quote_gmic_arg(a))
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        tokens
    }

    #[test]
    fn run_filter_with_allows_tabs_and_newlines() {
        // Argument validation should not reject benign whitespace
        // (multi-line text fields). The actual command execution path
        // is exercised by integration tests with a live gmic binary.
        let chosen = ChosenFilter {
            command: "fx".into(),
            args: vec!["first line\nsecond line\twith tab".into()],
        };
        // Don't `.unwrap()` — there is no gmic to actually run; we
        // just assert the error is not InvalidCharsInConfig.
        let mut fr = unsafe { std::mem::zeroed() };
        let err = run_filter_with(&mut fr, &chosen).err();
        assert!(!matches!(err, Some(GmicError::InvalidCharsInConfig)));
    }
}
