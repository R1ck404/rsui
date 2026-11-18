//! What WebView2 calls back. Every handler interface it has is the same shape: `IUnknown` and then `Invoke`, given two
//! things, which are a result and what came of it, or a sender and what it says. So one object does for all of them: it
//! says it's whichever interface it was made as, and hands what `Invoke` was given to a closure, which knows which it is.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::core::{GUID, HRESULT, IUnknown, Interface};

/// What `Invoke` was given: two pointer-sized values, which a closure reads as what its interface says they are. A result
/// code comes as the low half of the first.
pub(crate) type Call = Box<dyn FnMut(usize, usize) -> HRESULT>;

#[repr(C)]
struct Vtable {
    query_interface: unsafe extern "system" fn(*mut Handler, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut Handler) -> u32,
    release: unsafe extern "system" fn(*mut Handler) -> u32,
    invoke: unsafe extern "system" fn(*mut Handler, usize, usize) -> HRESULT,
}

#[repr(C)]
struct Handler {
    vtable: &'static Vtable,
    count: AtomicU32,
    interface: GUID,
    call: Call,
}

static VTABLE: Vtable = Vtable {
    query_interface,
    add_ref,
    release,
    invoke,
};

const E_NOINTERFACE: HRESULT = HRESULT(0x8000_4002_u32 as i32);

unsafe extern "system" fn query_interface(this: *mut Handler, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: COM calls this with a handler made by `make`, and pointers that are valid for this call.
    unsafe {
        if out.is_null() || iid.is_null() {
            return E_NOINTERFACE;
        }
        let wanted = *iid;
        if wanted == (*this).interface || wanted == IUnknown::IID {
            add_ref(this);
            *out = this.cast();
            HRESULT(0)
        } else {
            *out = std::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn add_ref(this: *mut Handler) -> u32 {
    // SAFETY: `this` is a live handler.
    unsafe { (*this).count.fetch_add(1, Ordering::Relaxed) + 1 }
}

unsafe extern "system" fn release(this: *mut Handler) -> u32 {
    // SAFETY: `this` is a live handler, and is freed once nothing holds it, as it was boxed by `make`.
    unsafe {
        let left = (*this).count.fetch_sub(1, Ordering::Release) - 1;
        if left == 0 {
            std::sync::atomic::fence(Ordering::Acquire);
            drop(Box::from_raw(this));
        }
        left
    }
}

unsafe extern "system" fn invoke(this: *mut Handler, first: usize, second: usize) -> HRESULT {
    // SAFETY: `this` is a live handler, and WebView2 only calls it on the thread it was made on, one call at a time.
    unsafe { ((*this).call)(first, second) }
}

/// A handler that says it's `interface`, held once, as an `IUnknown` that lets go of it when it's dropped. What's given is
/// its raw pointer, which the method it's given to holds on to for itself.
pub(crate) struct Made(IUnknown);

impl Made {
    pub(crate) fn raw(&self) -> *mut c_void {
        self.0.as_raw()
    }
}

pub(crate) fn make(interface: GUID, call: impl FnMut(usize, usize) -> HRESULT + 'static) -> Made {
    let handler = Box::new(Handler {
        vtable: &VTABLE,
        count: AtomicU32::new(1),
        interface,
        call: Box::new(call),
    });
    // SAFETY: the handler's first field is its vtable, laid out as IUnknown's is, and its one reference is the one this
    // takes.
    Made(unsafe { IUnknown::from_raw(Box::into_raw(handler).cast()) })
}

/// The interface `raw` is, borrowed for as long as the call it came in lasts, if it's there.
///
/// # Safety
/// `raw` must be null or a live interface pointer of type `T`.
pub(crate) unsafe fn borrowed<T: Interface + Clone>(raw: usize) -> Option<T> {
    // SAFETY: as the caller says.
    unsafe { T::from_raw_borrowed(&(raw as *mut c_void)).cloned() }
}

/// What a result code given as the first of `Invoke`'s values is.
pub(crate) fn code(first: usize) -> HRESULT {
    HRESULT(first as u32 as i32)
}
