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
