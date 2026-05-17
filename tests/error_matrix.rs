//! Drive the picker's error-handling matrix (design §5.3) without
//! involving AppKit or a live `FilterRecord`. We can't call
//! `PluginMain` directly without a real Photoshop host setting up its
//! own `*data` slot and rect / row-bytes pointers, but the alert
//! messages are produced by the same [`alert_error`] helper used in
//! `src/lib.rs`, so testing those formatting paths here against a
//! [`CaptureSink`] guards the entire matrix.

use GmicFilter::ui::alert::{alert_error, CaptureSink};

#[test]
fn gmic_not_found_message() {
    let sink = CaptureSink::default();
    alert_error(
        &sink,
        "G'MIC",
        "G'MIC isn't installed. Install it with: brew install gmic and try again.",
        false,
    );
    let evts = sink.events.lock().unwrap();
    assert_eq!(evts.len(), 1, "exactly one alert expected");
    assert!(
        evts[0].1.contains("brew install gmic"),
        "missing install hint: {:?}",
        evts[0].1
    );
    assert!(
        !evts[0].1.contains("gmic-affinity.log"),
        "user-error path must not append the log footer"
    );
    assert_eq!(evts[0].0, "G'MIC", "alert title is G'MIC");
}

#[test]
fn tiff_error_includes_log_hint() {
    let sink = CaptureSink::default();
    alert_error(
        &sink,
        "G'MIC",
        "G'MIC produced an image we couldn't read back.",
        true,
    );
    let evts = sink.events.lock().unwrap();
    assert!(
        evts[0].1.contains("gmic-affinity.log"),
        "diagnostic path must point user at the log file"
    );
}

#[test]
fn last_filter_empty_message() {
    let sink = CaptureSink::default();
    alert_error(
        &sink,
        "G'MIC",
        "No previous G'MIC filter to repeat. Pick one from Filters > Plugins > G'MIC > G'MIC….",
        false,
    );
    let evts = sink.events.lock().unwrap();
    assert!(
        evts[0].1.contains("Pick one"),
        "the empty-Last-Filter copy must guide the user to the menu"
    );
}

#[test]
fn picker_panic_message_includes_log_hint() {
    // PARAMETERS arm wraps `show_picker` in `catch_unwind`; when the
    // panel itself panics we emit a single non-user-actionable alert
    // with the log footer so the user can attach it to a bug report.
    let sink = CaptureSink::default();
    alert_error(&sink, "G'MIC", "Couldn't open the G'MIC dialog.", true);
    let evts = sink.events.lock().unwrap();
    assert!(evts[0].1.contains("Couldn't open the G'MIC dialog."));
    assert!(evts[0].1.contains("gmic-affinity.log"));
}

#[test]
fn corrupt_catalogue_message_includes_log_hint() {
    // PARAMETERS also wraps `catalogue::builtin()` in `catch_unwind`;
    // a panic there means the bundled snapshot is wrong, which is a
    // build-time / supply-chain issue worth pointing the user at.
    let sink = CaptureSink::default();
    alert_error(
        &sink,
        "G'MIC",
        "G'MIC filter list is unreadable — this build of the plugin may be corrupted.",
        true,
    );
    let evts = sink.events.lock().unwrap();
    assert!(evts[0].1.contains("filter list is unreadable"));
    assert!(evts[0].1.contains("gmic-affinity.log"));
}

#[test]
fn gmic_failure_includes_exit_code_and_log_hint() {
    // CONTINUE's GmicError::Failed { status } branch formats the
    // command + exit code so a user reporting a bug can attach the
    // exact gmic invocation that failed.
    let sink = CaptureSink::default();
    alert_error(
        &sink,
        "G'MIC",
        "G'MIC reported an error running `fx_painting` (exit 1).",
        true,
    );
    let evts = sink.events.lock().unwrap();
    assert!(evts[0].1.contains("fx_painting"));
    assert!(evts[0].1.contains("exit 1"));
    assert!(evts[0].1.contains("gmic-affinity.log"));
}
