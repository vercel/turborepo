/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under both the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree and the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root
 * directory of this source tree.
 */

//!
//! General macros useful for path declaration

use std::cmp;

///
/// Generates ['cmp::PartialEq'] and ['cmp::PartialOrd'] for the `lhs` and `rhs`
/// types, where `ty` is the unowned, reference path type.
macro_rules! impl_cmp {
    ($lhs:ty, $rhs:ty, $ty:ty) => {
        impl cmp::PartialEq<$rhs> for $lhs {
            #[inline]
            fn eq(&self, other: &$rhs) -> bool {
                <$ty as cmp::PartialEq>::eq(self, other)
            }
        }

        impl cmp::PartialEq<$lhs> for $rhs {
            #[inline]
            fn eq(&self, other: &$lhs) -> bool {
                <$ty as cmp::PartialEq>::eq(self, other)
            }
        }

        impl cmp::PartialOrd<$rhs> for $lhs {
            #[inline]
            fn partial_cmp(&self, other: &$rhs) -> Option<cmp::Ordering> {
                <$ty as cmp::PartialOrd>::partial_cmp(self, other)
            }
        }

        impl cmp::PartialOrd<$lhs> for $rhs {
            #[inline]
            fn partial_cmp(&self, other: &$lhs) -> Option<cmp::Ordering> {
                <$ty as cmp::PartialOrd>::partial_cmp(self, other)
            }
        }
    };
}

///
/// Generates ['cmp::PartialEq'] and ['cmp::PartialOrd'] for the `lhs` and `rhs`
/// string types, where `ty` is the unowned, reference path type.
macro_rules! impl_cmp_str {
    ($lhs:ty, $rhs:ty, $ty:ty) => {
        impl cmp::PartialEq<$rhs> for $lhs {
            #[inline]
            fn eq(&self, other: &$rhs) -> bool {
                match <$ty>::new(other) {
                    Ok(other) => <$ty as cmp::PartialEq>::eq(self, other),
                    _ => false,
                }
            }
        }

        impl cmp::PartialEq<$lhs> for $rhs {
            #[inline]
            fn eq(&self, other: &$lhs) -> bool {
                match <$ty>::new(self) {
                    Ok(this) => <$ty as cmp::PartialEq>::eq(this, other),
                    _ => false,
                }
            }
        }

        impl cmp::PartialOrd<$rhs> for $lhs {
            #[inline]
            fn partial_cmp(&self, other: &$rhs) -> Option<cmp::Ordering> {
                match <$ty>::new(other) {
                    Ok(other) => <$ty as cmp::PartialOrd>::partial_cmp(self, other),
                    _ => None,
                }
            }
        }

        impl cmp::PartialOrd<$lhs> for $rhs {
            #[inline]
            fn partial_cmp(&self, other: &$lhs) -> Option<cmp::Ordering> {
                match <$ty>::new(self) {
                    Ok(this) => <$ty as cmp::PartialOrd>::partial_cmp(this, other),
                    _ => None,
                }
            }
        }
    };
}

use crate::relative_forward_unix_path::{RelativeForwardUnixPath, RelativeForwardUnixPathBuf};

impl_cmp!(
    RelativeForwardUnixPathBuf,
    RelativeForwardUnixPath,
    RelativeForwardUnixPath
);
impl_cmp!(
    RelativeForwardUnixPathBuf,
    &'_ RelativeForwardUnixPath,
    RelativeForwardUnixPath
);

impl_cmp_str!(RelativeForwardUnixPathBuf, str, RelativeForwardUnixPath);
impl_cmp_str!(RelativeForwardUnixPathBuf, &'_ str, RelativeForwardUnixPath);
impl_cmp_str!(RelativeForwardUnixPathBuf, String, RelativeForwardUnixPath);
impl_cmp_str!(RelativeForwardUnixPath, str, RelativeForwardUnixPath);
impl_cmp_str!(RelativeForwardUnixPath, &'_ str, RelativeForwardUnixPath);
impl_cmp_str!(RelativeForwardUnixPath, String, RelativeForwardUnixPath);
impl_cmp_str!(&'_ RelativeForwardUnixPath, str, RelativeForwardUnixPath);
impl_cmp_str!(&'_ RelativeForwardUnixPath, String, RelativeForwardUnixPath);

use crate::absolute_forward_system_path::{
    AbsoluteForwardSystemPath, AbsoluteForwardSystemPathBuf,
};

impl_cmp!(
    AbsoluteForwardSystemPathBuf,
    AbsoluteForwardSystemPath,
    AbsoluteForwardSystemPath
);
impl_cmp!(
    AbsoluteForwardSystemPathBuf,
    &'_ AbsoluteForwardSystemPath,
    AbsoluteForwardSystemPath
);

impl_cmp_str!(AbsoluteForwardSystemPathBuf, str, AbsoluteForwardSystemPath);
impl_cmp_str!(
    AbsoluteForwardSystemPathBuf,
    &'_ str,
    AbsoluteForwardSystemPath
);
impl_cmp_str!(
    AbsoluteForwardSystemPathBuf,
    String,
    AbsoluteForwardSystemPath
);
impl_cmp_str!(AbsoluteForwardSystemPath, str, AbsoluteForwardSystemPath);
impl_cmp_str!(
    AbsoluteForwardSystemPath,
    &'_ str,
    AbsoluteForwardSystemPath
);
impl_cmp_str!(AbsoluteForwardSystemPath, String, AbsoluteForwardSystemPath);
impl_cmp_str!(
    &'_ AbsoluteForwardSystemPath,
    str,
    AbsoluteForwardSystemPath
);
impl_cmp_str!(
    &'_ AbsoluteForwardSystemPath,
    String,
    AbsoluteForwardSystemPath
);

use crate::project_relative_path::{AnchoredUnixPath, AnchoredUnixPathBuf};

impl_cmp!(AnchoredUnixPathBuf, AnchoredUnixPath, AnchoredUnixPath);
impl_cmp!(AnchoredUnixPathBuf, &'_ AnchoredUnixPath, AnchoredUnixPath);

impl_cmp_str!(AnchoredUnixPathBuf, str, AnchoredUnixPath);
impl_cmp_str!(AnchoredUnixPathBuf, &'_ str, AnchoredUnixPath);
impl_cmp_str!(AnchoredUnixPathBuf, String, AnchoredUnixPath);
impl_cmp_str!(AnchoredUnixPath, str, AnchoredUnixPath);
impl_cmp_str!(AnchoredUnixPath, &'_ str, AnchoredUnixPath);
impl_cmp_str!(AnchoredUnixPath, String, AnchoredUnixPath);
impl_cmp_str!(&'_ AnchoredUnixPath, str, AnchoredUnixPath);
impl_cmp_str!(&'_ AnchoredUnixPath, String, AnchoredUnixPath);
