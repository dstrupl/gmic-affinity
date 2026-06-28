//! Live-only preview pane for the picker: resolves, loads, and renders
//! the pre-computed PNG for the selected filter, or a placeholder.

use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::ClassType;
use objc2_app_kit::{NSImage, NSImageScaling, NSImageView, NSTextField, NSView};
use objc2_foundation::{CGPoint, CGRect, CGSize, MainThreadMarker, NSString};

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
        NSView::initWithFrame(
            mtm.alloc(),
            CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size: CGSize {
                    width: 280.0,
                    height: 400.0,
                },
            },
        )
    };
    let image = unsafe {
        NSImageView::initWithFrame(
            mtm.alloc(),
            CGRect {
                origin: CGPoint { x: 8.0, y: 110.0 },
                size: CGSize {
                    width: 264.0,
                    height: 282.0,
                },
            },
        )
    };
    unsafe {
        image.setImageScaling(NSImageScaling::NSImageScaleProportionallyUpOrDown);
        image.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::NSViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::NSViewHeightSizable,
        );
    }
    let caption = unsafe {
        let label = NSTextField::initWithFrame(
            mtm.alloc(),
            CGRect {
                origin: CGPoint { x: 8.0, y: 8.0 },
                size: CGSize {
                    width: 264.0,
                    height: 96.0,
                },
            },
        );
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
    let view = PreviewView {
        root,
        image,
        caption,
    };
    view.show_placeholder();
    view
}

impl PreviewView {
    /// Show the preview for `command` (or a placeholder if none exists)
    /// and the filter `description` as the caption.
    pub(crate) fn show(&self, command: &str, description: Option<&str>) {
        let has_image = match preview_path_for(command) {
            Some(path) => self.set_image_from(&path),
            None => {
                unsafe { self.image.setImage(None) };
                false
            }
        };
        // Caption priority: the filter's description when it has one;
        // otherwise the "No preview available" hint, but only when there
        // is no image (a visible preview with no description shows a
        // blank caption rather than a contradictory "no preview" line).
        let caption = match description {
            Some(text) if !text.trim().is_empty() => text,
            _ if has_image => "",
            _ => "No preview available",
        };
        unsafe {
            self.caption.setStringValue(&NSString::from_str(caption));
        }
    }

    /// Load and display the PNG at `path`. Returns whether an image was
    /// actually set (false if the file failed to decode).
    fn set_image_from(&self, path: &std::path::Path) -> bool {
        let ns = NSString::from_str(&path.to_string_lossy());
        let img: Option<Retained<NSImage>> =
            unsafe { NSImage::initWithContentsOfFile(NSImage::alloc(), &ns) };
        match img {
            Some(image) => {
                unsafe { self.image.setImage(Some(&image)) };
                true
            }
            None => {
                unsafe { self.image.setImage(None) };
                false
            }
        }
    }

    fn show_placeholder(&self) {
        unsafe { self.image.setImage(None) };
        unsafe {
            self.caption
                .setStringValue(&NSString::from_str("No preview available"));
        }
    }
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
