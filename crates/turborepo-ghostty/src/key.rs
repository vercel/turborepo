//! Minimal key types required by the terminal API surface we expose.

use crate::ffi;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KittyKeyFlags(pub u8);

impl KittyKeyFlags {
    pub fn from_bits_retain(bits: ffi::KittyKeyFlags) -> Self {
        Self(bits)
    }
}
