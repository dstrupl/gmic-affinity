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
use std::process::Command;

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
    Failed { status: Option<i32> },
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
/// Format: `[<input.tif>, <filter tokens...>, -output, <output.tif>]`.
///
/// We deliberately *don't* try to force the output pixel type via the
/// `,uchar` (or any other) suffix on `-output`. The set of accepted
/// type names in `output_tiff` is undocumented across gmic versions
/// (3.7.6 verbose-logs the value back but then rejects 'uchar' at
/// write time, for instance), so the only portable choice is to let
/// gmic write whatever it wants and convert in `tiff_io::read_tiff`.
/// That also means our pipeline transparently works with any future
/// gmic filter, not just the ones that happen to stay in 8-bit.
pub fn build_argv(
    input: &Path,
    output: &Path,
    filter_cmd: &str,
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

    args.push(OsString::from("-output"));
    args.push(output.as_os_str().to_owned());
    Ok(args)
}

/// Execute gmic with a cleared environment, hiding its stdout/stderr so we
/// don't pollute Console.app on success. On failure we surface the exit
/// status so the caller can log it.
pub fn run_subprocess(gmic: &Path, args: &[OsString], tmpdir: &Path) -> Result<(), GmicError> {
    log(&format!(
        "spawn {} (argc={}) tmpdir={}",
        gmic.display(),
        args.len(),
        tmpdir.display()
    ));
    let output = Command::new(gmic)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", tmpdir)
        .env("TMPDIR", tmpdir)
        .env("LANG", "C")
        .output()?;
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
    if !output.status.success() {
        return Err(GmicError::Failed {
            status: output.status.code(),
        });
    }
    Ok(())
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
    // gmic-qt-style invocation: the command, then each parameter as
    // its own argv entry. `build_argv` (legacy) joined tokens with
    // whitespace, which is not what the picker produces; here we
    // bypass that path entirely.
    let mut tokens: Vec<String> = Vec::with_capacity(chosen.args.len() + 1);
    let command = if chosen.command.starts_with('-') {
        chosen.command.clone()
    } else {
        format!("-{}", chosen.command)
    };
    tokens.push(command);
    for arg in &chosen.args {
        tokens.push(arg.clone());
    }
    run_with_tokens(fr, &tokens)
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

    let argv = build_argv_from_tokens(&in_path, &out_path, tokens)?;
    run_subprocess(&gmic, &argv, dir.path())?;

    read_tiff(
        &out_path,
        out_slice,
        buf.width as u32,
        buf.height as u32,
        buf.planes as u32,
        buf.out_row_bytes as u32,
    )?;

    Ok(())
}

/// Token-based variant of [`build_argv`]: every element of `tokens`
/// becomes its own argv entry so `chosen.args` with spaces in a single
/// parameter (text fields!) survive intact across the subprocess
/// boundary. Same length / size caps as the file-based path.
fn build_argv_from_tokens(
    input: &Path,
    output: &Path,
    tokens: &[String],
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
    args.push(OsString::from("-output"));
    args.push(output.as_os_str().to_owned());
    Ok(args)
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
        let argv = build_argv(&in_p, &out_p, "-blur 3 -sharpen 2").unwrap();
        let strs: Vec<&str> = argv.iter().map(|s| s.to_str().unwrap()).collect();
        assert_eq!(
            strs,
            vec![
                "/tmp/a/in.tif",
                "-blur",
                "3",
                "-sharpen",
                "2",
                "-output",
                "/tmp/a/out.tif",
            ]
        );
    }

    #[test]
    fn argv_collapses_extra_whitespace() {
        let argv = build_argv(Path::new("a"), Path::new("b"), "  -blur   3   ").unwrap();
        let strs: Vec<&str> = argv.iter().map(|s| s.to_str().unwrap()).collect();
        assert_eq!(strs, vec!["a", "-blur", "3", "-output", "b"]);
    }

    #[test]
    fn argv_rejects_too_many_args() {
        let many = (0..MAX_FILTER_ARGS + 5)
            .map(|i| format!("-x{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(matches!(
            build_argv(Path::new("a"), Path::new("b"), &many),
            Err(GmicError::TooManyArgs(_))
        ));
    }

    #[test]
    fn argv_rejects_arg_too_long() {
        let big = "x".repeat(MAX_ARG_BYTES + 1);
        assert!(matches!(
            build_argv(Path::new("a"), Path::new("b"), &big),
            Err(GmicError::ArgTooLong(_))
        ));
    }

    #[test]
    #[ignore = "requires Homebrew gmic at /opt/homebrew/bin or /usr/local/bin"]
    fn locate_gmic_finds_homebrew_install() {
        let p = locate_gmic().expect("gmic not found");
        assert!(p.exists());
        assert!(p.metadata().unwrap().permissions().mode() & 0o111 != 0);
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
        )
        .unwrap();
        let strs: Vec<&str> = argv.iter().map(|s| s.to_str().unwrap()).collect();
        assert_eq!(
            strs,
            vec!["/in.tif", "-blur", "3 5", "-sharpen", "-output", "/out.tif"]
        );
    }

    #[test]
    fn argv_from_tokens_rejects_too_many() {
        let many: Vec<String> = (0..MAX_FILTER_ARGS + 5).map(|i| i.to_string()).collect();
        assert!(matches!(
            build_argv_from_tokens(Path::new("a"), Path::new("b"), &many),
            Err(GmicError::TooManyArgs(_))
        ));
    }

    #[test]
    fn argv_from_tokens_rejects_oversized() {
        let big = "x".repeat(MAX_ARG_BYTES + 1);
        assert!(matches!(
            build_argv_from_tokens(Path::new("a"), Path::new("b"), &["-blur".into(), big]),
            Err(GmicError::ArgTooLong(_))
        ));
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
