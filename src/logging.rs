//! Side-channel file logger.
//!
//! Affinity Photo 2 runs plugins in-process but redirects stderr to its own
//! sinks, so `eprintln!` output never reaches Console.app and `log show`.
//! That made it impossible to tell whether `PluginMain` was being called
//! at all during early bring-up. To remove that ambiguity every code path
//! that matters writes a one-line record to a known file:
//!
//! ```text
//! ~/Library/Logs/gmic-affinity.log
//! ```
//!
//! Format: `<ISO-8601 UTC> pid=<pid> <message>\n`
//!
//! Implementation notes:
//! - The file is opened append/create on every call (no global state, so
//!   plugin unload/reload is safe). Open failures are silently swallowed
//!   because logging must never bring the host down.
//! - We use `O_APPEND` so concurrent writes from any background Affinity
//!   thread interleave at line boundaries (`write(2)` of <= PIPE_BUF is
//!   atomic on macOS for regular files too).
//! - There is no rotation; the file is expected to stay small (a handful
//!   of lines per invocation). A user who runs the plugin thousands of
//!   times can simply `rm` it.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Resolve the destination log path, honouring `$HOME` like the rest of
/// the crate. Returns `None` if `$HOME` is unset (in which case logging
/// becomes a no-op rather than writing somewhere unexpected).
fn log_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Logs/gmic-affinity.log"))
}

/// Best-effort write of one log line. Never panics, never propagates I/O
/// errors. Safe to call from `PluginMain` and from `Drop` paths.
pub fn log(msg: &str) {
    let _ = (|| -> std::io::Result<()> {
        let path = log_path()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME not set"))?;
        if let Some(parent) = path.parent() {
            // Best-effort directory creation; usually exists already.
            let _ = std::fs::create_dir_all(parent);
        }
        let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
        let line = format!("{} pid={} {}\n", iso8601_now(), std::process::id(), msg);
        f.write_all(line.as_bytes())?;
        Ok(())
    })();
}

/// Tiny RFC3339 / ISO-8601 UTC formatter (`YYYY-MM-DDTHH:MM:SS.mmmZ`).
/// We don't pull `chrono` in just for this; the standard library gives us
/// seconds since the epoch and we do the calendar math inline. The output
/// is grep-friendly and sortable.
///
/// Exposed publicly so `PluginMain`'s settings-recording path (T11)
/// can timestamp the user's pick with the same format used in log
/// lines, keeping the file format and log file trivially correlatable.
pub fn iso8601_now() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let millis = now.subsec_millis();
    let (year, month, day, hour, minute, second) = civil_from_unix(secs);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, month, day, hour, minute, second, millis
    )
}

/// Convert seconds-since-1970 (UTC) to (year, month, day, hour, minute,
/// second). Based on Howard Hinnant's `days_from_civil` algorithm, which
/// is the standard inverse and handles leap years up to year 9999 without
/// branches. We accept any post-epoch value (pre-epoch returns zeros).
fn civil_from_unix(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    if secs < 0 {
        return (1970, 1, 1, 0, 0, 0);
    }
    let days = secs / 86_400;
    let time_of_day = (secs % 86_400) as u32;
    let hour = time_of_day / 3600;
    let minute = (time_of_day % 3600) / 60;
    let second = time_of_day % 60;

    // Howard Hinnant's algorithm, days since 1970-01-01.
    let z = days + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = (y + if m <= 2 { 1 } else { 0 }) as i32;

    (year, m as u32, d as u32, hour, minute, second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_known_dates() {
        // A handful of values cross-checked against
        //   `date -u -j -f '%Y-%m-%dT%H:%M:%SZ' '<iso>' +%s`
        // so we don't trust ourselves on the arithmetic.
        for &(secs, expected) in &[
            (1_704_067_200_i64, (2024, 1, 1, 0, 0, 0)), // 2024-01-01
            (1_735_689_600_i64, (2025, 1, 1, 0, 0, 0)), // 2025-01-01
            (1_709_251_200_i64, (2024, 3, 1, 0, 0, 0)), // day after leap
            (1_767_225_600_i64, (2026, 1, 1, 0, 0, 0)), // 2026-01-01
            (0_i64, (1970, 1, 1, 0, 0, 0)),             // epoch
            (1_577_836_799_i64, (2019, 12, 31, 23, 59, 59)),
        ] {
            assert_eq!(civil_from_unix(secs), expected, "secs={secs}");
        }
    }

    #[test]
    fn pre_epoch_is_clamped() {
        assert_eq!(civil_from_unix(-1), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn leap_day_is_correct() {
        // 2024-02-29T12:34:56Z is 1_709_210_096.
        let (y, mo, d, h, mi, s) = civil_from_unix(1_709_210_096);
        assert_eq!((y, mo, d, h, mi, s), (2024, 2, 29, 12, 34, 56));
    }

    #[test]
    fn iso_string_matches_grammar() {
        let s = iso8601_now();
        assert_eq!(s.len(), 24, "expected YYYY-MM-DDTHH:MM:SS.mmmZ, got {s}");
        assert!(s.ends_with('Z'));
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], "T");
    }
}
