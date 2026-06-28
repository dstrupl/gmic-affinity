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
#[allow(dead_code)]
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
#[allow(dead_code)]
pub(crate) fn preview_path_for(command: &str) -> Option<PathBuf> {
    let p = previews_dir()?.join(format!("{}.png", sanitise_key(command)));
    p.exists().then_some(p)
}

#[allow(dead_code)]
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
