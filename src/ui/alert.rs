//! User-facing error reporting.
//!
//! The pure-Rust core of this module — the `Sink` trait, the
//! `alert_error` helper, and the `CaptureSink` capture impl — is
//! compiled in every build so that PluginMain's error-handling
//! matrix can be table-tested without ever opening a window.
//!
//! The AppKit `NSAlert` backend lives in the `nsalert` submodule and
//! is only compiled under `--features live`, where the rest of the
//! `ui` module (picker, runloop, modal-close delegate) is wired up.

use std::sync::Mutex;

pub trait Sink: Send + Sync {
    fn display(&self, title: &str, message: &str);
}

/// Append the standard log-hint footer to a message when appropriate.
fn with_log_hint(message: &str, log_hint: bool) -> String {
    if log_hint {
        format!("{message}\n\nSee ~/Library/Logs/gmic-affinity.log for details.")
    } else {
        message.to_string()
    }
}

/// Route an error through the configured sink with an optional
/// log-file pointer footer.
pub fn alert_error(sink: &dyn Sink, title: &str, message: &str, log_hint: bool) {
    sink.display(title, &with_log_hint(message, log_hint));
}

// ---- Test / headless capture sink ----

#[derive(Default)]
pub struct CaptureSink {
    pub events: Mutex<Vec<(String, String)>>,
}

impl Sink for CaptureSink {
    fn display(&self, title: &str, message: &str) {
        self.events
            .lock()
            .unwrap()
            .push((title.to_string(), message.to_string()));
    }
}

// ---- Production NSAlert backend (live builds only) ----

#[cfg(feature = "live")]
mod nsalert {
    use super::Sink;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSAlert, NSAlertStyle};
    use objc2_foundation::{MainThreadMarker, NSString};

    pub struct NsAlertSink;

    impl Sink for NsAlertSink {
        fn display(&self, title: &str, message: &str) {
            let Some(mtm) = MainThreadMarker::new() else {
                // Off the main thread: log instead of crashing. Should
                // never happen from PluginMain but worth defending.
                crate::logging::log(&format!(
                    "alert: not on main thread; would have shown title={title:?}, message={message:?}",
                ));
                return;
            };
            unsafe {
                let alert: Retained<NSAlert> = NSAlert::new(mtm);
                let title_ns = NSString::from_str(title);
                let message_ns = NSString::from_str(message);
                alert.setMessageText(&title_ns);
                alert.setInformativeText(&message_ns);
                alert.setAlertStyle(NSAlertStyle::Warning);
                let _ = alert.runModal();
            }
        }
    }
}

#[cfg(feature = "live")]
pub use nsalert::NsAlertSink;

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
