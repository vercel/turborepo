//! Adapting custom allocators to work with libghostty.
use std::{
    borrow::Borrow,
    ffi::c_void,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    ptr::NonNull,
};

#[cfg(feature = "allocator_api")]
use allocator_api2::alloc;

use crate::{
    error::{Error, Result},
    ffi,
};

/// A custom allocator that libghostty uses for its memory allocations.
///
/// The allocator may depend on some external state for the
/// duration of lifetime `'ctx`. This is useful for adapting external,
/// stateful allocators that may not have a `'static` lifetime.
///
/// One example of a custom allocator that *does* have a `'static`
/// lifetime is Rust's own default allocator, which can also be used
/// within libghostty as [`Allocator::GLOBAL`].
#[derive(Debug)]
pub struct Allocator<'ctx> {
    pub(crate) inner: ffi::Allocator,
    _phan: PhantomData<&'ctx ()>,
}

impl Allocator<'_> {
    pub(crate) fn to_raw(&self) -> *const ffi::Allocator {
        std::ptr::from_ref(&self.inner)
    }
    pub(crate) unsafe fn from_raw(raw: *const ffi::Allocator) -> Self {
        Self {
            inner: unsafe { *raw },
            _phan: PhantomData,
        }
    }
}

/// An internal helper struct for dealing with the common allocation
/// pattern of allowing custom allocators for libghostty's opaque objects.
#[derive(Debug)]
pub(crate) struct Object<'alloc, T> {
    pub(crate) ptr: NonNull<T>,
    _phan: PhantomData<&'alloc ffi::Allocator>,
}

impl<T> Object<'_, T> {
    pub(crate) fn new(raw: *mut T) -> Result<Self> {
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _phan: PhantomData,
        })
    }
    pub(crate) fn as_raw(&self) -> *mut T {
        self.ptr.as_ptr()
    }
}

/// Borrowed version of `Object`.
#[derive(Debug)]
pub(crate) struct Ref<'a, T> {
    pub(crate) ptr: NonNull<T>,
    _phan: PhantomData<&'a ()>,
}

impl<T> Ref<'_, T> {
    pub(crate) fn new(raw: *mut T) -> Result<Self> {
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self {
            ptr,
            _phan: PhantomData,
        })
    }
    pub(crate) fn as_raw(&self) -> *mut T {
        self.ptr.as_ptr()
    }
}

/// Bytes allocated by libghostty, possibly using a custom allocator.
#[derive(Debug)]
pub struct Bytes<'alloc> {
    ptr: NonNull<u8>,
    len: usize,
    alloc: *const ffi::Allocator,
    _phan: PhantomData<&'alloc ffi::Allocator>,
}
impl<'alloc> Bytes<'alloc> {
    /// Allocate `len` bytes with libghostty's default allocator.
    ///
    /// Not really useful except in very niche cases.
    pub fn new(len: usize) -> Result<Self> {
        // SAFETY: A NULL allocator is always valid
        unsafe { Self::new_inner(std::ptr::null(), len) }
    }

    /// Allocate `len` bytes with a custom allocator.
    ///
    /// Not really useful except in very niche cases.
    pub fn new_with_alloc<'ctx: 'alloc>(
        alloc: &'alloc Allocator<'ctx>,
        len: usize,
    ) -> Result<Self> {
        // SAFETY: Borrow checking should forbid invalid allocators
        unsafe { Self::new_inner(alloc.to_raw(), len) }
    }

    unsafe fn new_inner(alloc: *const ffi::Allocator, len: usize) -> Result<Self> {
        let raw = unsafe { ffi::ghostty_alloc(alloc, len) };
        let ptr = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(unsafe { Self::from_raw_parts(ptr, len, alloc) })
    }

    pub(crate) unsafe fn from_raw_parts(
        ptr: NonNull<u8>,
        len: usize,
        alloc: *const ffi::Allocator,
    ) -> Self {
        Self {
            ptr,
            len,
            alloc,
            _phan: PhantomData,
        }
    }
}
impl Drop for Bytes<'_> {
    fn drop(&mut self) {
        // SAFETY: The lifetime dictates that the allocator must
        // remain valid through here. We retain ownership of the bytes
        // memory itself so it should not be freed beforehand.
        unsafe { ffi::ghostty_free(self.alloc, self.ptr.as_ptr(), self.len) };
    }
}
impl Deref for Bytes<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: See Drop
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}
impl DerefMut for Bytes<'_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: See Drop
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}
impl AsRef<[u8]> for Bytes<'_> {
    fn as_ref(&self) -> &[u8] {
        self
    }
}
impl AsMut<[u8]> for Bytes<'_> {
    fn as_mut(&mut self) -> &mut [u8] {
        self
    }
}
impl Borrow<[u8]> for Bytes<'_> {
    fn borrow(&self) -> &[u8] {
        self
    }
}
impl<'a> IntoIterator for &'a Bytes<'_> {
    type Item = &'a u8;
    type IntoIter = std::slice::Iter<'a, u8>;

    fn into_iter(self) -> Self::IntoIter {
        self.deref().iter()
    }
}

//------------------------------------
// GlobalAlloc
//------------------------------------

impl Allocator<'static> {
    /// A custom allocator based on Rust's built-in
    /// [global allocator](std::alloc::GlobalAlloc).
    pub const GLOBAL: Self = Self {
        inner: ffi::Allocator {
            ctx: std::ptr::null_mut(),
            vtable: &ffi::AllocatorVtable {
                alloc: Some(_global_alloc),
                free: Some(_global_free),
                resize: Some(_global_resize),
                remap: Some(_global_remap),
            },
        },
        _phan: PhantomData,
    };
}

unsafe extern "C" fn _global_alloc(
    _allocator: *mut c_void,
    len: usize,
    alignment: u8,
    _ret_addr: usize,
) -> *mut c_void {
    let Ok(layout) = std::alloc::Layout::from_size_align(len, 1 << alignment) else {
        return std::ptr::null_mut();
    };
    unsafe { std::alloc::alloc(layout).cast::<c_void>() }
}

unsafe extern "C" fn _global_free(
    _allocator: *mut c_void,
    mem: *mut c_void,
    len: usize,
    alignment: u8,
    _ret_addr: usize,
) {
    let Ok(layout) = std::alloc::Layout::from_size_align(len, 1 << alignment) else {
        return;
    };
    unsafe { std::alloc::dealloc(mem.cast::<u8>(), layout) }
}
unsafe extern "C" fn _global_resize(
    _allocator: *mut c_void,
    _mem: *mut c_void,
    _old_len: usize,
    _alignment: u8,
    _new_len: usize,
    _ret_addr: usize,
) -> bool {
    false
}
unsafe extern "C" fn _global_remap(
    _allocator: *mut c_void,
    mem: *mut c_void,
    old_len: usize,
    alignment: u8,
    new_len: usize,
    _ret_addr: usize,
) -> *mut c_void {
    let Ok(layout) = std::alloc::Layout::from_size_align(old_len, 1 << alignment) else {
        return std::ptr::null_mut();
    };
    unsafe { std::alloc::realloc(mem.cast::<u8>(), layout, new_len).cast::<c_void>() }
}

//------------------------------------
// Allocator API
//------------------------------------

/// Adapt a Rust Allocator into a libghostty Allocator.
#[cfg(feature = "allocator_api")]
impl<'ctx, A: alloc::Allocator + 'ctx> From<A> for Allocator<'ctx> {
    fn from(value: A) -> Self {
        Self {
            inner: ffi::Allocator {
                ctx: std::ptr::from_ref(value.by_ref()) as *mut std::ffi::c_void,
                vtable: &ffi::AllocatorVtable {
                    alloc: Some(_alloc::<A>),
                    free: Some(_free::<A>),
                    resize: Some(_resize),
                    remap: Some(_remap::<A>),
                },
            },
            _phan: PhantomData,
        }
    }
}

#[cfg(feature = "allocator_api")]
unsafe extern "C" fn _alloc<A: alloc::Allocator>(
    allocator: *mut c_void,
    len: usize,
    alignment: u8,
    _ret_addr: usize,
) -> *mut c_void {
    let layout = alloc::Layout::from_size_align(len, 1 << alignment).ok();

    unsafe { get_allocator::<A>(allocator) }
        .and_then(|alloc| alloc.allocate(layout?).ok())
        .map(|p| p.as_ptr().cast::<c_void>())
        .unwrap_or(std::ptr::null_mut())
}

#[cfg(feature = "allocator_api")]
unsafe extern "C" fn _free<A: alloc::Allocator>(
    allocator: *mut c_void,
    mem: *mut c_void,
    len: usize,
    alignment: u8,
    _ret_addr: usize,
) {
    let Some(mem) = NonNull::new(mem.cast::<u8>()) else {
        return;
    };
    let Some(layout) = alloc::Layout::from_size_align(len, 1 << alignment).ok() else {
        return;
    };
    if let Some(alloc) = unsafe { get_allocator::<A>(allocator) } {
        unsafe { alloc.deallocate(mem, layout) };
    }
}

/// Resize (grow or shrink) an allocation *in-place*.
///
/// Rather unfortunately, Rust's Allocator API does not guarantee that
/// growing or shrinking an allocation would necessarily be in-place.
/// Therefore, we have to assume rather pessimistically that every
/// resizing operation might relocate the memory block, so in-place
/// resizes are always impossible.
#[cfg(feature = "allocator_api")]
unsafe extern "C" fn _resize(
    _allocator: *mut c_void,
    _mem: *mut c_void,
    _old_len: usize,
    _alignment: u8,
    _new_len: usize,
    _ret_addr: usize,
) -> bool {
    false
}

/// Resize (grow or shrink) an allocation, *allowing relocation if necessary*,
/// returning `null` if resizing requires reallocation.
#[cfg(feature = "allocator_api")]
unsafe extern "C" fn _remap<A: alloc::Allocator>(
    allocator: *mut c_void,
    mem: *mut c_void,
    old_len: usize,
    alignment: u8,
    new_len: usize,
    _ret_addr: usize,
) -> *mut c_void {
    let mem = NonNull::new(mem.cast::<u8>());
    let old_layout = alloc::Layout::from_size_align(old_len, 1 << alignment).ok();
    let new_layout = alloc::Layout::from_size_align(new_len, 1 << alignment).ok();

    unsafe { get_allocator::<A>(allocator) }
        .and_then(|alloc| {
            if new_len < old_len {
                unsafe { alloc.shrink(mem?, old_layout?, new_layout?) }.ok()
            } else {
                unsafe { alloc.grow(mem?, old_layout?, new_layout?) }.ok()
            }
        })
        .map(|p| p.as_ptr().cast::<c_void>())
        .unwrap_or(std::ptr::null_mut())
}

/// Get the allocator back from a vtable function.
///
/// # Safety
///
/// This function only behaves correctly if called by one of the vtable functions.
/// In particular, it expects the vtable function to be used correctly, which means
/// libghostty must have received a valid allocator object from elsewhere in this
/// crate. If any of these preconditions are unmet, this will definitely cause
/// Undefined Behavior.
///
/// The returned allocator must **never** be smuggled outside the lifetime of the caller.
#[inline(always)]
#[cfg(feature = "allocator_api")]
unsafe fn get_allocator<'a, A: alloc::Allocator>(ptr: *mut c_void) -> Option<&'a A> {
    unsafe { ptr.cast::<A>().as_ref() }
}
