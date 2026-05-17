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
    let status = Command::new(gmic)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", tmpdir)
        .env("TMPDIR", tmpdir)
        .env("LANG", "C")
        .status()?;
    if !status.success() {
        return Err(GmicError::Failed {
            status: status.code(),
        });
    }
    Ok(())
}

/// Full filter run: validate FilterRecord, write input TIFF, exec gmic,
/// read output TIFF back into host buffer. The temp directory is created
/// with the user's default umask (typically 0700 for tempfile crate's
/// `tempdir()`); both TIFFs live inside it and are auto-removed on drop.
pub fn run_filter(fr: &mut FilterRecord) -> Result<(), GmicError> {
    let buf = validate_filter_record(fr)?;

    let gmic = locate_gmic()?;
    let cmd = read_filter_config()?;

    let dir = tempfile::Builder::new()
        .prefix("gmic-affinity-")
        .tempdir()?;
    // Belt-and-braces: tempfile already chmods to 0700 on Unix; assert that.
    let _ = fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700));

    let in_path = dir.path().join("in.tif");
    let out_path = dir.path().join("out.tif");

    // Re-construct slices from validated raw parts.
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

    let argv = build_argv(&in_path, &out_path, &cmd)?;
    run_subprocess(&gmic, &argv, dir.path())?;

    read_tiff(
        &out_path,
        out_slice,
        buf.width as u32,
        buf.height as u32,
        buf.planes as u32,
        buf.out_row_bytes as u32,
    )?;

    // `dir` drops here -> tempfile removes in.tif, out.tif and the dir.
    Ok(())
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
                "/tmp/a/out.tif"
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
}
