#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(clippy::all)]
#![allow(rustdoc::all)]

mod bindings;

use std::ops::Deref;

pub use bindings::*;

/// Initialize a "sized" FFI object.
#[macro_export]
macro_rules! sized {
    ($ty:ty) => {{
        let mut t = <$ty as ::std::default::Default>::default();
        t.size = ::std::mem::size_of::<$ty>();
        t
    }};
}

impl<S> From<S> for bindings::String
where
    S: Deref<Target = str>,
{
    fn from(value: S) -> Self {
        Self {
            ptr: value.as_ptr(),
            len: value.len(),
        }
    }
}

impl bindings::String {
    /// Borrow the string as raw bytes. A null pointer is treated as an empty
    /// string; the C API does not promise a non-null pointer for zero-length
    /// strings, and `slice::from_raw_parts` requires one even at length zero.
    ///
    /// # Safety
    ///
    /// The caller must uphold that the associated lifetime is valid
    /// with the given context behind the FFI string.
    pub unsafe fn to_bytes<'a>(self) -> &'a [u8] {
        if self.ptr.is_null() {
            return &[];
        }
        // SAFETY: To be upheld by caller
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// # Safety
    ///
    /// The caller must uphold that the associated lifetime is valid
    /// with the given context behind the FFI string, and that it contains
    /// valid UTF-8 data.
    pub unsafe fn to_str<'a>(self) -> &'a str {
        // SAFETY: To be upheld by caller
        unsafe { std::str::from_utf8_unchecked(self.to_bytes()) }
    }
}

#[cfg(test)]
mod tests {
    use super::bindings;

    #[test]
    fn sized_macro_sets_the_size_field() {
        let colors = crate::sized!(bindings::RenderStateColors);
        assert_eq!(
            colors.size,
            std::mem::size_of::<bindings::RenderStateColors>()
        );
    }

    #[test]
    fn ffi_enums_match_signed_c_int_abi() {
        let _: std::os::raw::c_int = bindings::TerminalOption::SCROLLBACK_MAX_BYTES;
        let _: std::os::raw::c_int = bindings::TerminalData::SCROLLBACK_MAX_LINES;
        let _: std::os::raw::c_int = bindings::RenderStateDirty::FULL;
        let _: std::os::raw::c_int = bindings::FormatterFormat::PLAIN;
    }

    #[test]
    fn ffi_string_accepts_null_empty_string() {
        let raw = bindings::String::default();
        // SAFETY: No bytes are read from a null, zero-length string.
        assert_eq!(unsafe { raw.to_bytes() }, b"");
        assert_eq!(unsafe { raw.to_str() }, "");
    }

    #[test]
    fn ffi_string_borrows_non_utf8_bytes() {
        let bytes = [0xff, 0x80];
        let raw = bindings::String {
            ptr: bytes.as_ptr(),
            len: bytes.len(),
        };
        // SAFETY: `bytes` outlives the borrow; to_bytes does not require UTF-8.
        assert_eq!(unsafe { raw.to_bytes() }, bytes);
    }

    #[test]
    fn ffi_string_round_trips_static_str() {
        let raw = bindings::String::from("ghostty");
        assert_eq!(raw.len, "ghostty".len());
        // SAFETY: The source string is `'static` and valid UTF-8.
        assert_eq!(unsafe { raw.to_str() }, "ghostty");
    }

    #[test]
    fn ffi_string_round_trips_borrowed_string() {
        let owned = String::from("terminal");
        let raw = bindings::String::from(owned.as_str());
        // SAFETY: `owned` outlives the conversion and contains valid UTF-8.
        assert_eq!(unsafe { raw.to_str() }, "terminal");
    }
}
